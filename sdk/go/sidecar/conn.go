package sidecar

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"sync"
	"time"

	"github.com/srelens/srelens/sdk/go/protocol"
)

const (
	// tunnelWindow is how many bytes either side may send before the other
	// returns credit: the protocol's 256 KiB.
	tunnelWindow = 262144
	// maxChunk is the most bytes one tunnel/data carries, before base64.
	maxChunk = 65536
)

// PodTarget names the pod, or the Service, a sidecar dials.
type PodTarget struct {
	Capability string // the k8s.connectPod binding
	Object     string // the scope object; "" for a namespace-granted binding
	Pod        string // exactly one of Pod and Service
	Service    string
}

// Dial opens a TCP connection to the pod target names, through srelens (API
// 0.3). ctx only bounds the dial itself, so a network client may call Dial
// with any context: keep the Host from the operation and pass its own. The
// Host belongs to the session, and Dial on it fails with ErrSessionEnded once
// that is over. The conn is a net.Conn: calls of the same method are
// serialised, each Write whole, and Close and deadlines may come from any
// goroutine; its addresses are placeholders. Close it: an open conn holds one
// of the sidecar's tunnels at srelens until it is closed or the session ends.
func (h *Host) Dial(ctx context.Context, cc CallContext, target PodTarget) (net.Conn, error) {
	if h.absent {
		return nil, ErrNoSession
	}
	if !h.atLeast("0.3.0") {
		return nil, fmt.Errorf("%w: pod tunnels require sidecar API 0.3.0", ErrInvalidCall)
	}
	if (target.Pod == "") == (target.Service == "") {
		return nil, fmt.Errorf("%w: name exactly one of Pod and Service", ErrInvalidCall)
	}
	if cc.Namespace == nil {
		return nil, fmt.Errorf("%w: select a namespace to dial in", ErrInvalidCall)
	}
	fields := []field{{"capability", target.Capability, protocol.IsIdentifier, identifierRule}}
	optional := func(name, value string) *string {
		if value == "" {
			return nil
		}
		fields = append(fields, field{name, value, protocol.IsObjectName, objectNameRule})
		return &value
	}
	params := protocol.HostConnectParams{
		Context:    protocol.JobContext{ClusterID: cc.ClusterID, Namespace: *cc.Namespace},
		Capability: target.Capability,
		Name:       optional("object", target.Object),
		Pod:        optional("pod", target.Pod),
		Service:    optional("service", target.Service),
	}
	if err := check(cc, fields...); err != nil {
		return nil, err
	}
	// srelens refuses a tunnel id that is not greater than every earlier one,
	// so an id is taken and its request queued as one step: another Dial
	// cannot queue a greater id first. ponytail: Dials queue one at a time (a
	// full queue holds the next Dial up), a per-id sequencer if that matters.
	select {
	case h.order <- struct{}{}:
	case <-h.ended.Done():
		return nil, ErrSessionEnded
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	// The conn is registered before its request is queued, so a tunnel/data
	// that arrives before the answer has somewhere to go.
	c, err := h.openTunnel()
	if err != nil {
		<-h.order
		return nil, err
	}
	params.Tunnel = c.id
	raw, err := json.Marshal(params)
	if err != nil {
		h.forgetTunnel(c.id)
		<-h.order
		return nil, err
	}
	id, p, err := h.submit(ctx, protocol.MethodHostConnect, raw)
	<-h.order
	if err != nil {
		h.forgetTunnel(c.id)
		return nil, err
	}
	answer, err := h.await(ctx, id, p)
	if err != nil {
		h.forgetTunnel(c.id) // a cancelled call leaves no conn: later frames for the id are dropped
		if ctx.Err() != nil && errors.Is(err, ctx.Err()) {
			// srelens may have accepted the connect before the $/cancelRequest
			// await sent reached it. The pipe is ordered: if the cancel won, this
			// close is a frame for a tunnel that is not open and is dropped; if the
			// answer won, it ends the tunnel, which would otherwise hold one of the
			// sidecar's tunnels until the session ends.
			c.sendClose(nil)
		}
		return nil, err
	}
	var result protocol.HostConnectResult
	if err := json.Unmarshal(answer, &result); err != nil {
		_ = c.Close() // srelens opened it
		return nil, fmt.Errorf("%w: host/connect's answer: %v", ErrProtocol, err)
	}
	c.remote = tunnelAddr("pod/" + result.Pod)
	return c, nil
}

func (h *Host) openTunnel() (*tunnelConn, error) {
	h.tmu.Lock()
	defer h.tmu.Unlock()
	if h.tunnels == nil {
		return nil, ErrSessionEnded
	}
	h.lastID++
	c := newTunnelConn(h, h.lastID)
	h.tunnels[c.id] = c
	return c, nil
}

func (h *Host) forgetTunnel(id uint64) {
	h.tmu.Lock()
	delete(h.tunnels, id)
	h.tmu.Unlock()
}

// endTunnels ends every conn: the session is over, and nothing more is written.
func (h *Host) endTunnels() {
	h.tmu.Lock()
	conns := h.tunnels
	h.tunnels = nil
	h.tmu.Unlock()
	for _, c := range conns {
		c.sessionEnded()
	}
}

// tunnelFrame routes srelens's tunnel/* notification to its conn. It runs on
// the session's reader goroutine and never waits on a conn: a frame for a
// conn that is not there is dropped (a close and a frame cross on the pipe),
// and one that breaks the protocol closes that conn alone.
func (h *Host) tunnelFrame(note protocol.Notification) {
	var reason string
	var c *tunnelConn
	lookup := func(id uint64) *tunnelConn {
		h.tmu.Lock()
		defer h.tmu.Unlock()
		return h.tunnels[id]
	}
	switch note.Method {
	case protocol.MethodTunnelData:
		var p protocol.TunnelDataParams
		if json.Unmarshal(note.Params, &p) != nil {
			return
		}
		if c = lookup(p.Tunnel); c != nil {
			b, err := base64.StdEncoding.DecodeString(p.Data)
			if err != nil {
				reason = "tunnel/data is not base64"
			} else {
				reason = c.deliver(b)
			}
		}
	case protocol.MethodTunnelCredit:
		var p protocol.TunnelCreditParams
		if json.Unmarshal(note.Params, &p) != nil {
			return
		}
		if c = lookup(p.Tunnel); c != nil {
			reason = c.grant(p.Bytes)
		}
	case protocol.MethodTunnelClose:
		var p protocol.TunnelCloseParams
		if json.Unmarshal(note.Params, &p) != nil {
			return
		}
		if c = lookup(p.Tunnel); c != nil {
			c.peerClosed(p.Message)
		}
	}
	if reason != "" {
		// ponytail: one goroutine per violating conn, which the table bounds,
		// because queueing the close may wait for room and this must not.
		if first, gone := c.finish(); first && !gone {
			go c.sendClose(&reason)
		}
	}
}

// tunnelConn is one tunnel as a net.Conn. The window each way is
// tunnelWindow: in holds at most what srelens was granted, so tunnelFrame
// appends without waiting, and Write spends credit srelens returns.
type tunnelConn struct {
	h      *Host
	id     uint64
	remote net.Addr
	rd, wd deadline   // the read and write deadlines; fired for good when the conn ends, wd too
	dmu    sync.Mutex // held while the deadlines are set, so the paired ones never differ

	betweenDeadlines func()        // a test's hook, between a read deadline and its credit twin; nil
	cd               deadline      // bounds the credit Read sends: the read deadline, and fired for good when the conn ends
	rlock            chan struct{} // held for the whole of a Read
	wlock            chan struct{} // held for the whole of a Write
	rsig             chan struct{} // woken by data, a close, or the session's end
	wsig             chan struct{} // woken by credit, a close, or the session's end

	mu      sync.Mutex
	in      bytes.Buffer
	allowed int   // what srelens may still send before more credit is returned
	owed    int   // read, and not yet returned as credit
	credit  int   // what the conn may still send
	closed  bool  // Close was called, or the conn broke the protocol
	gone    error // why srelens ended the tunnel, or the session ended
}

func newTunnelConn(h *Host, id uint64) *tunnelConn {
	return &tunnelConn{
		h: h, id: id, remote: tunnelAddr("pod"),
		rd: newDeadline(), wd: newDeadline(), cd: newDeadline(),
		rlock: make(chan struct{}, 1), wlock: make(chan struct{}, 1), rsig: make(chan struct{}, 1), wsig: make(chan struct{}, 1),
		allowed: tunnelWindow, credit: tunnelWindow,
	}
}

type tunnelAddr string

func (a tunnelAddr) Network() string { return "srelens-tunnel" }
func (a tunnelAddr) String() string  { return string(a) }

func (c *tunnelConn) LocalAddr() net.Addr  { return tunnelAddr("sidecar") }
func (c *tunnelConn) RemoteAddr() net.Addr { return c.remote }

func wake(sig chan struct{}) {
	select {
	case sig <- struct{}{}:
	default:
	}
}

func (c *tunnelConn) wake() {
	wake(c.rsig)
	wake(c.wsig)
}

// Read reads what srelens sent, and returns that many bytes of credit: the
// sender's window grows as the application consumes, not as frames arrive.
func (c *tunnelConn) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	if err := c.acquire(c.rlock, &c.rd, false); err != nil {
		return 0, err
	}
	defer func() { <-c.rlock }()
	c.flushCredit() // credit an earlier Read could not send
	for {
		c.mu.Lock()
		switch {
		case c.closed:
			c.mu.Unlock()
			return 0, net.ErrClosed
		case c.rd.expired():
			c.mu.Unlock()
			return 0, os.ErrDeadlineExceeded
		case c.in.Len() > 0:
			n, _ := c.in.Read(p)
			c.owed += n
			c.mu.Unlock()
			// ponytail: one tunnel/credit per Read; coalesce if small reads
			// make the pipe chatty.
			c.flushCredit()
			return n, nil
		case c.gone != nil:
			err := c.gone
			c.mu.Unlock()
			return 0, err
		}
		c.mu.Unlock()
		select {
		case <-c.rsig:
		case <-c.rd.wait():
		}
	}
}

