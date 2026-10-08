package sidecar

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"sync/atomic"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// maxHostCalls is the most calls srelens works on at once from one sidecar
// (and it stops a sidecar with 16 unanswered); fewer when initialize's
// maxConcurrentRequests says so.
const maxHostCalls = 8

// Whether a call's request reached the queue decides what happens to its
// slot when its caller stops waiting. If it did not, srelens never saw the
// id: the slot is freed at once, and nothing is sent. If it did, srelens is
// working on the call and will answer it, -32800 for a cancelled one: the
// slot stays taken until then, and a $/cancelRequest asks for that answer
// sooner. The session's end frees every slot still taken.
//
// Each slot is freed exactly once. Once a call has its id, the one of
// answered, forget and disconnect that finds it still holding its slot
// frees it; the others find it already freed.

// Host is the way to srelens from a handler: HostFrom(ctx).
type Host struct {
	out     *outbox
	next    atomic.Uint64
	slots   chan struct{}
	ended   context.Context    // done once disconnected
	end     context.CancelFunc // ends ended
	mu      sync.Mutex
	waiting map[string]*pending // by id key; nil once disconnected
	absent  bool                // no session: every call fails with ErrNoSession
}

// pending is a call srelens has not answered: where its answer goes, and
// whether it still holds its slot, which is read and cleared under Host.mu.
type pending struct {
	answer    chan protocol.Response
	holdsSlot bool
}

// letGo clears p's hold on its slot; whether it held it, and so whether the
// caller must free it. Host.mu is held.
func (p *pending) letGo() bool {
	held := p.holdsSlot
	p.holdsSlot = false
	return held
}

func newHost(out *outbox, limits protocol.InitializeLimits) *Host {
	n := maxHostCalls
	if m := limits.MaxConcurrentRequests; m >= 1 && m < uint64(n) {
		n = int(m)
	}
	ended, end := context.WithCancel(context.Background())
	return &Host{
		out:     out,
		slots:   make(chan struct{}, n),
		ended:   ended,
		end:     end,
		waiting: map[string]*pending{},
	}
}

var noSession = &Host{absent: true}

// HostFrom is the way to srelens for the handler ctx belongs to. For a
// context the SDK did not make, such as a unit test's, every call fails with
// ErrNoSession.
func HostFrom(ctx context.Context) *Host {
	if sh := sharedFrom(ctx); sh != nil && sh.host != nil {
		return sh.host
	}
	return noSession
}

// The shapes srelens's broker holds fields to, in its words.
const (
	identifierRule = "1 to 64 ASCII letters, digits and hyphens, as the manifest names it"
	objectNameRule = "a Kubernetes object name: 1 to 253 ASCII letters, digits, dots and hyphens"
	tokenRule      = "1 to 128 printable ASCII characters, as the object carries it"
)

type field struct {
	name, value string
	fits        func(string) bool
	rule        string
}

// check refuses a call srelens would refuse, before it takes a slot.
func check(cc CallContext, fields ...field) error {
	if !protocol.IsClusterID(cc.ClusterID) {
		return fmt.Errorf("%w: `context.clusterId` must name a cluster, in at most %d bytes", ErrInvalidCall, protocol.MaxClusterIDBytes)
	}
	if cc.Namespace != nil && !protocol.IsNamespace(*cc.Namespace) {
		value := []rune(*cc.Namespace)
		if len(value) > 64 {
			value = value[:64]
		}
		return fmt.Errorf("%w: `context.namespace` must be a Kubernetes namespace name, or null for none; not %q",
			ErrInvalidCall, string(value))
	}
	for _, f := range fields {
		if !f.fits(f.value) {
			return fmt.Errorf("%w: `%s` must be %s", ErrInvalidCall, f.name, f.rule)
		}
	}
	return nil
}

// Read reads one of the app's declared readers, or one of its network.http
// requests, on the cluster cc names.
func (h *Host) Read(ctx context.Context, cc CallContext, capability string) (json.RawMessage, error) {
	if err := check(cc, field{"capability", capability, protocol.IsIdentifier, identifierRule}); err != nil {
		return nil, err
	}
	return h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{Context: cc, Capability: capability})
}

// ReadPage reads a bounded page from a declared image inventory binding (API 0.2).
func (h *Host) ReadPage(ctx context.Context, cc CallContext, capability, cursor string) (json.RawMessage, error) {
	if h.absent {
		return nil, ErrNoSession
	}
	if APIVersion(ctx) != "0.2.0" {
		return nil, fmt.Errorf("%w: paged reads require sidecar API 0.2.0", ErrInvalidCall)
	}
	if err := check(cc, field{"capability", capability, protocol.IsIdentifier, identifierRule}); err != nil {
		return nil, err
	}
	if len(cursor) > 8192 {
		return nil, fmt.Errorf("%w: cursor is at most 8192 printable bytes", ErrInvalidCall)
	}
	for _, b := range []byte(cursor) {
		if b < 33 || b > 126 {
			return nil, fmt.Errorf("%w: cursor is at most 8192 printable bytes", ErrInvalidCall)
		}
	}
	return h.call(ctx, protocol.MethodHostRead, protocol.HostReadParams{Context: cc, Capability: capability, Cursor: &cursor})
}

