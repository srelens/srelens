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
)

// Error is a handler's failure as srelens is to be told it: a JSON-RPC
// error. A handler may wrap it; the answer is the first *Error in the chain.
type Error struct {
	Code    int64
	Message string
	Data    json.RawMessage
}

func (e *Error) Error() string { return e.Message }

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
// ("the handler for `greet`").
func asRPCError(err error, what string) *protocol.RPCError {
	var e *Error
	if errors.As(err, &e) {
		return &protocol.RPCError{Code: e.Code, Message: e.Message, Data: e.Data}
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
