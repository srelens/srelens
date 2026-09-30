package sidecar_test

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

func prod(t testing.TB) sidecar.CallContext {
	t.Helper()
	team := "team"
	cc, err := sidecar.NewCallContext("kind-dev", &team)
	if err != nil {
		t.Fatal(err)
	}
	return cc
}

func TestEachHostCallNamesItsContextAndGetsTheHostsAnswer(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
		return sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
	})
	sidecar.Operation(s, "resource", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
		return sidecar.HostFrom(ctx).Resource(ctx, prod(t), "apps", "web")
	})
	sidecar.Operation(s, "action", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
		return sidecar.HostFrom(ctx).Action(ctx, prod(t), "apps", "web", "sync", "u-1", "42")
	})
	h := start(t, s)
	h.initialize()
	named := map[string]any{"clusterId": "kind-dev", "namespace": "team"}
	for _, c := range []struct {
		op, method string
		params     map[string]any
	}{
		{"read", "host/read", map[string]any{"context": named, "capability": "apps"}},
		{"resource", "host/resource", map[string]any{"context": named, "capability": "apps", "name": "web"}},
		{"action", "host/action", map[string]any{"context": named, "capability": "apps", "name": "web",
			"action": "sync", "uid": "u-1", "resourceVersion": "42"}},
	} {
		id := h.request(c.op, map[string]any{})
		call := h.call()
		if call["method"] != c.method || !reflect.DeepEqual(call["params"], c.params) {
			t.Fatalf("%s sent %v", c.op, call)
		}
		if callID, _ := call["id"].(string); !strings.HasPrefix(callID, "c-") {
			t.Fatalf("the call's id: %v", call["id"])
		}
		h.reply(call["id"], map[string]any{"answered": c.method}, nil)
		if r := h.answer(id)["result"]; !reflect.DeepEqual(r, map[string]any{"answered": c.method}) {
			t.Fatalf("%s answered %v", c.op, r)
		}
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestTheHostsRefusalsBecomeErrorsByCode(t *testing.T) {
	s := sidecar.New("t", "1")
	var mu sync.Mutex
	var seen []error
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
		_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
		mu.Lock()
		seen = append(seen, err)
		mu.Unlock()
		return struct{}{}, nil
	})
	h := start(t, s)
	h.initialize()
	for _, c := range []struct {
		code    int
		message string
	}{{-32002, "declined"}, {-32003, "no grant"}, {-32602, "bad"}, {-32800, "cancelled"}, {-32601, "no such"}} {
		id := h.request("read", map[string]any{})
		call := h.call()
		h.reply(call["id"], nil, map[string]any{"code": c.code, "message": c.message})
		h.answer(id)
	}
	for i, sentinel := range []error{sidecar.ErrConsentDenied, sidecar.ErrCapabilityFailed, sidecar.ErrInvalidParams, sidecar.ErrCallCancelled} {
		if !errors.Is(seen[i], sentinel) {
			t.Errorf("answer %d: %v is not %v", i, seen[i], sentinel)
		}
	}
	var he *sidecar.HostError
	if !errors.As(seen[1], &he) || he.Message != "no grant" || he.Code != -32003 {
		t.Errorf("%v", seen[1])
	}
	if !errors.As(seen[4], &he) || he.Code != -32601 || errors.Is(seen[4], sidecar.ErrInvalidParams) {
		t.Errorf("%v", seen[4])
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestACallSrelensWouldRefuseIsRefusedBeforeItIsSent(t *testing.T) {
	s := sidecar.New("t", "1")
	upperCase := "Team" // not a Kubernetes namespace name
	// Built by hand, as CallContext's fields allow, past NewCallContext's checks.
	cases := map[string]func(context.Context) error{
		"clusterId": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Read(ctx, sidecar.CallContext{ClusterID: "\u00a0"}, "apps")
			return err
		},
		"namespace": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Read(ctx, sidecar.CallContext{ClusterID: "prod", Namespace: &upperCase}, "apps")
			return err
		},
		"capability": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "two words")
			return err
		},
		"name": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Resource(ctx, prod(t), "apps", "..")
			return err
		},
		"action": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Action(ctx, prod(t), "apps", "web", "under_score", "u-1", "42")
			return err
		},
		"uid": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Action(ctx, prod(t), "apps", "web", "sync", "u 1", "42")
			return err
		},
		"resourceVersion": func(ctx context.Context) error {
			_, err := sidecar.HostFrom(ctx).Action(ctx, prod(t), "apps", "web", "sync", "u-1", "")
			return err
		},
	}
	for field, call := range cases {
		// Each operation answers with its call's refusal, as text.
		sidecar.Operation(s, "bad-"+strings.ToLower(field), func(ctx context.Context, _ struct{}) (string, error) {
			return call(ctx).Error(), nil
		})
	}
	h := start(t, s)
	h.initialize()
	for field := range cases {
		id := h.request("bad-"+strings.ToLower(field), map[string]any{})
		// The operation's own answer is the next line: no call reached srelens.
		message, _ := h.answer(id)["result"].(string)
		if !strings.Contains(message, "`"+field+"`") && !strings.Contains(message, "`context."+field+"`") {
			t.Errorf("%s: %q does not name the field", field, message)
		}
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestARefusedCallMatchesErrInvalidCall(t *testing.T) {
	s := sidecar.New("t", "1")
	result := make(chan error, 1)
	sidecar.Operation(s, "bad", func(ctx context.Context, _ struct{}) (struct{}, error) {
		_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "")
		result <- err
		return struct{}{}, nil
	})
	h := start(t, s)
	h.initialize()
	h.answer(h.request("bad", map[string]any{}))
	if err := <-result; !errors.Is(err, sidecar.ErrInvalidCall) {
		t.Fatalf("%v", err)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestACallTheSidecarStopsWaitingForIsCancelledAtTheHost(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
		_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
		return struct{}{}, err
	})
	h := start(t, s)
	h.initialize()
	id := h.request("read", map[string]any{})
	call := h.call()
	h.notify("$/cancelRequest", map[string]any{"id": id})
	// srelens's cancel answer, and the sidecar's cancel of its own call, in either order.
	lines := []map[string]any{h.recv(), h.recv()}
	if lines[0]["method"] != nil {
		lines[0], lines[1] = lines[1], lines[0]
	}
	if e := errorOf(t, lines[0]); e.code != -32800 {
		t.Fatalf("%v", lines[0])
	}
	want := map[string]any{"jsonrpc": "2.0", "method": "$/cancelRequest", "params": map[string]any{"id": call["id"]}}
	if !reflect.DeepEqual(lines[1], want) {
		t.Fatalf("%v", lines[1])
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestACancelledCallKeepsItsSlotUntilSrelensAnswers(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
		_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
		return struct{}{}, err
	})
	h := start(t, s)
	limits := defaultLimits()
	limits["maxConcurrentRequests"] = 1
	h.initializeWith(limits)
	first := h.request("read", map[string]any{})
	call := h.call()
	h.notify("$/cancelRequest", map[string]any{"id": first})
	h.recv()
	h.recv() // the -32800 answer and the $/cancelRequest, in either order
	second := h.request("read", map[string]any{})
	if m, ok := h.nextWithin(200 * time.Millisecond); ok {
		t.Fatalf("a second call went out while the cancelled one held the only slot: %v", m)
	}
	h.reply(call["id"], nil, map[string]any{"code": -32800, "message": "cancelled"})
	next := h.call()
	h.reply(next["id"], map[string]any{}, nil)
	h.answer(second)
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAtMostTheHostsLimitOfCallsAreInFlight(t *testing.T) {
	for _, c := range []struct{ limit, want int }{{8, 8}, {2, 2}, {20, 8}} {
		s := sidecar.New("t", "1")
		sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
			_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
			return struct{}{}, err
		})
		h := start(t, s)
		limits := defaultLimits()
		limits["maxConcurrentRequests"] = c.limit
		h.initializeWith(limits)
		for i := 0; i < c.want+1; i++ {
			h.request("read", map[string]any{})
		}
		var calls []map[string]any
		for i := 0; i < c.want; i++ {
			calls = append(calls, h.call())
		}
		if m, ok := h.nextWithin(200 * time.Millisecond); ok {
			t.Fatalf("limit %d: call %d went out: %v", c.limit, c.want+1, m)
		}
		h.reply(calls[0]["id"], map[string]any{}, nil)
		answers := 0
		for answers < 1 || len(calls) < c.want+1 {
			m := h.recv()
			if m["method"] == "host/read" {
				calls = append(calls, m)
			} else {
				answers++
			}
		}
		for _, call := range calls[1:] {
			h.reply(call["id"], map[string]any{}, nil)
		}
		for i := 0; i < c.want; i++ {
			h.recv()
		}
		if err := h.finish(); err != nil {
			t.Fatal(err)
		}
	}
}

