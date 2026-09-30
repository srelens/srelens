package sidecar

import (
	"bufio"
	"errors"
	"io"
	"strings"
	"testing"
	"time"
)

// A lifecycle answer queued behind a full general lane is the next line the
// writer writes, not the 65th.
func TestTheWriterTakesALifecycleLineAheadOfQueuedLines(t *testing.T) {
	out := newOutbox()
	r, w := io.Pipe()
	defer r.Close()
	for i := 0; i < queueLines; i++ {
		if err := out.send(out.general, map[string]int{"frame": i}, nil); err != nil {
			t.Fatal(err)
		}
	}
	if err := out.send(out.lifecycle, map[string]string{"health": "ok"}, nil); err != nil {
		t.Fatal(err)
	}
	go out.run(w)
	br := bufio.NewReader(r)
	got := make(chan string, 1)
	go func() {
		line, _ := br.ReadString('\n')
		got <- line
	}()
	select {
	case line := <-got:
		if !strings.Contains(line, "health") {
			t.Fatalf("the first line was %q", line)
		}
	case <-time.After(time.Second):
		t.Fatal("nothing was written")
	}
}

// send must not queue a line once stop has already fired, even with room in
// the lane: select picks at random among ready cases, so without its own
// pre-check, send could still take the "lane <- line" branch. A fresh outbox
// each time keeps the race live across every iteration.
func TestSendWithStopAlreadyFiredQueuesNothing(t *testing.T) {
	stop := make(chan struct{})
	close(stop)
	for i := 0; i < 200; i++ {
		out := newOutbox() // its writer never runs, but the lane has plenty of room
		if err := out.send(out.general, "x", stop); !errors.Is(err, errStopped) {
			t.Fatalf("call %d: %v", i, err)
		}
		if len(out.general) != 0 {
			t.Fatalf("call %d: queued %d lines", i, len(out.general))
		}
	}
}
