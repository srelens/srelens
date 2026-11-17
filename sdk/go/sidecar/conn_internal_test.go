package sidecar

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// dialHost is a Host whose session negotiated 0.3.0. Its tests dial with plain
// contexts, as a network client does.
func dialHost(t *testing.T) (*Host, *outbox) {
	t.Helper()
	h, out := testHost(t)
	h.version = "0.3.0"
	return h, out
}

func queued(t *testing.T, out *outbox) protocol.Message {
	t.Helper()
	select {
	case line := <-out.general:
		msg, err := protocol.ParseMessage(line)
		if err != nil {
			t.Fatal(err)
		}
		return msg
	case <-time.After(5 * time.Second):
		t.Fatal("nothing was queued")
	}
	return protocol.Message{}
}

func TestDialCancelledLeavesNoConn(t *testing.T) {
	h, out := dialHost(t)
	ns := "team"
	cc, err := NewCallContext("kind-dev", &ns)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	result := make(chan error, 1)
	go func() {
		_, err := h.Dial(ctx, cc, PodTarget{Capability: "kafka", Pod: "kafka-0"})
		result <- err
	}()
	req := queued(t, out).Request
	if req == nil || req.Method != protocol.MethodHostConnect {
		t.Fatalf("%+v", req)
	}
	cancel()
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatalf("%v", err)
	}
	note := queued(t, out).Notification
	var cancelled protocol.CancelParams
	if note == nil || note.Method != protocol.MethodCancelRequest || json.Unmarshal(note.Params, &cancelled) != nil || cancelled.ID != req.ID {
		t.Fatalf("expected $/cancelRequest for %v, got %+v", req.ID, note)
	}
	// srelens may have accepted the connect before the cancel reached it, so
	// the close follows the cancel: the pipe is ordered, and a close for a
	// tunnel that is not open is dropped.
	note = queued(t, out).Notification
	var closed protocol.TunnelCloseParams
	if note == nil || note.Method != protocol.MethodTunnelClose || json.Unmarshal(note.Params, &closed) != nil || closed.Tunnel != 1 || closed.Message != nil {
		t.Fatalf("expected tunnel/close for tunnel 1, got %+v", note)
	}
	h.tmu.Lock()
	open := len(h.tunnels)
	h.tmu.Unlock()
	if open != 0 {
		t.Fatalf("%d conns left", open)
	}
	// A late answer and a late frame for the id are dropped.
	h.answered(protocol.Response{ID: req.ID, Result: json.RawMessage(`{"pod":"kafka-0"}`)})
	data, _ := json.Marshal(protocol.TunnelDataParams{Tunnel: 1, Data: "eA=="})
	h.tunnelFrame(protocol.Notification{Method: protocol.MethodTunnelData, Params: data})
	if len(out.general) != 0 {
		t.Fatalf("%d lines queued for a dropped frame", len(out.general))
	}
}

// Ids are assigned in the order the requests are queued, so srelens, which
// refuses an id that is not greater than every earlier one, never sees them
// out of order.
func TestConcurrentDialsQueueTheirIDsInOrder(t *testing.T) {
	h, out := dialHost(t)
	ns := "team"
	cc, _ := NewCallContext("kind-dev", &ns)
	parent, cancel := context.WithCancel(context.Background())
	defer cancel() // the dials wait for answers that never come
	ctx := parent
	for range 4 {
		go h.Dial(ctx, cc, PodTarget{Capability: "kafka", Pod: "kafka-0"})
	}
	var last uint64
	for range 4 {
		var p protocol.HostConnectParams
		if err := json.Unmarshal(queued(t, out).Request.Params, &p); err != nil {
			t.Fatal(err)
		}
		if p.Tunnel != last+1 {
			t.Fatalf("tunnel %d queued after %d", p.Tunnel, last)
		}
		last = p.Tunnel
	}
}

// A conn whose output queue has room for room more lines, and nothing is
// draining it: the next send past that waits, as it does behind a slow pipe.
func queuedConn(t *testing.T, room int) (*Host, *tunnelConn) {
	t.Helper()
	h, out := dialHost(t)
	c, err := h.openTunnel()
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < queueLines-room; i++ {
		if err := out.send(out.general, i, nil); err != nil {
			t.Fatal(err)
		}
	}
	return h, c
}

type ioResult struct {
	n   int
	err error
}

// returns runs f and reports its result, or fails if it has not returned
// within the harness's wait.
func returns(t *testing.T, f func() (int, error)) func() ioResult {
	t.Helper()
	done := make(chan ioResult, 1)
	go func() {
		n, err := f()
		done <- ioResult{n, err}
	}()
	return func() ioResult {
		t.Helper()
		select {
		case r := <-done:
			return r
		case <-time.After(5 * time.Second):
			t.Fatal("the call is still waiting on the output queue")
		}
		return ioResult{}
	}
}

// A Write waiting for room in the output queue stops when the host closes the
// tunnel, with what was queued before it and the host's reason.
func TestABlockedWriteReturnsTheHostsCloseReason(t *testing.T) {
	h, c := queuedConn(t, 1)
	result := returns(t, func() (int, error) { return c.Write(make([]byte, 2*maxChunk)) })
	queueIsFull(t, h) // the first chunk is queued, the second waits
	msg := "the pod left scope"
	c.peerClosed(&msg)
	if r := result(); r.n != maxChunk || r.err == nil || !strings.Contains(r.err.Error(), msg) {
		t.Fatalf("%+v", r)
	}
}

