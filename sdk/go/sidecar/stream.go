package sidecar

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// Frames is where a stream handler sends its frames.
type Frames struct {
	stream uint64
	out    *outbox
	ctx    context.Context
	cancel context.CancelCauseFunc
	// mu is held for reading while a frame is queued, and for writing by
	// finish: no frame can be queued after the stream's terminal frame.
	mu sync.RWMutex
}

// ID is the id srelens gave the stream.
func (f *Frames) ID() uint64 { return f.stream }

// Send sends one frame: v, serialized as JSON. It fails once srelens has
// cancelled the stream (ErrStreamCancelled), once its handler has returned
// (ErrStreamFinished), or once the session has ended (ErrSessionEnded). It
// also fails when the frame is over the message limit (*FrameTooLargeError,
// which matches ErrFrameTooLarge), or when v cannot be serialized. A Send
// waiting for room in the queue returns as soon as the stream stops.
func (f *Frames) Send(v any) error {
	f.mu.RLock()
	defer f.mu.RUnlock()
	if err := f.stopped(); err != nil {
		return err
	}
	data, err := json.Marshal(v)
	if err != nil {
		return fmt.Errorf("the frame could not be serialized: %w", err)
	}
	params, err := json.Marshal(protocol.StreamDataParams{Stream: f.stream, Data: data})
	if err != nil {
		return fmt.Errorf("the frame could not be serialized: %w", err)
	}
	err = f.out.send(f.out.general, protocol.Notification{Method: protocol.MethodStreamData, Params: params}, f.ctx.Done())
	var over *errOverLimit
	switch {
	case err == nil:
		return nil
	case errors.As(err, &over):
		return &FrameTooLargeError{Bytes: over.bytes}
	case errors.Is(err, errStopped):
		return f.stopped()
	default:
		return ErrSessionEnded
	}
}

// stopped is why the stream takes no more frames, or nil.
func (f *Frames) stopped() error {
	if f.ctx.Err() == nil {
		return nil
	}
	switch cause := context.Cause(f.ctx); {
	case errors.Is(cause, ErrCancelled):
		return ErrStreamCancelled
	case errors.Is(cause, ErrHandlerReturned):
		return ErrStreamFinished
	default:
		return ErrSessionEnded
	}
}

// finish refuses every later frame, from any copy of f: the handler
// returned. It waits for a frame that is already being queued, so the
// terminal frame queued after it is the stream's last line.
func (f *Frames) finish() {
	f.mu.Lock()
	f.cancel(ErrHandlerReturned)
	f.mu.Unlock()
}

// streamFunc decodes a stream's params; its run, or why it refused.
type streamFunc func(ctx context.Context, params json.RawMessage, frames *Frames) (func() error, error)

// Stream serves the stream name: srelens's stream/open for name runs handler
// with its params decoded into In; params that do not decode are refused
// -32602, and the stream never opens. Returning nil closes the stream, and
// an error fails it with the error's message. After srelens cancels the
// stream nothing more is sent, a terminal frame included. The handler's ctx
// is done when srelens cancels the stream (cause ErrCancelled), the session
// ends (ErrSessionEnded), or the handler returns (ErrHandlerReturned).
func Stream[In any](s *Sidecar, name string, handler func(context.Context, In, *Frames) error) {
	s.register(name)
	s.streams[name] = func(ctx context.Context, params json.RawMessage, frames *Frames) (func() error, error) {
		var in In
		if err := decode(params, &in); err != nil {
			return nil, InvalidParams(err.Error())
		}
		return func() error { return handler(ctx, in, frames) }, nil
	}
}
