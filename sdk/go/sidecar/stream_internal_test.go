package sidecar

import (
	"context"
	"errors"
	"testing"
	"time"
)

// A Send waiting for room in a full queue returns as soon as srelens
// cancels the stream, rather than waiting for room that may never come.
func TestASendWaitingOnAFullQueueReturnsOnceTheStreamIsCancelled(t *testing.T) {
	out := newOutbox() // its writer never runs, so nothing drains the queue
	for i := 0; i < queueLines; i++ {
		if err := out.send(out.general, i, nil); err != nil {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithCancelCause(context.Background())
	frames := &Frames{stream: 1, out: out, ctx: ctx, cancel: cancel}
	sent := make(chan error, 1)
	go func() { sent <- frames.Send("waits") }()
	select {
	case err := <-sent:
		t.Fatalf("Send returned %v with the queue full", err)
	case <-time.After(50 * time.Millisecond):
	}
	cancel(ErrCancelled)
	select {
	case err := <-sent:
		if !errors.Is(err, ErrStreamCancelled) {
			t.Fatalf("Send returned %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("Send kept waiting after the cancel")
	}
}