// BindingAvailability discovers only the app's declared resource readers.
// An older protocol is incompatible, never evidence that the Operator is absent.
func (h *Host) BindingAvailability(ctx context.Context, cc CallContext, bindings []string) (json.RawMessage, error) {
	if h.absent {
		return nil, ErrNoSession
	}
	if APIVersion(ctx) != "0.2.0" {
		return nil, fmt.Errorf("%w: binding discovery requires sidecar API 0.2.0", ErrInvalidCall)
	}
	if err := check(cc); err != nil {
		return nil, err
	}
	if !protocol.IsBindingNames(bindings) {
		return nil, fmt.Errorf("%w: choose 1–16 unique declared bindings", ErrInvalidCall)
	}
	return h.call(ctx, protocol.MethodHostBindingAvailability, protocol.HostBindingAvailabilityParams{Context: cc, Bindings: bindings})
}

// RunJob runs only a declared, digest-pinned container in cc's namespace.
// The host asks for confirmation and cleans up its Job on cancellation.
func (h *Host) RunJob(ctx context.Context, cc CallContext, capability string, inputs map[string]string) (json.RawMessage, error) {
	if h.absent {
		return nil, ErrNoSession
	}
	if APIVersion(ctx) != "0.2.0" {
		return nil, fmt.Errorf("%w: Jobs require sidecar API 0.2.0", ErrInvalidCall)
	}
	if err := check(cc, field{"capability", capability, protocol.IsIdentifier, identifierRule}); err != nil {
		return nil, err
	}
	if cc.Namespace == nil {
		return nil, fmt.Errorf("%w: select a namespace for the Job", ErrInvalidCall)
	}
	if len(inputs) > 16 {
		return nil, fmt.Errorf("%w: at most sixteen Job inputs", ErrInvalidCall)
	}
	for name, value := range inputs {
		if !protocol.IsIdentifier(name) || len(value) == 0 || value[0] == '-' || len(value) > 512 {
			return nil, fmt.Errorf("%w: invalid Job input", ErrInvalidCall)
		}
		for _, b := range []byte(value) {
			if b < ' ' || b > '~' {
				return nil, fmt.Errorf("%w: Job inputs must be printable ASCII", ErrInvalidCall)
			}
		}
	}
	if inputs == nil {
		inputs = map[string]string{}
	}
	return h.call(ctx, protocol.MethodHostRunJob, protocol.HostRunJobParams{Context: protocol.JobContext{ClusterID: cc.ClusterID, Namespace: *cc.Namespace}, Capability: capability, Inputs: inputs})
}

// Resource inspects object name of a declared custom-resource reader.
func (h *Host) Resource(ctx context.Context, cc CallContext, capability, name string) (json.RawMessage, error) {
	if err := check(cc,
		field{"capability", capability, protocol.IsIdentifier, identifierRule},
		field{"name", name, protocol.IsObjectName, objectNameRule},
	); err != nil {
		return nil, err
	}
	return h.call(ctx, protocol.MethodHostResource, protocol.HostResourceParams{Context: cc, Capability: capability, Name: name})
}

// Action runs one of the app's declared actions on object name, as read (uid,
// resourceVersion). srelens asks a person first.
func (h *Host) Action(ctx context.Context, cc CallContext, capability, name, action, uid, resourceVersion string) (json.RawMessage, error) {
	if err := check(cc,
		field{"capability", capability, protocol.IsIdentifier, identifierRule},
		field{"name", name, protocol.IsObjectName, objectNameRule},
		field{"action", action, protocol.IsIdentifier, identifierRule},
		field{"uid", uid, protocol.IsToken, tokenRule},
		field{"resourceVersion", resourceVersion, protocol.IsToken, tokenRule},
	); err != nil {
		return nil, err
	}
	return h.call(ctx, protocol.MethodHostAction, protocol.HostActionParams{
		Context: cc, Capability: capability, Name: name, Action: action, UID: uid, ResourceVersion: resourceVersion,
	})
}

