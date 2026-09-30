package sidecar

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/protocol"
)

func testHost(t *testing.T) (*Host, *outbox) {
	t.Helper()
	out := newOutbox() // its writer never runs
	return newHost(out, protocol.InitializeLimits{MaxConcurrentRequests: 8}), out
}

// The field checks keep Read, Resource and Action far under the limit, so
// only the internal call can show this.
func TestACallOverTheMessageLimitIsRefusedAsTooLargeAndFreesItsSlot(t *testing.T) {
	h, _ := testHost(t)
	_, err := h.call(context.Background(), protocol.MethodHostRead, map[string]string{"big": strings.Repeat("x", 5<<20)})
	var tooLarge *CallTooLargeError
	if !errors.Is(err, ErrCallTooLarge) || !errors.As(err, &tooLarge) || !strings.Contains(err.Error(), "4 MiB") {
		t.Fatalf("%v", err)
	}
	if len(h.slots) != 0 || len(h.waiting) != 0 {
		t.Fatalf("slots %d, waiting %d", len(h.slots), len(h.waiting))
	}
}

// A ctx that is already done when call is entered must send nothing and hold
// no slot: select picks at random among ready cases, so without its own
// pre- and post-slot checks, call can still take the slot and queue the
// request. A fresh host each time keeps the race live across every
// iteration.
func TestACallWhoseContextIsAlreadyDoneSendsNothingAndFreesItsSlot(t *testing.T) {
	for i := 0; i < 200; i++ {
		h, out := testHost(t)
		ctx, cancel := context.WithCancel(context.Background())
		cancel()
		_, err := h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{})
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("call %d: %v", i, err)
		}
		if len(out.general) != 0 {
			t.Fatalf("call %d: queued %d lines", i, len(out.general))
		}
		if len(h.slots) != 0 || len(h.waiting) != 0 {
			t.Fatalf("call %d: slots %d, waiting %d", i, len(h.slots), len(h.waiting))
		}
	}
}

func TestACallWhoseContextEndsBeforeItIsQueuedFreesItsSlot(t *testing.T) {
	h, out := testHost(t)
	for i := 0; i < queueLines; i++ {
		if err := out.send(out.general, i, nil); err != nil {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	_, err := h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{})
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("%v", err)
	}
	if len(h.slots) != 0 || len(h.waiting) != 0 {
		t.Fatalf("slots %d, waiting %d", len(h.slots), len(h.waiting))
	}
}

// forget must not free a slot that answered has already freed: a host that
// answers an id it never actually received (a race, or a misbehaving host)
// while the caller's own send is still in flight removes the waiting entry
// and frees the slot itself. forget must see the entry is already gone and
// do nothing, not free the same slot a second time.
func TestForgetDoesNotFreeASlotAnsweredAlreadyFreed(t *testing.T) {
	h, _ := testHost(t)
	// Acquire one slot and register a waiting entry, exactly as call does
	// before it queues the request.
	h.slots <- struct{}{}
	id := protocol.StringID("c-1")
	answer := make(chan protocol.Response, 1)
	h.mu.Lock()
	h.waiting[id.Key()] = answer
	h.mu.Unlock()

	// srelens answers first: this removes the entry and frees the slot.
	h.answered(protocol.Response{ID: id, Result: json.RawMessage("null")})
	if len(h.slots) != 0 {
		t.Fatalf("answered did not free the slot: %d", len(h.slots))
	}

	// The caller's own send now fails (as it would once its ctx ends): forget
	// must find nothing left to remove, and must not touch h.slots again.
	done := make(chan struct{})
	go func() {
		h.forget(id)
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(200 * time.Millisecond):
		t.Fatal("forget blocked trying to free a slot answered had already freed")
	}
	if len(h.slots) != 0 {
		t.Fatalf("forget freed a slot nothing had taken: %d", len(h.slots))
	}
}

// forget must still free the slot when the host has disconnected: answered
// can no longer have removed the entry (its waiting map is nil), so this
// call is definitely still holding its own slot.
func TestForgetFreesTheSlotWhenTheHostHasDisconnected(t *testing.T) {
	h, _ := testHost(t)
	h.slots <- struct{}{}
	id := protocol.StringID("c-1")
	h.mu.Lock()
	h.waiting[id.Key()] = make(chan protocol.Response, 1)
	h.mu.Unlock()

	h.disconnect()

	done := make(chan struct{})
	go func() {
		h.forget(id)
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(200 * time.Millisecond):
		t.Fatal("forget did not free the slot after disconnect")
	}
	if len(h.slots) != 0 {
		t.Fatalf("slots %d", len(h.slots))
	}
}

// session.end must disconnect the host before it cancels handlers. This is
// deterministic, unlike the end-to-end test in host_test.go: disconnect and
// cancelAll both run synchronously in the same goroutine that calls end, so
// there is no scheduling race to observe -- the fake handler's cancel runs
// from inside cancelAll, and by then disconnect either has or has not
// already nilled h.waiting, with nothing racy about which.
func TestEndDisconnectsTheHostBeforeItCancelsHandlers(t *testing.T) {
	out := newOutbox() // its writer never runs
	h := newHost(out, protocol.InitializeLimits{MaxConcurrentRequests: 8})
	se := &session{out: out, running: map[string]context.CancelCauseFunc{}, shared: &shared{host: h}}

	var called, sawDisconnected bool
	se.track("request x", func(cause error) {
		called = true
		h.mu.Lock()
		sawDisconnected = h.waiting == nil
		h.mu.Unlock()
	})

	se.end()

	if !called {
		t.Fatal("end did not cancel the running handler")
	}
	if !sawDisconnected {
		t.Fatal("the handler was cancelled before the host disconnected")
	}
}

// check's namespace refusal must cut the value to 64 runes, as
// protocol.ContextError.Error does, not quote the whole thing.
func TestCheckTruncatesALongNamespaceInItsRefusal(t *testing.T) {
	long := strings.Repeat("x", 100)
	err := check(CallContext{ClusterID: "prod", Namespace: &long})
	if err == nil {
		t.Fatal("expected a refusal")
	}
	msg := err.Error()
	if strings.Contains(msg, long) {
		t.Fatalf("the refusal was not truncated: %s", msg)
	}
	if !strings.Contains(msg, strings.Repeat("x", 64)) {
		t.Fatalf("the refusal dropped the namespace: %s", msg)
	}
}