// flushCredit sends what has been read as credit. If it cannot be sent (the
// read deadline passed) it is owed still, and the next Read sends it: srelens
// would otherwise wait on credit for good. Once the conn has ended the credit
// is dropped: the tunnel is gone.
func (c *tunnelConn) flushCredit() {
	c.mu.Lock()
	n := c.owed
	if n == 0 || c.closed || c.gone != nil {
		c.mu.Unlock()
		return
	}
	c.owed = 0
	c.allowed += n
	c.mu.Unlock()
	params, _ := json.Marshal(protocol.TunnelCreditParams{Tunnel: c.id, Bytes: uint64(n)})
	if c.h.out.send(c.h.out.general, protocol.Notification{Method: protocol.MethodTunnelCredit, Params: params}, c.cd.wait()) != nil {
		c.mu.Lock()
		c.owed += n
		c.allowed -= n
		c.mu.Unlock()
	}
}

// Write sends p in chunks of at most 64 KiB, each spending credit, and waits
// for credit when it has none.
func (c *tunnelConn) Write(p []byte) (int, error) {
	if err := c.acquire(c.wlock, &c.wd, true); err != nil {
		return 0, err
	}
	defer func() { <-c.wlock }()
	sent := 0
	for len(p) > 0 {
		n, err := c.reserve(len(p))
		if err != nil {
			return sent, err
		}
		params, _ := json.Marshal(protocol.TunnelDataParams{Tunnel: c.id, Data: base64.StdEncoding.EncodeToString(p[:n])})
		if err := c.h.out.send(c.h.out.general, protocol.Notification{Method: protocol.MethodTunnelData, Params: params}, c.wd.wait()); err != nil {
			c.mu.Lock()
			c.credit += n // never sent
			c.mu.Unlock()
			if errors.Is(err, errStopped) {
				return sent, c.stoppedErr(true)
			}
			return sent, pipeFailed(c.h.out.err)
		}
		p = p[n:]
		sent += n
	}
	return sent, nil
}

