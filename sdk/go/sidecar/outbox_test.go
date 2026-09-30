package sidecar

import (
	"bufio"
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
