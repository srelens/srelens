package sidecar_test

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"net"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

const window = 262144

// rig is a sidecar with one operation, "dial", that dials the pod its
// parameters name and hands the conn (or the error) to the test.
type rig struct {
	*fakeHost
	conns chan net.Conn
	hosts chan *sidecar.Host
}

func newRig(t *testing.T, version string) *rig {
	t.Helper()
	s := sidecar.New("t", "1")
	conns := make(chan net.Conn, 4)
	sidecar.Operation(s, "dial", func(ctx context.Context, target sidecar.PodTarget) (json.RawMessage, error) {
		c, err := sidecar.HostFrom(ctx).Dial(ctx, prod(t), target)
		if err != nil {
			return nil, err
		}
		conns <- c
		return json.RawMessage("{}"), nil
	})
	hosts := make(chan *sidecar.Host, 1)
	// "grab" is a client keeping the Host: its operation ends, the Host stays.
	sidecar.Operation(s, "grab", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
		hosts <- sidecar.HostFrom(ctx)
		return json.RawMessage("{}"), nil
	})
	r := &rig{fakeHost: start(t, s), conns: conns, hosts: hosts}
	r.answer(r.request("initialize", initializeParams([]string{version}, r.DataDir, defaultLimits())))
	r.answer(r.request("activate", map[string]any{}))
	return r
}

var kafkaPod = sidecar.PodTarget{Capability: "kafka", Pod: "kafka-0"}

// dial runs a dial operation and answers the host/connect it sends; the call
// and the conn.
func (r *rig) dial(target sidecar.PodTarget) (map[string]any, net.Conn) {
	r.t.Helper()
	id := r.request("dial", target)
	call := r.call()
	r.reply(call["id"], map[string]any{"pod": "kafka-0"}, nil)
	if r.answer(id)["error"] != nil {
		r.t.Fatal("the dial operation failed")
	}
	return call, <-r.conns
}

func (r *rig) frame(method string) map[string]any {
	r.t.Helper()
	m := r.recv()
	if m["method"] != method || m["id"] != nil {
		r.t.Fatalf("expected a %s notification, got %v", method, m)
	}
	return m["params"].(map[string]any)
}

func (r *rig) data(tunnel int, b []byte) {
	r.t.Helper()
	r.notify("tunnel/data", map[string]any{"tunnel": tunnel, "data": base64.StdEncoding.EncodeToString(b)})
}