// reserve takes up to want bytes (at most one chunk) of credit, waiting for
// some.
func (c *tunnelConn) reserve(want int) (int, error) {
	for {
		c.mu.Lock()
		switch {
		case c.closed:
			c.mu.Unlock()
			return 0, net.ErrClosed
		case c.gone != nil:
			err := c.writeErr()
			c.mu.Unlock()
			return 0, err
		case c.wd.expired():
			c.mu.Unlock()
			return 0, os.ErrDeadlineExceeded
		case c.credit > 0:
			n := min(want, c.credit, maxChunk)
			c.credit -= n
			c.mu.Unlock()
			return n, nil
		}
		c.mu.Unlock()
		select {
		case <-c.wsig:
		case <-c.wd.wait():
		}
	}
}

// acquire takes a Read's or a Write's lock, waiting behind another caller
// until Close or the deadline ends the wait.
func (c *tunnelConn) acquire(lock chan struct{}, d *deadline, write bool) error {
	select {
	case lock <- struct{}{}:
		return nil
	case <-d.wait():
		return c.stoppedErr(write)
	}
}

// writeErr is what a Write returns once srelens has ended the tunnel or the
// session has: the reason, which is io.ErrClosedPipe for a close with no
// message. c.mu is held.
func (c *tunnelConn) writeErr() error {
	if c.gone == io.EOF {
		return io.ErrClosedPipe
	}
	return c.gone
}

