package sidecar

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"slices"
	"strings"
	"sync"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// supportedVersions are the sidecar API versions this SDK speaks, oldest first.
var supportedVersions = []string{protocol.APIVersion}

// sessionKey is the context key for what initialize told the sidecar.
type sessionKey struct{}

// shared is what initialize told the sidecar, which every handler's context
// carries.
type shared struct {
	dataDir    string
	limits     protocol.InitializeLimits
	apiVersion string
	host       *Host
}

type session struct {
	sc      *Sidecar
	out     *outbox
	shared  *shared         // set by initialize
	base    context.Context // carries shared: every handler's context derives from it
	mu      sync.Mutex
	running map[string]context.CancelCauseFunc
}

type flow int

const (
	proceed flow = iota
	shutdown
)

// serve is Sidecar.Run.
func serve(ctx context.Context, s *Sidecar, r io.Reader, w io.Writer) error {
	out := newOutbox()
	go out.run(w)
	se := &session{sc: s, out: out, running: map[string]context.CancelCauseFunc{}}
	lines := make(chan []byte)
	readErr := make(chan error, 1)
	stopReading := make(chan struct{})
	defer close(stopReading)
	go readLines(r, lines, readErr, stopReading)
	ended := func() error {
		for {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-out.done:
				// The writer stopped before the session decided to end: a
				// write to srelens failed.
				return pipeFailed(out.err)
			case line, ok := <-lines:
				if !ok {
					return <-readErr
				}
				if len(bytes.TrimSpace(line)) == 0 {
					continue
				}
				msg, err := protocol.ParseMessage(line)
				if err != nil {
					slog.Error("srelens wrote a line that is not JSON-RPC: " + err.Error())
					return fmt.Errorf("%w: %v", ErrProtocol, err)
				}
				if se.handle(msg) == shutdown {
					return nil
				}
			}
		}
	}()
	se.end()
	out.close()
	if ended == nil && out.err != nil {
		return pipeFailed(out.err)
	}
	return ended
}

func pipeFailed(err error) error {
	if err == nil {
		err = errors.New("the writer stopped")
	}
	return fmt.Errorf("the pipe to srelens failed: %w", err)
}

// readLines sends each line of r, then closes lines; readErr then holds nil
// at the end of r, or the read error.
func readLines(r io.Reader, lines chan<- []byte, readErr chan<- error, stop <-chan struct{}) {
	br := bufio.NewReader(r)
	for {
		line, err := br.ReadBytes('\n')
		if len(line) > 0 {
			select {
			case lines <- line:
			case <-stop:
				return
			}
		}
		if err != nil {
			if errors.Is(err, io.EOF) {
				err = nil
			} else {
				err = pipeFailed(err)
			}
			readErr <- err
			close(lines)
			return
		}
	}
}

func (se *session) handle(msg protocol.Message) flow {
	switch {
	case msg.Request != nil:
		return se.request(*msg.Request)
	case msg.Notification != nil:
		se.notify(*msg.Notification)
	case msg.Response != nil:
		se.answered(*msg.Response)
	}
	return proceed
}

func (se *session) request(req protocol.Request) flow {
	if req.Method == protocol.MethodInitialize {
		se.initialize(req.ID, req.Params)
		return proceed
	}
	if se.shared == nil {
		se.answerError(req.ID, protocol.CodeInvalidRequest, "srelens has not initialized this sidecar", nil)
		return proceed
	}
	switch req.Method {
	case protocol.MethodActivate, protocol.MethodHealth, protocol.MethodDeactivate:
		se.answerOn(se.out.lifecycle, req.ID, json.RawMessage("{}"))
	case protocol.MethodShutdown:
		// Ended before the answer, as end orders it: a handler blocked in a
		// host call sends nothing that could follow the answer.
		se.end()
		se.answer(req.ID, json.RawMessage("{}"))
		return shutdown
	default:
		se.dispatch(req)
	}
	return proceed
}

// dispatch serves an app request: a stream's opening, or an operation.
func (se *session) dispatch(req protocol.Request) {
	if req.Method == protocol.MethodStreamOpen {
		se.openStream(req.ID, req.Params)
		return
	}
	se.operation(req)
}

