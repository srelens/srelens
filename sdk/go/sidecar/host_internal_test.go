package sidecar

import (
	"context"
	"encoding/json"
	"errors"
	"runtime"
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

// register takes a slot and registers call id as waiting, exactly as call
// does before it queues the request.
func register(h *Host, id protocol.RequestID) *pending {
	h.slots <- struct{}{}
	p := &pending{answer: make(chan protocol.Response, 1), holdsSlot: true}
	h.mu.Lock()
	h.waiting[id.Key()] = p
	h.mu.Unlock()
	return p
}

// forget must not free a slot that answered has already freed: a host that
// answers an id it never actually received (a race, or a misbehaving host)
// while the caller's own send is still in flight removes the waiting entry
// and frees the slot itself. forget must see the slot is already freed and
// do nothing, not free the same slot a second time.
func TestForgetDoesNotFreeASlotAnsweredAlreadyFreed(t *testing.T) {
	h, _ := testHost(t)
	id := protocol.StringID("c-1")
	p := register(h, id)

	// srelens answers first: this removes the entry and frees the slot.
	h.answered(protocol.Response{ID: id, Result: json.RawMessage("null")})
	if len(h.slots) != 0 {
		t.Fatalf("answered did not free the slot: %d", len(h.slots))
	}

	// The caller's own send now fails (as it would once its ctx ends): forget
	// must find nothing left to free, and must not touch h.slots again.
	done := make(chan struct{})
	go func() {
		h.forget(id, p)
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

// The slot of a call whose request never reached the queue is freed once
// when the host disconnects: disconnect frees it, and forget, as the call
// gives up afterwards, finds it freed and neither blocks nor frees again.
func TestTheSlotOfACallNotYetQueuedIsFreedOnceWhenTheHostDisconnects(t *testing.T) {
	h, _ := testHost(t)
	id := protocol.StringID("c-1")
	p := register(h, id)

	h.disconnect()

	done := make(chan struct{})
	go func() {
		h.forget(id, p)
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(200 * time.Millisecond):
		t.Fatal("forget blocked after disconnect")
	}
	if len(h.slots) != 0 {
		t.Fatalf("slots %d", len(h.slots))
	}
}

// fillQueue fills out's general lane, whose writer never runs, so the next
// request waits for room.
func fillQueue(t *testing.T, out *outbox) {
	t.Helper()
	for i := 0; i < queueLines; i++ {
		if err := out.send(out.general, i, nil); err != nil {
			t.Fatal(err)
		}
	}
}

// waitForCalls waits until n calls are waiting on h: each holds its slot and
// has its id, and is about to queue its request or is waiting for room.
func waitForCalls(t *testing.T, h *Host, n int) {
	t.Helper()
	deadline := time.Now().Add(time.Second)
	for {
		h.mu.Lock()
		got := len(h.waiting)
		h.mu.Unlock()
		if got == n {
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("%d calls waiting, want %d", got, n)
		}
		time.Sleep(time.Millisecond)
	}
}

// srelens may answer a call whose request is still waiting for room in the
// queue (a race, or a misbehaving host): answered frees that call's slot.
// When the session then ends, the call must not free its slot a second
// time: that blocks it for good when no other slot is held, and otherwise
// takes another call's. others is how many other slots are held meanwhile.
func TestACallAnsweredWhileItWaitsForRoomFreesItsSlotOnceWhenTheSessionEnds(t *testing.T) {
	for _, others := range []int{0, 1} {
		h, out := testHost(t)
		fillQueue(t, out)
		for range others {
			h.slots <- struct{}{} // a call between taking its slot and taking its id
		}
		ctx, cancel := context.WithCancel(context.Background())
		result := make(chan error, 1)
		go func() {
			_, err := h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{})
			result <- err
		}()
		waitForCalls(t, h, 1)
		h.answered(protocol.Response{ID: protocol.StringID("c-1"), Result: json.RawMessage("null")})
		h.disconnect()
		cancel() // as end does, after disconnecting
		select {
		case <-result:
		case <-time.After(time.Second):
			t.Fatalf("others %d: the call blocked freeing a slot answered had already freed", others)
		}
		if len(h.slots) != others {
			t.Fatalf("others %d: %d slots held after the call returned", others, len(h.slots))
		}
	}
}

// A call waiting for room in the queue when the session ends gets
// ErrSessionEnded and queues nothing, whether the end cancels its ctx (a
// handler's, which end cancels after disconnecting) or not (a detached one).
func TestACallWaitingForRoomWhenTheSessionEndsGetsErrSessionEnded(t *testing.T) {
	for _, detached := range []bool{false, true} {
		h, out := testHost(t)
		fillQueue(t, out)
		ctx, cancel := context.WithCancel(context.Background())
		callCtx := ctx
		if detached {
			callCtx = context.WithoutCancel(ctx)
		}
		result := make(chan error, 1)
		go func() {
			_, err := h.call(callCtx, protocol.MethodHostRead, protocol.HostReadParams{})
			result <- err
		}()
		waitForCalls(t, h, 1)
		h.disconnect()
		cancel()
		select {
		case err := <-result:
			if !errors.Is(err, ErrSessionEnded) {
				t.Fatalf("detached %v: %v", detached, err)
			}
		case <-time.After(time.Second):
			t.Fatalf("detached %v: the call kept waiting for room after the session ended", detached)
		}
		if len(out.general) != queueLines || len(h.slots) != 0 {
			t.Fatalf("detached %v: queued %d lines, slots %d", detached, len(out.general)-queueLines, len(h.slots))
		}
	}
}

// A call whose request was just queued as the session ends gets
// ErrSessionEnded, though its ctx, which end cancels, is done too by the
// time the call waits for its answer. The call waits in an unbuffered lane
// until the test takes its request, which readies the call, and the
// session ends before the call runs on: it then finds both the end and its
// ctx done. With one P, the call can wait only in the lane once it is
// registered, and nothing else runs it before the end; select's choice
// between the two is random, so it is run many times.
func TestACallJustQueuedWhenTheSessionEndsGetsErrSessionEnded(t *testing.T) {
	defer runtime.GOMAXPROCS(runtime.GOMAXPROCS(1))
	for i := 0; i < 200; i++ {
		out := &outbox{lifecycle: make(chan []byte, 8), general: make(chan []byte), done: make(chan struct{})}
		h := newHost(out, protocol.InitializeLimits{MaxConcurrentRequests: 8})
		ctx, cancel := context.WithCancel(context.Background())
		result := make(chan error, 1)
		go func() {
			_, err := h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{})
			result <- err
		}()
		waitForCalls(t, h, 1)
		select {
		case <-out.general:
		case <-time.After(time.Second):
			t.Fatalf("call %d: nothing was queued", i)
		}
		h.disconnect()
		cancel()
		select {
		case err := <-result:
			if !errors.Is(err, ErrSessionEnded) {
				t.Fatalf("call %d: %v", i, err)
			}
		case <-time.After(time.Second):
			t.Fatalf("call %d: the call did not return", i)
		}
	}
}

// A call waiting for a slot that is handed one as the session ends gets
// ErrSessionEnded, though its ctx, which end cancels, is done too when it
// runs on, and frees the slot. With one P, the call runs until it waits for
// the slot, and the freed slot readies it; the session ends before it runs.
func TestACallGivenASlotAsTheSessionEndsGetsErrSessionEnded(t *testing.T) {
	defer runtime.GOMAXPROCS(runtime.GOMAXPROCS(1))
	h := newHost(newOutbox(), protocol.InitializeLimits{MaxConcurrentRequests: 1})
	h.slots <- struct{}{} // another call's
	ctx, cancel := context.WithCancel(context.Background())
	result := make(chan error, 1)
	go func() {
		_, err := h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{})
		result <- err
	}()
	time.Sleep(10 * time.Millisecond) // the call runs, and waits for the slot
	<-h.slots                         // the other call's slot is freed, and handed to this one
	h.disconnect()
	cancel()
	select {
	case err := <-result:
		if !errors.Is(err, ErrSessionEnded) {
			t.Fatalf("%v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("the call did not return")
	}
	if len(h.slots) != 0 {
		t.Fatalf("slots %d", len(h.slots))
	}
}

// The session's end frees the slot of every call srelens has not answered:
// one waiting for its answer, which gets ErrSessionEnded, and one whose
// caller stopped waiting after its request was queued, which kept its slot
// for srelens's answer.
func TestTheSessionsEndFreesTheSlotOfEveryUnansweredCall(t *testing.T) {
	h, out := testHost(t)
	waiting := make(chan error, 1)
	go func() {
		_, err := h.call(context.Background(), protocol.MethodHostRead, protocol.HostReadParams{})
		waiting <- err
	}()
	ctx, cancel := context.WithCancel(context.Background())
	abandoned := make(chan error, 1)
	go func() {
		_, err := h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{})
		abandoned <- err
	}()
	waitForCalls(t, h, 2)
	for len(out.general) < 2 {
		time.Sleep(time.Millisecond) // both requests queued
	}
	cancel()
	if err := <-abandoned; !errors.Is(err, context.Canceled) {
		t.Fatalf("the abandoned call got %v", err)
	}
	if len(h.slots) != 2 {
		t.Fatalf("slots %d before the end, want 2", len(h.slots))
	}
	h.disconnect()
	if err := <-waiting; !errors.Is(err, ErrSessionEnded) {
		t.Fatalf("the waiting call got %v", err)
	}
	if len(h.slots) != 0 {
		t.Fatalf("slots %d after the end", len(h.slots))
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

// shutdown must end the session as end does: the host disconnected first,
// then the handlers cancelled, so a handler blocked in a host call sends no
// $/cancelRequest that could land after the shutdown answer. The answer {}
// is then the one line queued, and serve's own end after its loop must be
// harmless. Deterministic for the same reason as the test above.
func TestShutdownDisconnectsTheHostBeforeItCancelsHandlers(t *testing.T) {
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

	if se.request(protocol.Request{ID: protocol.StringID("s"), Method: protocol.MethodShutdown}) != shutdown {
		t.Fatal("shutdown did not end the session")
	}
	if !called {
		t.Fatal("shutdown did not cancel the running handler")
	}
	if !sawDisconnected {
		t.Fatal("shutdown cancelled the handler before the host disconnected")
	}
	if len(out.general) != 1 {
		t.Fatalf("queued %d lines, want only the answer", len(out.general))
	}
	msg, err := protocol.ParseMessage(<-out.general)
	if err != nil || msg.Response == nil || msg.Response.ID != protocol.StringID("s") || string(msg.Response.Result) != "{}" {
		t.Fatalf("the queued line is not shutdown's answer {}: %+v, %v", msg, err)
	}
	se.end() // as serve does after its loop
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
