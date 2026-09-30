package sidecar

import (
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"runtime/debug"

	"github.com/srelens/srelens/sdk/go/protocol"
)

var (
	// ErrProtocol: srelens wrote a line that is not JSON-RPC, which ends the
	// session.
	ErrProtocol = errors.New("srelens wrote a line that is not JSON-RPC")
	// ErrSessionEnded: the session with srelens ended. It is a handler's
	// context's cause at shutdown or the end of input, a host call's error
	// when the session ends before its answer, and Frames.Send's error after
	// the end.
	ErrSessionEnded = errors.New("the session with srelens ended")
	// ErrCancelled is a handler's context's cause when srelens cancelled its
	// request or stream.
	ErrCancelled = errors.New("srelens cancelled this request or stream")
	// ErrHandlerReturned is a handler's context's cause once the handler has
	// returned, as net/http cancels a request's context: a goroutine the
	// handler left running sees its context done.
	ErrHandlerReturned = errors.New("the handler returned")
	// ErrStreamCancelled is Frames.Send's error once srelens has cancelled the stream.
	ErrStreamCancelled = errors.New("srelens cancelled the stream")
	// ErrStreamFinished is Frames.Send's error once the stream's handler has
	// returned: a Frames kept past its handler sends nothing.
	ErrStreamFinished = errors.New("the stream already ended: its handler returned")
	// ErrFrameTooLarge matches a *FrameTooLargeError.
	ErrFrameTooLarge = errors.New("the frame is over the message limit")
	// ErrNoSession is every host call's error for a context the SDK did not
	// make, such as a unit test's.
	ErrNoSession = errors.New("no session with srelens: this context did not come from the sidecar SDK")
	// ErrConsentDenied matches srelens's -32002: a person declined, or no one
	// could be asked. Nothing ran.
	ErrConsentDenied = errors.New("srelens was not given consent")
	// ErrCapabilityFailed matches -32003: srelens or the cluster refused or
	// failed the call.
	ErrCapabilityFailed = errors.New("srelens refused the call")
	// ErrInvalidParams matches -32602: srelens refused the call's params.
	ErrInvalidParams = errors.New("srelens refused the call's params")
	// ErrCallCancelled matches -32800: the call to srelens was cancelled.
	ErrCallCancelled = errors.New("the call to srelens was cancelled")
	// ErrCallTooLarge matches a *CallTooLargeError: never sent.
	ErrCallTooLarge = errors.New("the call is over the message limit")
	// ErrInvalidCall is a call with a field srelens would refuse, refused
	// before it was sent.
	ErrInvalidCall = errors.New("srelens would refuse this call")
)

// Error is a handler's failure as srelens is to be told it: a JSON-RPC
// error. A handler may wrap it; the answer is the first *Error in the chain.
type Error struct {
	Code    int64
	Message string
	Data    json.RawMessage
}

// Error is safe on a nil *Error: a handler may return one by mistake (for
// example, a nil *Error stored in a struct{}-typed local that is later
// returned as the error), and it must not crash the sidecar.
func (e *Error) Error() string {
	if e == nil {
		return "(nil *sidecar.Error)"
	}
	return e.Message
}

// NewError is a failure with code and message.
func NewError(code int64, message string) *Error { return &Error{Code: code, Message: message} }

// InvalidParams is a failure because the input was wrong: -32602.
func InvalidParams(message string) *Error { return NewError(protocol.CodeInvalidParams, message) }

// Internal is any other failure: -32603.
func Internal(message string) *Error { return NewError(protocol.CodeInternalError, message) }

// WithData attaches v, serialized as JSON, to the error. It panics if v
// cannot be serialized: a programming error, which the handler's recover
// answers as a panic.
func (e *Error) WithData(v any) *Error {
	data, err := json.Marshal(v)
	if err != nil {
		panic(fmt.Sprintf("sidecar: WithData: %v", err))
	}
	e.Data = data
	return e
}

// errPanicked is a handler that panicked; the answer names the handler.
var errPanicked = errors.New("panicked")