func (se *session) openStream(id protocol.RequestID, raw json.RawMessage) {
	var open protocol.StreamOpenParams
	if err := json.Unmarshal(raw, &open); err != nil {
		se.answerError(id, protocol.CodeInvalidParams, "stream/open: "+err.Error(), nil)
		return
	}
	start, ok := se.sc.streams[open.Method]
	if !ok {
		se.answerError(id, protocol.CodeMethodNotFound, fmt.Sprintf("this sidecar has no stream `%s`", open.Method), nil)
		return
	}
	ctx, cancel := context.WithCancelCause(se.base)
	frames := &Frames{stream: open.Stream, out: se.out, ctx: ctx, cancel: cancel}
	what := fmt.Sprintf("the stream `%s`", open.Method)
	run, err := start(ctx, open.Params, frames)
	if err != nil {
		cancel(err)
		se.send(se.out.general, protocol.Response{ID: id, Error: asRPCError(err, what)})
		return
	}
	key := streamKey(open.Stream)
	se.track(key, cancel)
	// The ack is queued before the handler starts, so it precedes every frame.
	se.answer(id, json.RawMessage("{}"))
	go func() {
		_, err := guarded(what, func() (struct{}, error) { return struct{}{}, run() })
		frames.finish()
		// Cancelled, or the session ended: srelens asked for nothing more, a
		// terminal frame included.
		if !se.untrack(key) {
			return
		}
		se.endStream(open.Stream, err, what)
	}()
}

func streamKey(stream uint64) string { return fmt.Sprintf("stream %d", stream) }

// endStream queues the stream's one terminal frame: stream/close, or
// stream/error with why. An error message too large to send is replaced with
// one that says so, since the stream must still end.
func (se *session) endStream(stream uint64, err error, what string) {
	if err == nil {
		params, _ := json.Marshal(protocol.StreamCloseParams{Stream: stream})
		_ = se.out.send(se.out.general, protocol.Notification{Method: protocol.MethodStreamClose, Params: params}, nil)
		return
	}
	params, _ := json.Marshal(protocol.StreamErrorParams{Stream: stream, Message: asRPCError(err, what).Message})
	sendErr := se.out.send(se.out.general, protocol.Notification{Method: protocol.MethodStreamError, Params: params}, nil)
	var over *errOverLimit
	if errors.As(sendErr, &over) {
		params, _ := json.Marshal(protocol.StreamErrorParams{Stream: stream, Message: fmt.Sprintf(
			"the stream's error is %d bytes, over the %s a message may be", over.bytes, limitText())})
		_ = se.out.send(se.out.general, protocol.Notification{Method: protocol.MethodStreamError, Params: params}, nil)
	}
}

func (se *session) operation(req protocol.Request) {
	run, ok := se.sc.operations[req.Method]
	if !ok {
		se.answerError(req.ID, protocol.CodeMethodNotFound, fmt.Sprintf("this sidecar has no operation `%s`", req.Method), nil)
		return
	}
	key := requestKey(req.ID)
	ctx, cancel := context.WithCancelCause(se.base)
	se.track(key, cancel)
	what := fmt.Sprintf("the handler for `%s`", req.Method)
	go func() {
		result, err := guarded(what, func() (json.RawMessage, error) { return run(ctx, req.Params) })
		cancel(ErrHandlerReturned)
		// Cancelled, or the session ended: -32800 was the answer, or none is
		// due, and this one is dropped.
		if !se.untrack(key) {
			return
		}
		if err != nil {
			se.send(se.out.general, protocol.Response{ID: req.ID, Error: asRPCError(err, what)})
			return
		}
		se.answer(req.ID, result)
	}()
}

func requestKey(id protocol.RequestID) string { return "request " + id.Key() }

func (se *session) track(key string, cancel context.CancelCauseFunc) {
	se.mu.Lock()
	se.running[key] = cancel
	se.mu.Unlock()
}

// untrack forgets key; whether it was still running, which it is not once
// srelens cancelled it or the session ended.
func (se *session) untrack(key string) bool {
	se.mu.Lock()
	defer se.mu.Unlock()
	_, ok := se.running[key]
	delete(se.running, key)
	return ok
}

// cancelRunning stops key with cause, if it is still running; whether it was.
func (se *session) cancelRunning(key string, cause error) bool {
	se.mu.Lock()
	cancel, ok := se.running[key]
	delete(se.running, key)
	se.mu.Unlock()
	if ok {
		cancel(cause)
	}
	return ok
}

// notify handles srelens's notifications; one this SDK does not know is ignored.
func (se *session) notify(note protocol.Notification) {
	switch note.Method {
	case protocol.MethodCancelRequest:
		var p protocol.CancelParams
		if err := json.Unmarshal(note.Params, &p); err != nil || p.ID.IsZero() {
			return
		}
		if se.cancelRunning(requestKey(p.ID), ErrCancelled) {
			se.answerError(p.ID, protocol.CodeRequestCancelled, "srelens cancelled the request", nil)
		}
	case protocol.MethodStreamCancel:
		var p protocol.StreamCancelParams
		if err := json.Unmarshal(note.Params, &p); err == nil {
			se.cancelRunning(streamKey(p.Stream), ErrCancelled)
		}
	}
}