// stoppedErr is why a wait stopped: Close, or (for a write) the tunnel's end,
// or else the deadline. A read's wait ends only by Close or its deadline: what
// srelens sent before closing is still read.
func (c *tunnelConn) stoppedErr(write bool) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	switch {
	case c.closed:
		return net.ErrClosed
	case write && c.gone != nil:
		return c.writeErr()
	}
	return os.ErrDeadlineExceeded
}

// Close ends the tunnel, and tells srelens unless it ended it first. A second
// Close returns net.ErrClosed.
func (c *tunnelConn) Close() error {
	first, gone := c.finish()
	if !first {
		return net.ErrClosed
	}
	if !gone {
		c.sendClose(nil)
	}
	return nil
}

// finish closes the conn on this side: whether this call did, and whether
// srelens had already closed the tunnel.
func (c *tunnelConn) finish() (first, gone bool) {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return false, false
	}
	c.closed, gone = true, c.gone != nil
	c.in.Reset()
	c.mu.Unlock()
	c.h.forgetTunnel(c.id)
	c.stopWaits()
	c.rd.fire()
	return true, gone
}

// sendClose queues tunnel/close, waiting for room only while the session lasts.
func (c *tunnelConn) sendClose(message *string) {
	params, _ := json.Marshal(protocol.TunnelCloseParams{Tunnel: c.id, Message: message})
	_ = c.h.out.send(c.h.out.general, protocol.Notification{Method: protocol.MethodTunnelClose, Params: params}, c.h.ended.Done())
}

// deliver buffers srelens's bytes; why not, if they break the protocol.
func (c *tunnelConn) deliver(b []byte) string {
	c.mu.Lock()
	defer c.mu.Unlock()
	switch {
	case c.closed || c.gone != nil || len(b) == 0:
	case len(b) > maxChunk:
		return fmt.Sprintf("tunnel/data carried %d bytes, over the %d a chunk may be", len(b), maxChunk)
	case len(b) > c.allowed:
		return fmt.Sprintf("tunnel/data went past the %d-byte window", tunnelWindow)
	default:
		c.allowed -= len(b)
		c.in.Write(b)
		wake(c.rsig)
	}
	return ""
}