func (h *Host) call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	if h.absent {
		return nil, ErrNoSession
	}
	raw, err := json.Marshal(params)
	if err != nil {
		return nil, err
	}
	select {
	case <-h.ended.Done():
		return nil, ErrSessionEnded
	default:
	}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	default:
	}
	select {
	case h.slots <- struct{}{}:
	case <-h.ended.Done():
		return nil, ErrSessionEnded
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	// select picks at random among ready cases: ctx may have ended at the
	// same instant a slot was free. Checked again now that a slot is held, so
	// a ctx that was already done before the queue is reached still sends
	// nothing and frees its slot at once, rather than reaching the queue.
	if ctx.Err() != nil {
		<-h.slots
		return nil, h.stopped(ctx)
	}
	id := protocol.StringID(fmt.Sprintf("c-%d", h.next.Add(1)))
	p := &pending{answer: make(chan protocol.Response, 1), holdsSlot: true}
	h.mu.Lock()
	if h.waiting == nil {
		h.mu.Unlock()
		<-h.slots
		return nil, ErrSessionEnded
	}
	h.waiting[id.Key()] = p
	h.mu.Unlock()
	err = h.queue(ctx, protocol.Request{ID: id, Method: method, Params: raw})
	if err != nil {
		h.forget(id, p)
		var over *errOverLimit
		switch {
		case errors.As(err, &over):
			return nil, &CallTooLargeError{Bytes: over.bytes}
		case errors.Is(err, errStopped):
			return nil, h.stopped(ctx)
		default:
			return nil, ErrSessionEnded
		}
	}
	return h.await(ctx, id, p)
}

// await waits for srelens's answer to call id, the caller's ctx to end, or
// the session to end. An answer that has already arrived wins: select picks
// at random among ready cases, and a call srelens answered, perhaps an
// action it ran, must not be reported as cancelled or cut off.
func (h *Host) await(ctx context.Context, id protocol.RequestID, p *pending) (json.RawMessage, error) {
	select {
	case resp := <-p.answer:
		return result(resp)
	case <-h.ended.Done():
		if resp, ok := arrived(p); ok {
			return result(resp)
		}
		return nil, ErrSessionEnded
	case <-ctx.Done():
		if resp, ok := arrived(p); ok {
			return result(resp)
		}
		h.cancelCall(id)
		return nil, h.stopped(ctx)
	}
}

// arrived is p's answer if srelens has already given it.
func arrived(p *pending) (protocol.Response, bool) {
	select {
	case resp := <-p.answer:
		return resp, true
	default:
		return protocol.Response{}, false
	}
}

// result is a call's outcome from srelens's answer.
func result(resp protocol.Response) (json.RawMessage, error) {
	if resp.Error != nil {
		return nil, &HostError{Code: resp.Error.Code, Message: resp.Error.Message, Data: resp.Error.Data}
	}
	return resp.Result, nil
}

// queue queues a call's request, waiting for room until the caller's ctx is
// done or the session ends (errStopped).
func (h *Host) queue(ctx context.Context, req protocol.Request) error {
	sending, stop := context.WithCancel(ctx)
	defer stop()
	defer context.AfterFunc(h.ended, stop)()
	return h.out.send(h.out.general, req, sending.Done())
}

// stopped is why a call stopped waiting: ErrSessionEnded once the session
// has ended, which also cancels a handler's ctx, and otherwise ctx's error.
func (h *Host) stopped(ctx context.Context) error {
	if h.ended.Err() != nil {
		return ErrSessionEnded
	}
	return ctx.Err()
}

// forget drops a call whose request never reached the queue, and frees its
// slot if it still holds it. It may not: srelens may already have answered
// the id (a race, or a misbehaving host) while the request waited for room,
// or the session ended, and either freed the slot.
func (h *Host) forget(id protocol.RequestID, p *pending) {
	h.mu.Lock()
	delete(h.waiting, id.Key())
	held := p.letGo()
	h.mu.Unlock()
	if held {
		<-h.slots
	}
}

// cancelCall asks srelens to stop call id, if it has not answered it and the
// session is still on.
func (h *Host) cancelCall(id protocol.RequestID) {
	h.mu.Lock()
	_, waiting := h.waiting[id.Key()]
	h.mu.Unlock()
	if !waiting {
		return
	}
	params, _ := json.Marshal(protocol.CancelParams{ID: id})
	_ = h.out.send(h.out.general, protocol.Notification{Method: protocol.MethodCancelRequest, Params: params}, h.ended.Done())
}

// answered routes srelens's answer to the call waiting for it, and frees that
// call's slot. An answer to no waiting call is dropped.
func (h *Host) answered(resp protocol.Response) {
	h.mu.Lock()
	p, ok := h.waiting[resp.ID.Key()]
	held := false
	if ok {
		delete(h.waiting, resp.ID.Key())
		held = p.letGo()
	}
	h.mu.Unlock()
	if !ok {
		return
	}
	if held {
		<-h.slots
	}
	p.answer <- resp
}

// disconnect answers every waiting call ErrSessionEnded, refuses later ones
// at once, and frees every slot still taken. The session calls it before it
// stops its handlers, so a call a handler abandons as it stops sends
// nothing; a second call does nothing.
func (h *Host) disconnect() {
	h.mu.Lock()
	held := 0
	for _, p := range h.waiting {
		if p.letGo() {
			held++
		}
	}
	h.waiting = nil
	h.mu.Unlock()
	h.end()
	for range held {
		<-h.slots
	}
}