// answered routes srelens's answer to one of the sidecar's calls.
func (se *session) answered(resp protocol.Response) {
	if se.shared != nil {
		se.shared.host.answered(resp)
	}
}

func (se *session) initialize(id protocol.RequestID, raw json.RawMessage) {
	if se.shared != nil {
		se.answerError(id, protocol.CodeInvalidRequest, "srelens already initialized this sidecar", nil)
		return
	}
	var params protocol.InitializeParams
	if err := json.Unmarshal(raw, &params); err != nil {
		se.answerError(id, protocol.CodeInvalidParams, "initialize: "+err.Error(), nil)
		return
	}
	// The newest version both speak.
	chosen := ""
	for i := len(supportedVersions) - 1; i >= 0 && chosen == ""; i-- {
		if slices.Contains(params.APIVersions, supportedVersions[i]) {
			chosen = supportedVersions[i]
		}
	}
	if chosen == "" {
		data, _ := json.Marshal(protocol.UnsupportedAPIVersion{Supported: supportedVersions})
		se.answerError(id, protocol.CodeUnsupportedAPIVersion, fmt.Sprintf(
			"this sidecar speaks sidecar API %s; srelens offered %s",
			strings.Join(supportedVersions, ", "), strings.Join(params.APIVersions, ", ")), data)
		return
	}
	se.shared = &shared{dataDir: params.DataDirectory, limits: params.Limits, apiVersion: chosen, host: newHost(se.out, params.Limits)}
	if se.sc.sizeRuntime {
		sizeRuntime(params.Limits, processKnobs)
	}
	se.base = context.WithValue(context.Background(), sessionKey{}, se.shared)
	result, _ := json.Marshal(protocol.InitializeResult{
		APIVersion: chosen,
		Sidecar:    &protocol.Peer{Name: se.sc.name, Version: se.sc.version},
	})
	se.answer(id, result)
}

// answer queues request id's result on the general lane.
func (se *session) answer(id protocol.RequestID, result json.RawMessage) {
	se.answerOn(se.out.general, id, result)
}

func (se *session) answerOn(lane chan []byte, id protocol.RequestID, result json.RawMessage) {
	se.send(lane, protocol.Response{ID: id, Result: result})
}

func (se *session) answerError(id protocol.RequestID, code int64, message string, data json.RawMessage) {
	se.send(se.out.general, protocol.Response{ID: id, Error: &protocol.RPCError{Code: code, Message: message, Data: data}})
}

// send queues an answer. One too large to send, or that fails to serialize
// for any other reason, is answered with why instead, so srelens never waits
// on an answer the SDK could not write. Nothing more can be done once the
// writer has stopped: the pipe to srelens is already gone.
func (se *session) send(lane chan []byte, resp protocol.Response) {
	err := se.out.send(lane, resp, nil)
	if err == nil || errors.Is(err, errWriterStopped) {
		return
	}
	var why *protocol.RPCError
	var over *errOverLimit
	if errors.As(err, &over) {
		why = &protocol.RPCError{
			Code:    protocol.CodeInternalError,
			Message: fmt.Sprintf("the answer is %d bytes, over the %s a message may be", over.bytes, limitText()),
		}
	} else {
		slog.Error(fmt.Sprintf("the answer to request %s could not be serialized: %v", resp.ID, err))
		why = &protocol.RPCError{Code: protocol.CodeInternalError, Message: "the answer could not be serialized"}
	}
	_ = se.out.send(lane, protocol.Response{ID: resp.ID, Error: why}, nil)
}

// cancelAll stops every running handler with cause, and forgets them: nothing
// more is written for any of them.
func (se *session) cancelAll(cause error) {
	se.mu.Lock()
	running := se.running
	se.running = map[string]context.CancelCauseFunc{}
	se.mu.Unlock()
	for _, cancel := range running {
		cancel(cause)
	}
}

// end stops every handler: the session is over. The host is disconnected
// first, so a call a handler abandons as it stops sends nothing. shutdown
// ends the session before its answer, and serve after its loop: a second
// end does nothing more.
func (se *session) end() {
	if se.shared != nil {
		se.shared.host.disconnect()
	}
	se.cancelAll(ErrSessionEnded)
}