func TestCallsWaitingWhenTheSessionEndsGetErrSessionEnded(t *testing.T) {
	s := sidecar.New("t", "1")
	hosts := make(chan *sidecar.Host, 1)
	outcome := make(chan error, 1)
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
		host := sidecar.HostFrom(ctx)
		hosts <- host
		detached := context.WithoutCancel(ctx) // outlives the handler
		go func() {
			_, err := host.Read(detached, prod(t), "apps")
			outcome <- err
		}()
		return struct{}{}, nil
	})
	h := start(t, s)
	h.initialize()
	id := h.request("read", map[string]any{})
	// The operation's answer and the detached call, in either order.
	lines := []map[string]any{h.recv(), h.recv()}
	if lines[0]["method"] != nil {
		lines[0], lines[1] = lines[1], lines[0]
	}
	if lines[0]["id"] != float64(id) || lines[1]["method"] != "host/read" {
		t.Fatalf("%v", lines)
	}
	host := <-hosts
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-outcome:
		if !errors.Is(err, sidecar.ErrSessionEnded) {
			t.Fatalf("the waiting call got %v", err)
		}
	case <-time.After(wait):
		t.Fatal("the waiting call never returned")
	}
	began := time.Now()
	if _, err := host.Read(context.Background(), prod(t), "apps"); !errors.Is(err, sidecar.ErrSessionEnded) {
		t.Fatalf("a later call got %v", err)
	}
	if time.Since(began) > time.Second {
		t.Fatal("a call after the end waited")
	}
}

func TestHostFromAContextTheSDKDidNotMakeFailsWithErrNoSession(t *testing.T) {
	_, err := sidecar.HostFrom(context.Background()).Read(context.Background(), prod(t), "apps")
	if !errors.Is(err, sidecar.ErrNoSession) {
		t.Fatalf("%v", err)
	}
}