// guarded runs f, turning a panic into errPanicked after logging it, with
// its stack, as what panicked.
func guarded[T any](what string, f func() (T, error)) (result T, err error) {
	defer func() {
		if p := recover(); p != nil {
			slog.Error(fmt.Sprintf("%s panicked: %v\n%s", what, p, debug.Stack()))
			err = errPanicked
		}
	}()
	return f()
}

// asRPCError is the answer for a handler's error. what names the handler
// ("the handler for `greet`"). A handler's error is untrusted: asRPCError
// never panics (a nil *Error, or an error type whose Error, Is or As method
// panics, is answered -32603 instead), and never returns an *RPCError that
// cannot be serialized (an *Error whose Data is not valid JSON is answered
// with its Code and Message but no Data).
func asRPCError(err error, what string) (rpcErr *protocol.RPCError) {
	defer func() {
		if p := recover(); p != nil {
			slog.Error(fmt.Sprintf("%s's error could not be read: %v\n%s", what, p, debug.Stack()))
			rpcErr = &protocol.RPCError{Code: protocol.CodeInternalError, Message: what + "'s error could not be read"}
		}
	}()
	var e *Error
	if errors.As(err, &e) {
		if e == nil {
			return &protocol.RPCError{Code: protocol.CodeInternalError, Message: what + " returned a nil *sidecar.Error"}
		}
		data := e.Data
		if len(data) > 0 && !json.Valid(data) {
			slog.Warn(fmt.Sprintf("%s returned a *sidecar.Error whose Data is not valid JSON: dropping it", what))
			data = nil
		}
		return &protocol.RPCError{Code: e.Code, Message: e.Message, Data: data}
	}
	var c *ContextError
	if errors.As(err, &c) {
		return &protocol.RPCError{Code: protocol.CodeInvalidParams, Message: c.Error()}
	}
	if errors.Is(err, errPanicked) {
		return &protocol.RPCError{Code: protocol.CodeInternalError, Message: what + " panicked"}
	}
	return &protocol.RPCError{Code: protocol.CodeInternalError, Message: err.Error()}
}

// FrameTooLargeError is a frame over the message limit, which was not sent.
type FrameTooLargeError struct{ Bytes int }

func (e *FrameTooLargeError) Error() string {
	return fmt.Sprintf("the frame is %d bytes, over the %s a message may be", e.Bytes, limitText())
}

func (e *FrameTooLargeError) Is(target error) bool { return target == ErrFrameTooLarge }

// HostError is srelens's answer refusing or failing a call. errors.Is
// matches it to ErrConsentDenied, ErrCapabilityFailed, ErrInvalidParams or
// ErrCallCancelled by its code.
type HostError struct {
	Code    int64
	Message string
	Data    json.RawMessage
}

func (e *HostError) Error() string {
	switch e.Code {
	case protocol.CodeConsentDenied:
		return "srelens was not given consent: " + e.Message
	case protocol.CodeCapabilityFailed:
		return "srelens refused the call: " + e.Message
	case protocol.CodeInvalidParams:
		return "srelens refused the call's params: " + e.Message
	case protocol.CodeRequestCancelled:
		return "the call to srelens was cancelled"
	}
	return fmt.Sprintf("srelens answered with an error: %s (%d)", e.Message, e.Code)
}

func (e *HostError) Is(target error) bool {
	switch target {
	case ErrConsentDenied:
		return e.Code == protocol.CodeConsentDenied
	case ErrCapabilityFailed:
		return e.Code == protocol.CodeCapabilityFailed
	case ErrInvalidParams:
		return e.Code == protocol.CodeInvalidParams
	case ErrCallCancelled:
		return e.Code == protocol.CodeRequestCancelled
	}
	return false
}

// CallTooLargeError is a call over the message limit: it was never sent, and
// the session goes on.
type CallTooLargeError struct{ Bytes int }

func (e *CallTooLargeError) Error() string {
	return fmt.Sprintf("the call is %d bytes, over the %s a message may be, so it was not sent", e.Bytes, limitText())
}

func (e *CallTooLargeError) Is(target error) bool { return target == ErrCallTooLarge }
