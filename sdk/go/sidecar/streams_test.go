package sidecar_test

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

type countInput struct {
	To int `json:"to"`
}

func streamer() *sidecar.Sidecar {
	s := sidecar.New("t", "1")
	sidecar.Stream(s, "count", func(ctx context.Context, in countInput, frames *sidecar.Frames) error {
		for n := 0; n < in.To; n++ {
			if err := frames.Send(n); err != nil {
				return err
			}
		}
		return nil
	})
	sidecar.Stream(s, "fail", func(_ context.Context, _ struct{}, frames *sidecar.Frames) error {
		if err := frames.Send("first"); err != nil {
			return err
		}
		return errors.New("the registry did not answer")
	})
	sidecar.Stream(s, "forever", func(ctx context.Context, _ struct{}, frames *sidecar.Frames) error {
		for n := 0; ; n++ {
			if err := frames.Send(n); err != nil {
				return err
			}
			if ctx.Err() != nil {
				return nil
			}
		}
	})
	return s
}

func frame(stream int, data any) map[string]any {
	return map[string]any{"jsonrpc": "2.0", "method": "stream/data", "params": map[string]any{"stream": float64(stream), "data": data}}
}

func TestAStreamIsAcknowledgedThenSendsItsFramesThenCloses(t *testing.T) {
	h := start(t, streamer())
	h.initialize()
	id := h.request("stream/open", map[string]any{"stream": 1, "method": "count", "params": map[string]any{"to": 3}})
	if r := h.answer(id)["result"]; !reflect.DeepEqual(r, map[string]any{}) {
		t.Fatalf("the ack was %v", r)
	}
	for n := 0; n < 3; n++ {
		if m := h.recv(); !reflect.DeepEqual(m, frame(1, float64(n))) {
			t.Fatalf("frame %d: %v", n, m)
		}
	}
	want := map[string]any{"jsonrpc": "2.0", "method": "stream/close", "params": map[string]any{"stream": float64(1)}}
	if m := h.recv(); !reflect.DeepEqual(m, want) {
		t.Fatalf("the end: %v", m)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAFailingStreamEndsWithItsError(t *testing.T) {
	h := start(t, streamer())
	h.initialize()
	h.answer(h.request("stream/open", map[string]any{"stream": 2, "method": "fail", "params": map[string]any{}}))
	if m := h.recv(); !reflect.DeepEqual(m, frame(2, "first")) {
		t.Fatalf("%v", m)
	}
	want := map[string]any{"jsonrpc": "2.0", "method": "stream/error",
		"params": map[string]any{"stream": float64(2), "message": "the registry did not answer"}}
	if m := h.recv(); !reflect.DeepEqual(m, want) {
		t.Fatalf("%v", m)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAnUnknownStreamOrBadInputIsRefusedBeforeItOpens(t *testing.T) {
	h := start(t, streamer())
	h.initialize()
	id := h.request("stream/open", map[string]any{"stream": 3, "method": "nope", "params": map[string]any{}})
	if e := errorOf(t, h.answer(id)); e.code != -32601 || !strings.Contains(e.message, "`nope`") {
		t.Fatalf("%+v", e)
	}
	id = h.request("stream/open", map[string]any{"stream": 4, "method": "count", "params": map[string]any{"to": "three"}})
	if e := errorOf(t, h.answer(id)); e.code != -32602 {
		t.Fatalf("%+v", e)
	}
	h.answer(h.request("health", map[string]any{}))
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestACancelledStreamStopsAndSendsNoTerminalFrame(t *testing.T) {
	h := start(t, streamer())
	h.initialize()
	h.answer(h.request("stream/open", map[string]any{"stream": 5, "method": "forever", "params": map[string]any{}}))
	h.recv()
	h.notify("stream/cancel", map[string]any{"stream": 5})
	// Fenced by a request answered on the general lane, which is FIFO:
	// unlike health (on the lifecycle lane, which always jumps the general
	// queue), this answer can only arrive once every frame queued ahead of
	// it -- everything queued before the cancel took effect -- has already
	// been delivered.
	fence := h.request("stream/open", map[string]any{"stream": 6, "method": "nope", "params": map[string]any{}})
	for {
		m := h.recv()
		if m["id"] == float64(fence) {
			if e := errorOf(t, m); e.code != -32601 {
				t.Fatalf("the fence: %+v", e)
			}
			break
		}
		if m["method"] != "stream/data" {
			t.Fatalf("only frames queued before the cancel: %v", m)
		}
	}
	// After the fence, at most the one frame that was mid-send when the
	// cancel landed, and never a close or an error.
	stragglers := 0
	for {
		m, ok := h.nextWithin(300 * time.Millisecond)
		if !ok {
			break
		}
		if m["method"] != "stream/data" {
			t.Fatalf("no terminal frame after a cancel: %v", m)
		}
		if stragglers++; stragglers > 1 {
			t.Fatal("the stream kept sending after its cancel")
		}
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAStreamWhoseErrorIsTooLargeStillEndsWithAnErrorFrame(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Stream(s, "loud", func(context.Context, struct{}, *sidecar.Frames) error {
		return errors.New(strings.Repeat("x", 5<<20))
	})
	h := start(t, s)
	h.initialize()
	h.answer(h.request("stream/open", map[string]any{"stream": 7, "method": "loud", "params": map[string]any{}}))
	m := h.recv()
	params := m["params"].(map[string]any)
	if m["method"] != "stream/error" || params["stream"] != float64(7) || !strings.Contains(params["message"].(string), "4 MiB") {
		t.Fatalf("%v", m)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAFrameThatCannotBeSerializedOrIsTooLargeIsRefusedToTheHandler(t *testing.T) {
	s := sidecar.New("t", "1")
	refused := make(chan error, 2)
	sidecar.Stream(s, "bad", func(_ context.Context, _ struct{}, frames *sidecar.Frames) error {
		refused <- frames.Send(func() {})
		refused <- frames.Send(strings.Repeat("x", 5<<20))
		return nil
	})
	h := start(t, s)
	h.initialize()
	h.answer(h.request("stream/open", map[string]any{"stream": 8, "method": "bad", "params": map[string]any{}}))
	if m := h.recv(); m["method"] != "stream/close" {
		t.Fatalf("%v", m)
	}
	if err := <-refused; err == nil || !strings.Contains(err.Error(), "could not be serialized") {
		t.Fatalf("an unserializable frame: %v", err)
	}
	err := <-refused
	var tooLarge *sidecar.FrameTooLargeError
	if !errors.Is(err, sidecar.ErrFrameTooLarge) || !errors.As(err, &tooLarge) || tooLarge.Bytes <= 5<<20 {
		t.Fatalf("an oversized frame: %v", err)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAStreamHandlerThatPanicsEndsItsStreamAndTheSidecarGoesOn(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Stream(s, "boom", func(_ context.Context, _ struct{}, frames *sidecar.Frames) error {
		_ = frames.Send(1)
		panic("nil map")
	})
	h := start(t, s)
	h.initialize()
	h.answer(h.request("stream/open", map[string]any{"stream": 9, "method": "boom", "params": map[string]any{}}))
	h.recv()
	want := map[string]any{"jsonrpc": "2.0", "method": "stream/error",
		"params": map[string]any{"stream": float64(9), "message": "the stream `boom` panicked"}}
	if m := h.recv(); !reflect.DeepEqual(m, want) {
		t.Fatalf("%v", m)
	}
	h.answer(h.request("health", map[string]any{}))
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAFramesKeptPastItsHandlerSendsNothingAfterTheClosingFrame(t *testing.T) {
	s := sidecar.New("t", "1")
	kept := make(chan *sidecar.Frames, 1)
	sidecar.Stream(s, "leak", func(_ context.Context, _ struct{}, frames *sidecar.Frames) error {
		kept <- frames
		return nil
	})
	h := start(t, s)
	h.initialize()
	h.answer(h.request("stream/open", map[string]any{"stream": 10, "method": "leak", "params": map[string]any{}}))
	want := map[string]any{"jsonrpc": "2.0", "method": "stream/close", "params": map[string]any{"stream": float64(10)}}
	if m := h.recv(); !reflect.DeepEqual(m, want) {
		t.Fatalf("%v", m)
	}
	frames := <-kept
	if frames.ID() != 10 {
		t.Fatalf("ID %d", frames.ID())
	}
	if err := frames.Send("late"); !errors.Is(err, sidecar.ErrStreamFinished) {
		t.Fatalf("a late frame: %v", err)
	}
	// finish also fails on any line not read, such as a late stream/data.
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

// TestStreamsAndOperationsShareOneNameSpace reproduces the bug where
// deleting sidecar.go's `streams` taken-twice check left the suite green:
// nothing exercised a stream registered over another stream, or an operation
// registered over a stream (the ordering only that check catches, since a
// name taken only by a stream is absent from `operations`).
func TestStreamsAndOperationsShareOneNameSpace(t *testing.T) {
	noopOp := func(context.Context, struct{}) (struct{}, error) { return struct{}{}, nil }
	noopStream := func(context.Context, struct{}, *sidecar.Frames) error { return nil }

	s := sidecar.New("t", "1")
	sidecar.Stream(s, "dup", noopStream)
	mustPanic(t, "registering a stream twice", func() { sidecar.Stream(s, "dup", noopStream) })

	s = sidecar.New("t", "1")
	sidecar.Stream(s, "taken", noopStream)
	mustPanic(t, "registering an operation over a stream", func() { sidecar.Operation(s, "taken", noopOp) })

	s = sidecar.New("t", "1")
	sidecar.Operation(s, "taken", noopOp)
	mustPanic(t, "registering a stream over an operation", func() { sidecar.Stream(s, "taken", noopStream) })

	for _, name := range []string{"health", "a_b"} {
		mustPanic(t, "registering a stream named "+name, func() {
			sidecar.Stream(sidecar.New("t", "1"), name, noopStream)
		})
	}
}