// grant adds srelens's credit; why not, if it takes the window past its size.
func (c *tunnelConn) grant(n uint64) string {
	c.mu.Lock()
	defer c.mu.Unlock()
	switch {
	case c.closed || c.gone != nil || n == 0:
	case n > uint64(tunnelWindow-c.credit):
		return fmt.Sprintf("tunnel/credit took the window past %d bytes", tunnelWindow)
	default:
		c.credit += int(n)
		wake(c.wsig)
	}
	return ""
}

// peerClosed is srelens ending the tunnel: what it sent is still read, then
// Read returns why.
func (c *tunnelConn) peerClosed(message *string) {
	err := io.EOF
	if message != nil {
		err = fmt.Errorf("srelens closed the tunnel: %s", *message)
	}
	c.end(err)
}

// sessionEnded is the session ending: nothing more can be sent or received.
func (c *tunnelConn) sessionEnded() { c.end(ErrSessionEnded) }

func (c *tunnelConn) end(why error) {
	c.mu.Lock()
	if c.gone == nil && !c.closed {
		c.gone = why
	}
	c.mu.Unlock()
	c.h.forgetTunnel(c.id)
	c.stopWaits()
}

// stopWaits ends every wait the conn makes on the output queue, and wakes the
// callers waiting on data or credit. They find out why from the conn's state.
// The read deadline is left alone: bytes already received are still read.
func (c *tunnelConn) stopWaits() {
	c.wd.fire()
	c.cd.fire()
	c.wake()
}

func (c *tunnelConn) SetDeadline(t time.Time) error {
	c.dmu.Lock()
	defer c.dmu.Unlock()
	c.rd.set(t)
	if c.betweenDeadlines != nil {
		c.betweenDeadlines()
	}
	c.cd.set(t)
	c.wd.set(t)
	return nil
}

// SetReadDeadline sets the read deadline and the credit deadline that mirrors
// it together: a concurrent setter cannot leave them different.
func (c *tunnelConn) SetReadDeadline(t time.Time) error {
	c.dmu.Lock()
	defer c.dmu.Unlock()
	c.rd.set(t)
	if c.betweenDeadlines != nil {
		c.betweenDeadlines()
	}
	c.cd.set(t)
	return nil
}

func (c *tunnelConn) SetWriteDeadline(t time.Time) error { c.wd.set(t); return nil }

// deadline is a channel that closes when its time passes, as net.Pipe's is:
// set re-arms it, and fire closes it for good.
type deadline struct {
	mu     sync.Mutex
	timer  *time.Timer
	cancel chan struct{}
	dead   bool
}

func newDeadline() deadline { return deadline{cancel: make(chan struct{})} }

func (d *deadline) wait() <-chan struct{} {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.cancel
}

func (d *deadline) expired() bool {
	select {
	case <-d.wait():
		return true
	default:
		return false
	}
}

func (d *deadline) set(t time.Time) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if !d.dead {
		d.apply(t)
	}
}

func (d *deadline) fire() {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.dead = true
	d.apply(time.Unix(1, 0))
}

func (d *deadline) apply(t time.Time) {
	if d.timer != nil && !d.timer.Stop() {
		<-d.cancel // the timer fired: wait for it to close the channel
	}
	d.timer = nil
	closed := false
	select {
	case <-d.cancel:
		closed = true
	default:
	}
	if t.IsZero() {
		if closed {
			d.cancel = make(chan struct{})
		}
		return
	}
	if dur := time.Until(t); dur > 0 {
		if closed {
			d.cancel = make(chan struct{})
		}
		cancel := d.cancel
		d.timer = time.AfterFunc(dur, func() { close(cancel) })
		return
	}
	if !closed {
		close(d.cancel)
	}
}
