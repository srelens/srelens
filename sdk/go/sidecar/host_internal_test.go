package sidecar

import (
	"context"
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
