package sidecar

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// Everything the sidecar writes goes through one writer goroutine, so lines
// never interleave, and each is checked against the protocol's size limit
// before it is queued: a longer line would break the framing, and srelens
// stops a sidecar that does that. Lifecycle answers have a lane of their own
// that the writer takes first, so health is never answered behind a queue of
// stream frames.

// queueLines is how many lines wait in the general lane.
const queueLines = 64

type outbox struct {
	lifecycle chan []byte
	general   chan []byte
	done      chan struct{} // closed once the writer has stopped
	err       error         // the failed write's error, or else the error from closing w; read after done
}

func newOutbox() *outbox {
	return &outbox{
		lifecycle: make(chan []byte, 8),
		general:   make(chan []byte, queueLines),
		done:      make(chan struct{}),
	}
}

// errOverLimit is a message longer than MaxMessageBytes, which was not queued.
type errOverLimit struct{ bytes int }

func (e *errOverLimit) Error() string {
	return fmt.Sprintf("the message is %d bytes, over the %s a message may be", e.bytes, limitText())
}

var (
	errWriterStopped = errors.New("the writer stopped")
	errStopped       = errors.New("stopped waiting for room in the queue")
)

// send queues msg as one line on lane, waiting for room until stop fires
// (errStopped) or the writer stops (errWriterStopped). A nil stop never fires.
func (o *outbox) send(lane chan []byte, msg any, stop <-chan struct{}) error {
	line, err := json.Marshal(msg)
	if err != nil {
		return err
	}
	if len(line) > protocol.MaxMessageBytes {
		return &errOverLimit{bytes: len(line)}
	}
	select {
	case <-o.done:
		return errWriterStopped
	default:
	}
	select {
	case lane <- line:
		return nil
	case <-o.done:
		return errWriterStopped
	case <-stop:
		return errStopped
	}
}

// run writes queued lines to w until the close marker (a nil line) or a
// failed write, and closes w on either exit if it can, so the reader on the
// other end reads to its end even after a failed write.
func (o *outbox) run(w io.Writer) {
	defer close(o.done)
	wroteErr := false
	defer func() {
		c, ok := w.(io.Closer)
		if !ok {
			return
		}
		closeErr := c.Close()
		if !wroteErr {
			o.err = closeErr
		}
	}()
	for {
		var line []byte
		select {
		case line = <-o.lifecycle:
		default:
			select {
			case line = <-o.lifecycle:
			case line = <-o.general:
			}
		}
		if line == nil {
			return
		}
		if _, err := w.Write(append(line, '\n')); err != nil {
			o.err = err
			wroteErr = true
			return
		}
	}
}

// close queues the close marker behind everything already queued, and waits
// for the writer to stop. A line queued after the marker is never written.
func (o *outbox) close() {
	select {
	case o.general <- nil:
		<-o.done
	case <-o.done:
	}
}

// limitText is "4 MiB", for sentences.
func limitText() string {
	return fmt.Sprintf("%d MiB", protocol.MaxMessageBytes/(1024*1024))
}