func TestABlockedWriteReturnsErrClosedAfterALocalClose(t *testing.T) {
	h, c := queuedConn(t, 1)
	result := returns(t, func() (int, error) { return c.Write(make([]byte, 2*maxChunk)) })
	queueIsFull(t, h)
	closed := make(chan struct{})
	go func() { c.Close(); close(closed) }() // its own tunnel/close waits for room too
	if r := result(); r.n != maxChunk || !errors.Is(r.err, net.ErrClosed) {
		t.Fatalf("%+v", r)
	}
	h.disconnect() // ends the Close's wait
	<-closed
}

// A Read that is returning credit stops waiting when the host closes the
// tunnel; the bytes already received are still read, then the reason.
func TestABlockedCreditSendReturnsAfterTheHostsCloseAndKeepsBufferedBytes(t *testing.T) {
	_, c := queuedConn(t, 0)
	if why := c.deliver([]byte("0123456789")); why != "" {
		t.Fatal(why)
	}
	result := returns(t, func() (int, error) { return c.Read(make([]byte, 4)) })
	creditIsWaiting(t, c, 4) // the credit for those 4 bytes waits
	msg := "the pod left scope"
	c.peerClosed(&msg)
	if r := result(); r.n != 4 || r.err != nil {
		t.Fatalf("%+v", r)
	}
	rest := make([]byte, 6)
	if n, err := io.ReadFull(c, rest); n != 6 || err != nil || string(rest) != "456789" {
		t.Fatalf("%q, %v", rest, err)
	}
	if _, err := c.Read(rest); err == nil || !strings.Contains(err.Error(), msg) {
		t.Fatalf("%v", err)
	}
}

func TestTheSessionEndingStopsBlockedSends(t *testing.T) {
	h, c := queuedConn(t, 1)
	c.deliver([]byte("0123456789"))
	write := returns(t, func() (int, error) { return c.Write(make([]byte, 2*maxChunk)) })
	queueIsFull(t, h)
	h.disconnect()
	if r := write(); r.n != maxChunk || !errors.Is(r.err, ErrSessionEnded) {
		t.Fatalf("%+v", r)
	}
	// The credit send of a Read blocked the same way.
	h2, c2 := queuedConn(t, 0)
	c2.deliver([]byte("0123456789"))
	read := returns(t, func() (int, error) { return c2.Read(make([]byte, 4)) })
	creditIsWaiting(t, c2, 4)
	h2.disconnect()
	if r := read(); r.n != 4 || r.err != nil {
		t.Fatalf("%+v", r)
	}
}

// eventually waits for cond, which only another goroutine can make true.
func eventually(t *testing.T, what string, cond func() bool) {
	t.Helper()
	for deadline := time.Now().Add(5 * time.Second); !cond(); time.Sleep(time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatalf("never happened: %s", what)
		}
	}
}

// queueIsFull waits until the output queue has no room: a send now waits.
func queueIsFull(t *testing.T, h *Host) {
	t.Helper()
	eventually(t, "the output queue filled", func() bool { return len(h.out.general) == queueLines })
}

// creditIsWaiting waits until a Read has taken n of the 10 delivered bytes and
// is sending their credit, which waits for room in the full queue.
func creditIsWaiting(t *testing.T, c *tunnelConn, n int) {
	t.Helper()
	eventually(t, "a Read is returning credit", func() bool {
		c.mu.Lock()
		defer c.mu.Unlock()
		return c.allowed == tunnelWindow-10+n
	})
}

// A Read's two deadlines are always the same one, whatever order concurrent
// setters run in: a credit send that kept an old deadline would either never
// send (the host's window fills and it stops sending) or wait past the
// deadline.
func TestConcurrentReadDeadlineSettersLeaveOneDeadline(t *testing.T) {
	_, c := queuedConn(t, queueLines)
	past := time.Now().Add(-time.Second)
	for i := 0; i < 50000; i++ {
		c.SetReadDeadline(time.Time{})
		var ready atomic.Int32
		var wg sync.WaitGroup
		for _, deadline := range []time.Time{past, {}} {
			wg.Add(1)
			go func() {
				defer wg.Done()
				ready.Add(1)
				for ready.Load() < 2 { // start together
				}
				c.SetReadDeadline(deadline)
			}()
		}
		wg.Wait()
		if c.rd.expired() != c.cd.expired() {
			t.Fatalf("round %d: read deadline expired: %v, credit deadline expired: %v", i, c.rd.expired(), c.cd.expired())
		}
	}
}

// A Read that times out reports the timeout, whatever else has happened to the
// conn: here it waits behind another Read, and srelens has closed the tunnel.
func TestAReadTimeoutIsATimeoutAfterTheHostsClose(t *testing.T) {
	_, c := queuedConn(t, queueLines)
	msg := "the pod left scope"
	c.peerClosed(&msg)
	c.rlock <- struct{}{} // another Read holds the lock
	c.SetReadDeadline(time.Now().Add(-time.Second))
	if _, err := c.Read(make([]byte, 1)); !errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("%v", err)
	}
}
