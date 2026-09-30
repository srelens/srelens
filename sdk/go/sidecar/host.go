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
// sooner.

// Host is the way to srelens from a handler: HostFrom(ctx).
type Host struct {
	out     *outbox
	next    atomic.Uint64
	slots   chan struct{}
	gone    chan struct{} // closed by disconnect
	mu      sync.Mutex
	waiting map[string]chan protocol.Response // by id key; nil once disconnected
	absent  bool                              // no session: every call fails with ErrNoSession
}

func newHost(out *outbox, limits protocol.InitializeLimits) *Host {
	n := maxHostCalls
	if m := limits.MaxConcurrentRequests; m >= 1 && m < uint64(n) {
		n = int(m)
	}
	return &Host{
		out:     out,
		slots:   make(chan struct{}, n),
		gone:    make(chan struct{}),
		waiting: map[string]chan protocol.Response{},
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
		return fmt.Errorf("%w: `context.namespace` must be a Kubernetes namespace name, or null for none; not %q",
			ErrInvalidCall, *cc.Namespace)
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
	case <-h.gone:
		return nil, ErrSessionEnded
	default:
	}
	select {
	case h.slots <- struct{}{}:
	case <-h.gone:
		return nil, ErrSessionEnded
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	id := protocol.StringID(fmt.Sprintf("c-%d", h.next.Add(1)))
	answer := make(chan protocol.Response, 1)
	h.mu.Lock()
	if h.waiting == nil {
		h.mu.Unlock()
		<-h.slots
		return nil, ErrSessionEnded
	}
	h.waiting[id.Key()] = answer
	h.mu.Unlock()
	err = h.out.send(h.out.general, protocol.Request{ID: id, Method: method, Params: raw}, ctx.Done())
	if err != nil {
		h.forget(id)
		var over *errOverLimit
		switch {
		case errors.As(err, &over):
			return nil, &CallTooLargeError{Bytes: over.bytes}
		case errors.Is(err, errStopped):
			return nil, ctx.Err()
		default:
			return nil, ErrSessionEnded
		}
	}
	select {
	case resp := <-answer:
		if resp.Error != nil {
			return nil, &HostError{Code: resp.Error.Code, Message: resp.Error.Message, Data: resp.Error.Data}
		}
		return resp.Result, nil
	case <-h.gone:
		return nil, ErrSessionEnded
	case <-ctx.Done():
		h.cancelCall(id)
		return nil, ctx.Err()
	}
}

// forget drops a call whose request never reached the queue, freeing its slot.
func (h *Host) forget(id protocol.RequestID) {
	h.mu.Lock()
	if h.waiting != nil {
		delete(h.waiting, id.Key())
	}
	h.mu.Unlock()
	<-h.slots
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
	_ = h.out.send(h.out.general, protocol.Notification{Method: protocol.MethodCancelRequest, Params: params}, h.gone)
}

// answered routes srelens's answer to the call waiting for it, and frees that
// call's slot. An answer to no waiting call is dropped.
func (h *Host) answered(resp protocol.Response) {
	h.mu.Lock()
	answer, ok := h.waiting[resp.ID.Key()]
	if ok {
		delete(h.waiting, resp.ID.Key())
	}
	h.mu.Unlock()
	if !ok {
		return
	}
	<-h.slots
	answer <- resp
}

// disconnect answers every waiting call ErrSessionEnded, and refuses later
// ones at once. The session calls it once, before it stops its handlers, so a
// call a handler abandons as it stops sends nothing.
func (h *Host) disconnect() {
	h.mu.Lock()
	h.waiting = nil
	h.mu.Unlock()
	close(h.gone)
}
