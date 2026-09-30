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
	// Frames already queued may still arrive before the answer to this.
	health := h.request("health", map[string]any{})
	for {
		m := h.recv()
		if m["id"] == float64(health) {
			break
		}
		if m["method"] != "stream/data" {
			t.Fatalf("only frames queued before the cancel: %v", m)
		}
	}
	// A frame already queued on the shared general lane before the cancel
	// landed can still arrive after health's answer: the lifecycle lane
	// always lets health jump that queue (see outbox.go's run), so a
	// backlog built up before the cancel is delivered out of order relative
	// to health, not after it. What must never appear is a terminal frame,
	// and the stream must actually stop once that backlog drains.
	stragglers := 0
	for {
		m, ok := h.nextWithin(300 * time.Millisecond)
		if !ok {
			break
		}
		if m["method"] != "stream/data" {
			t.Fatalf("no terminal frame after a cancel: %v", m)
		}
		if stragglers++; stragglers > 200 {
			t.Fatal("the stream kept sending long after its cancel")
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
