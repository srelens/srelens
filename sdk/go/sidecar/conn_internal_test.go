package sidecar

import (
	"context"
	"encoding/json"
	"errors"
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