func decoded(t *testing.T, params map[string]any) []byte {
	t.Helper()
	b, err := base64.StdEncoding.DecodeString(params["data"].(string))
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestDialNeedsAPI03(t *testing.T) {
	for _, version := range []string{"0.1.0", "0.2.0"} {
		t.Run(version, func(t *testing.T) {
			r := newRig(t, version)
			e := errorOf(t, r.answer(r.request("dial", kafkaPod)))
			if !strings.Contains(e.message, "0.3.0") {
				t.Fatalf("%+v", e)
			}
			if err := r.finish(); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestDialSendsGrowingTunnelIDs(t *testing.T) {
	r := newRig(t, "0.3.0")
	for want := 1; want <= 3; want++ {
		target := kafkaPod
		params := map[string]any{
			"context": map[string]any{"clusterId": "kind-dev", "namespace": "team"}, "capability": "kafka", "pod": "kafka-0", "tunnel": float64(want),
		}
		if want == 2 {
			target = sidecar.PodTarget{Capability: "kafka", Object: "my-cluster-kafka", Service: "my-cluster-bootstrap"}
			params = map[string]any{
				"context": map[string]any{"clusterId": "kind-dev", "namespace": "team"}, "capability": "kafka",
				"name": "my-cluster-kafka", "service": "my-cluster-bootstrap", "tunnel": float64(want),
			}
		}
		call, c := r.dial(target)
		if call["method"] != "host/connect" || !reflect.DeepEqual(call["params"], params) {
			t.Fatalf("dial %d sent %v", want, call)
		}
		if c.RemoteAddr().String() == "" || c.LocalAddr().String() == "" {
			t.Fatal("an address is blank")
		}
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestDialRefusesBothPodAndService(t *testing.T) {
	r := newRig(t, "0.3.0")
	for _, target := range []sidecar.PodTarget{
		{Capability: "kafka", Pod: "a", Service: "b"},
		{Capability: "kafka"},
		{Capability: "not valid", Pod: "a"},
		{Capability: "kafka", Pod: "a", Object: "bad name"},
	} {
		e := errorOf(t, r.answer(r.request("dial", target)))
		if !strings.Contains(e.message, "refuse") {
			t.Fatalf("%+v: %+v", target, e)
		}
	}
	if err := r.finish(); err != nil { // nothing was written
		t.Fatal(err)
	}
}

func TestConnReadsDataThatArrivesBeforeTheAnswer(t *testing.T) {
	r := newRig(t, "0.3.0")
	id := r.request("dial", kafkaPod)
	call := r.call()
	r.data(1, []byte("early"))
	r.reply(call["id"], map[string]any{"pod": "kafka-0"}, nil)
	r.answer(id)
	c := <-r.conns
	buf := make([]byte, 5)
	if _, err := io.ReadFull(c, buf); err != nil || string(buf) != "early" {
		t.Fatalf("%q, %v", buf, err)
	}
	r.frame("tunnel/credit")
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestConnWriteChunksAndWaitsForCredit(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	payload := bytes.Repeat([]byte("0123456789abcdef"), 300*1024/16)
	type result struct {
		n   int
		err error
	}
	done := make(chan result, 1)
	go func() {
		n, err := c.Write(payload)
		done <- result{n, err}
	}()
	var got []byte
	for len(got) < window {
		chunk := decoded(t, r.frame("tunnel/data"))
		if len(chunk) > 65536 {
			t.Fatalf("a chunk of %d bytes", len(chunk))
		}
		got = append(got, chunk...)
	}
	if len(got) != window {
		t.Fatalf("sent %d bytes on a window of %d", len(got), window)
	}
	if m, ok := r.nextWithin(100 * time.Millisecond); ok {
		t.Fatalf("the conn wrote %v with no credit", m)
	}
	select {
	case res := <-done:
		t.Fatalf("Write returned %v without credit", res)
	default:
	}
	r.notify("tunnel/credit", map[string]any{"tunnel": 1, "bytes": 65536})
	for len(got) < len(payload) {
		got = append(got, decoded(t, r.frame("tunnel/data"))...)
	}
	select {
	case res := <-done:
		if res.n != len(payload) || res.err != nil {
			t.Fatalf("%+v", res)
		}
	case <-time.After(wait):
		t.Fatal("Write did not return")
	}
	if !bytes.Equal(got, payload) {
		t.Fatal("the bytes differ")
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestConnReadReturnsCredit(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	r.data(1, bytes.Repeat([]byte("x"), 65536))
	r.data(1, bytes.Repeat([]byte("y"), 34464))
	// Nothing is returned for bytes still unread.
	if m, ok := r.nextWithin(50 * time.Millisecond); ok {
		t.Fatalf("credit before any Read: %v", m)
	}
	buf := make([]byte, 10)
	if n, err := c.Read(buf); n != 10 || err != nil {
		t.Fatalf("%d, %v", n, err)
	}
	if p := r.frame("tunnel/credit"); !reflect.DeepEqual(p, map[string]any{"tunnel": float64(1), "bytes": float64(10)}) {
		t.Fatalf("%v", p)
	}
	rest := make([]byte, 100000-10)
	if _, err := io.ReadFull(c, rest); err != nil {
		t.Fatal(err)
	}
	total := 0
	for total < len(rest) {
		total += int(r.frame("tunnel/credit")["bytes"].(float64))
	}
	if total != len(rest) {
		t.Fatalf("returned %d, consumed %d", total, len(rest))
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestConnDeadlines(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	if err := c.SetReadDeadline(time.Now().Add(-time.Second)); err != nil {
		t.Fatal(err)
	}
	if _, err := c.Read(make([]byte, 1)); !errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("a past read deadline: %v", err)
	}
	var ne net.Error
	if _, err := c.Read(make([]byte, 1)); !errors.As(err, &ne) || !ne.Timeout() {
		t.Fatalf("not a timeout: %v", err)
	}
	// A Read blocked with nothing to read stops at its deadline.
	c.SetReadDeadline(time.Now().Add(30 * time.Millisecond))
	if _, err := c.Read(make([]byte, 1)); !errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("a read deadline: %v", err)
	}
	// A Write blocked for credit stops at its deadline, having sent the window.
	c.SetWriteDeadline(time.Now().Add(100 * time.Millisecond))
	n, err := c.Write(make([]byte, window+1))
	if n != window || !errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("a write deadline: %d, %v", n, err)
	}
	for sent := 0; sent < window; sent += 65536 {
		r.frame("tunnel/data")
	}
	// Cleared, the conn works again.
	c.SetDeadline(time.Time{})
	r.data(1, []byte("ok"))
	if n, err := io.ReadFull(c, make([]byte, 2)); n != 2 || err != nil {
		t.Fatalf("%d, %v", n, err)
	}
	r.frame("tunnel/credit")
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestConnCloseSendsTunnelClose(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	if p := r.frame("tunnel/close"); !reflect.DeepEqual(p, map[string]any{"tunnel": float64(1)}) {
		t.Fatalf("%v", p)
	}
	if _, err := c.Read(make([]byte, 1)); !errors.Is(err, net.ErrClosed) {
		t.Fatalf("Read after Close: %v", err)
	}
	if _, err := c.Write([]byte("x")); !errors.Is(err, net.ErrClosed) {
		t.Fatalf("Write after Close: %v", err)
	}
	if err := c.Close(); err == nil {
		t.Fatal("a second Close said nothing")
	}
	// A frame that crossed the close is dropped without a word.
	r.data(1, []byte("late"))
	r.notify("tunnel/credit", map[string]any{"tunnel": 1, "bytes": 1})
	r.notify("tunnel/close", map[string]any{"tunnel": 1})
	if m, ok := r.nextWithin(50 * time.Millisecond); ok {
		t.Fatalf("the conn answered a frame for a closed tunnel: %v", m)
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestCloseUnblocksAPendingReadAndWrite(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	errs := make(chan error, 2)
	go func() { _, err := c.Read(make([]byte, 1)); errs <- err }()
	go func() { _, err := c.Write(make([]byte, window+1)); errs <- err }()
	for sent := 0; sent < window; sent += 65536 {
		r.frame("tunnel/data")
	}
	time.Sleep(20 * time.Millisecond) // let both block
	c.Close()
	r.frame("tunnel/close")
	for range 2 {
		select {
		case err := <-errs:
			if !errors.Is(err, net.ErrClosed) {
				t.Fatalf("%v", err)
			}
		case <-time.After(wait):
			t.Fatal("Close did not unblock")
		}
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestHostCloseSurfacesItsMessage(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	r.data(1, []byte("last words"))
	r.notify("tunnel/close", map[string]any{"tunnel": 1, "message": "the pod left scope"})
	r.answer(r.request("health", map[string]any{})) // the close has been handled: the reader goes in order
	buf := make([]byte, 10)
	if _, err := io.ReadFull(c, buf); err != nil || string(buf) != "last words" {
		t.Fatalf("%q, %v", buf, err)
	}
	if _, err := c.Read(buf); err == nil || !strings.Contains(err.Error(), "the pod left scope") {
		t.Fatalf("%v", err)
	}
	if _, err := c.Write([]byte("x")); err == nil {
		t.Fatal("Write succeeded on a closed tunnel")
	}
	c.Close() // nothing is sent: the host already closed
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestHostCloseWithoutMessageIsEOF(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	r.notify("tunnel/close", map[string]any{"tunnel": 1})
	if _, err := c.Read(make([]byte, 1)); err != io.EOF {
		t.Fatalf("%v", err)
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

// A conn nobody reads fills its window and no more: the reader goroutine is
// never held up, so health is still answered, and a frame past the window
// closes that conn and not the session.
func TestNotifyNeverBlocks(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	sent := make(chan struct{})
	go func() {
		for range window / 65536 {
			r.data(1, make([]byte, 65536))
		}
		close(sent)
	}()
	select {
	case <-sent:
	case <-time.After(wait):
		t.Fatal("the sidecar stopped reading with a full conn buffer")
	}
	if r.answer(r.request("health", map[string]any{}))["error"] != nil {
		t.Fatal("health failed")
	}
	r.data(1, []byte("one byte too many"))
	p := r.frame("tunnel/close")
	if p["tunnel"] != float64(1) || !strings.Contains(p["message"].(string), "window") {
		t.Fatalf("%v", p)
	}
	if _, err := c.Read(make([]byte, 1)); !errors.Is(err, net.ErrClosed) {
		t.Fatalf("%v", err)
	}
	if r.answer(r.request("health", map[string]any{}))["error"] != nil {
		t.Fatal("health failed after the violation")
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestCreditPastTheWindowClosesTheConn(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	r.notify("tunnel/credit", map[string]any{"tunnel": 1, "bytes": 1})
	p := r.frame("tunnel/close")
	if p["tunnel"] != float64(1) || p["message"] == nil {
		t.Fatalf("%v", p)
	}
	if _, err := c.Write([]byte("x")); !errors.Is(err, net.ErrClosed) {
		t.Fatalf("%v", err)
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestTheSessionEndingEndsTheConn(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	errs := make(chan error, 1)
	go func() { _, err := c.Read(make([]byte, 1)); errs <- err }()
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-errs:
		if !errors.Is(err, sidecar.ErrSessionEnded) {
			t.Fatalf("%v", err)
		}
	case <-time.After(wait):
		t.Fatal("the session's end left a Read blocked")
	}
}

// Two writers each own the conn for a whole Write: their chunks never
// interleave, and a credit wakes whichever is next, never leaving one asleep.
func TestConcurrentWritesDoNotInterleave(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	const filler = window - 50000
	if n, err := c.Write(make([]byte, filler)); n != filler || err != nil {
		t.Fatalf("%d, %v", n, err)
	}
	for got := 0; got < filler; {
		got += len(decoded(t, r.frame("tunnel/data")))
	}
	a, b := bytes.Repeat([]byte("A"), 100*1024), bytes.Repeat([]byte("B"), 100*1024)
	errs := make(chan error, 2)
	write := func(p []byte) {
		n, err := c.Write(p)
		if err == nil && n != len(p) {
			err = io.ErrShortWrite
		}
		errs <- err
	}
	go write(a)
	var got []byte
	for len(got) < 50000 { // A sends what the window holds, then blocks
		got = append(got, decoded(t, r.frame("tunnel/data"))...)
	}
	go write(b)
	time.Sleep(20 * time.Millisecond) // B is now waiting behind A
	r.notify("tunnel/credit", map[string]any{"tunnel": 1, "bytes": window})
	for len(got) < len(a)+len(b) {
		got = append(got, decoded(t, r.frame("tunnel/data"))...)
	}
	for range 2 {
		if err := <-errs; err != nil {
			t.Fatal(err)
		}
	}
	if !bytes.Equal(got, append(append([]byte{}, a...), b...)) {
		t.Fatalf("the Writes interleaved (first byte %q)", got[0])
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

// A second caller queued behind a blocked first one returns when the conn is
// closed, as the first does.
func TestCloseUnblocksTwoBlockedWritersAndTwoBlockedReaders(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	errs := make(chan error, 4)
	for range 2 {
		go func() { _, err := c.Read(make([]byte, 1)); errs <- err }()
		go func() { _, err := c.Write(make([]byte, window+1)); errs <- err }()
	}
	for sent := 0; sent < window; sent += 65536 {
		r.frame("tunnel/data")
	}
	time.Sleep(20 * time.Millisecond) // let all four block
	c.Close()
	r.frame("tunnel/close")
	for range 4 {
		select {
		case err := <-errs:
			if !errors.Is(err, net.ErrClosed) {
				t.Fatalf("%v", err)
			}
		case <-time.After(wait):
			t.Fatal("Close left a caller blocked")
		}
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

// A deadline ends a caller waiting behind another, too.
func TestAQueuedWriteStopsAtItsDeadline(t *testing.T) {
	r := newRig(t, "0.3.0")
	_, c := r.dial(kafkaPod)
	go c.Write(make([]byte, window+1)) // takes the window, then blocks for credit
	for sent := 0; sent < window; sent += 65536 {
		r.frame("tunnel/data")
	}
	c.SetWriteDeadline(time.Now().Add(50 * time.Millisecond))
	if _, err := c.Write([]byte("x")); !errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("%v", err)
	}
	c.Close()
	r.frame("tunnel/close")
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

// grab runs the "grab" operation: the Host, after its operation has returned.
func (r *rig) grab() *sidecar.Host {
	r.t.Helper()
	r.answer(r.request("grab", map[string]any{}))
	return <-r.hosts
}

// A network dialer is called with whatever context its client holds, not the
// operation's.
func TestDialWorksWithAContextOfItsOwn(t *testing.T) {
	r := newRig(t, "0.3.0")
	host := r.grab()
	timeout, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	for _, ctx := range []context.Context{context.Background(), timeout} {
		dialed := make(chan net.Conn, 1)
		go func() {
			c, err := host.Dial(ctx, prod(t), kafkaPod)
			if err != nil {
				t.Error(err)
			}
			dialed <- c
		}()
		call := r.call()
		if call["method"] != "host/connect" {
			t.Fatalf("%v", call)
		}
		r.reply(call["id"], map[string]any{"pod": "kafka-0"}, nil)
		c := <-dialed
		if c == nil {
			t.Fatal("no conn")
		}
		r.data(int(call["params"].(map[string]any)["tunnel"].(float64)), []byte("hi"))
		if n, err := io.ReadFull(c, make([]byte, 2)); n != 2 || err != nil {
			t.Fatalf("%d, %v", n, err)
		}
		r.frame("tunnel/credit")
	}
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestDialStillNeedsAPI03WithAnyContext(t *testing.T) {
	r := newRig(t, "0.2.0")
	host := r.grab()
	if _, err := host.Dial(context.Background(), prod(t), kafkaPod); !errors.Is(err, sidecar.ErrInvalidCall) || !strings.Contains(err.Error(), "0.3.0") {
		t.Fatalf("%v", err)
	}
	if err := r.finish(); err != nil { // nothing was written
		t.Fatal(err)
	}
}

// A Host belongs to the session, not to the operation that handed it out: a
// client's background reconnect works while the session is up, and fails with
// ErrSessionEnded once it is over.
func TestDialOnAKeptHostWorksUntilTheSessionEnds(t *testing.T) {
	r := newRig(t, "0.3.0")
	host := r.grab()
	if err := r.finish(); err != nil {
		t.Fatal(err)
	}
	if _, err := host.Dial(context.Background(), prod(t), kafkaPod); !errors.Is(err, sidecar.ErrSessionEnded) {
		t.Fatalf("%v", err)
	}
}
