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

func TestPageReadCarriesPinnedScopeAndOpaqueCursor(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "page", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
		return sidecar.HostFrom(ctx).ReadPage(ctx, prod(t), "images", "next-page")
	})
	h := start(t, s)
	h.answer(h.request("initialize", initializeParams([]string{"0.2.0"}, h.DataDir, defaultLimits())))
	h.answer(h.request("activate", map[string]any{}))
	id := h.request("page", map[string]any{})
	call := h.call()
	want := map[string]any{"context": map[string]any{"clusterId": "kind-dev", "namespace": "team"}, "capability": "images", "cursor": "next-page"}
	if !reflect.DeepEqual(call["params"], want) {
		t.Fatalf("wrong page payload: %v", call)
	}
	h.reply(call["id"], map[string]any{"items": []any{}, "nextCursor": ""}, nil)
	if h.answer(id)["error"] != nil {
		t.Fatal("page response failed")
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestFirstPageIsExplicitlyRequestedWithoutChangingLegacyReads(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "page", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
		return sidecar.HostFrom(ctx).ReadPage(ctx, prod(t), "images", "")
	})
	h := start(t, s)
	h.answer(h.request("initialize", initializeParams([]string{"0.2.0"}, h.DataDir, defaultLimits())))
	h.answer(h.request("activate", map[string]any{}))
	id := h.request("page", map[string]any{})
	call := h.call()
	params := call["params"].(map[string]any)
	if cursor, present := params["cursor"]; !present || cursor != "" {
		t.Fatal("missing explicit first-page cursor")
	}
	h.reply(call["id"], map[string]any{"items": []any{}}, nil)
	h.answer(id)
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
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

func TestBindingAvailabilityNeedsNewProtocolAndPreservesPinnedContext(t *testing.T) {
	for _, version := range []string{"0.1.0", "0.2.0"} {
		t.Run(version, func(t *testing.T) {
			s := sidecar.New("t", "1")
			sidecar.Operation(s, "availability", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
				return sidecar.HostFrom(ctx).BindingAvailability(ctx, prod(t), []string{"vulnerability-reports", "sbom-reports"})
			})
			h := start(t, s)
			init := h.request("initialize", initializeParams([]string{version}, h.DataDir, defaultLimits()))
			if h.answer(init)["error"] != nil {
				t.Fatal("protocol not supported")
			}
			h.answer(h.request("activate", map[string]any{}))
			id := h.request("availability", map[string]any{})
			if version == "0.1.0" {
				if h.answer(id)["error"] == nil {
					t.Fatal("old host accepted new callback")
				}
			} else {
				call := h.call()
				want := map[string]any{"context": map[string]any{"clusterId": "kind-dev", "namespace": "team"}, "bindings": []any{"vulnerability-reports", "sbom-reports"}}
				if call["method"] != "host/bindingAvailability" || !reflect.DeepEqual(call["params"], want) {
					t.Fatalf("wrong discovery call: %v", call)
				}
				h.reply(call["id"], map[string]any{"bindings": []any{map[string]any{"binding": "vulnerability-reports", "state": "absent"}}}, nil)
				if h.answer(id)["error"] != nil {
					t.Fatal("discovery response failed")
				}
			}
			if err := h.finish(); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestJobCallbackNamesItsNamespaceAndRejectsOldProtocol(t *testing.T) {
	for _, version := range []string{"0.1.0", "0.2.0"} {
		t.Run(version, func(t *testing.T) {
			s := sidecar.New("t", "1")
			sidecar.Operation(s, "scan", func(ctx context.Context, _ struct{}) (json.RawMessage, error) {
				return sidecar.HostFrom(ctx).RunJob(ctx, prod(t), "scan-namespace", map[string]string{"namespace": "team"})
			})
			h := start(t, s)
			h.answer(h.request("initialize", initializeParams([]string{version}, h.DataDir, defaultLimits())))
			h.answer(h.request("activate", map[string]any{}))
			id := h.request("scan", map[string]any{})
			if version == "0.1.0" {
				if h.answer(id)["error"] == nil {
					t.Fatal("old protocol accepted Job callback")
				}
			} else {
				call := h.call()
				want := map[string]any{"context": map[string]any{"clusterId": "kind-dev", "namespace": "team"}, "capability": "scan-namespace", "inputs": map[string]any{"namespace": "team"}}
				if call["method"] != "host/runJob" || !reflect.DeepEqual(call["params"], want) {
					t.Fatalf("wrong Job call: %v", call)
				}
				h.reply(call["id"], map[string]any{"path": "job-result.json"}, nil)
				if h.answer(id)["error"] != nil {
					t.Fatal("Job response failed")
				}
			}
			if err := h.finish(); err != nil {
				t.Fatal(err)
			}
		})
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

// An end-to-end smoke check: when the input ends while a handler is blocked
// in a host call on its own ctx, the sidecar writes no stray
// $/cancelRequest after the session has ended (finish fails on any line this
// test never read). It cannot reliably catch end() cancelling handlers
// before it disconnects the host; TestEndDisconnectsTheHostBeforeItCancelsHandlers
// in host_internal_test.go is the regression test for that order.
func TestEndDisconnectsTheHostBeforeCancellingHandlers(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
		_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
		return struct{}{}, err
	})
	h := start(t, s)
	h.initialize()
	h.request("read", map[string]any{})
	h.call() // the host/read request; never answered
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

// shutdown ends the session as the end of input does: a handler blocked in
// a host call on its own ctx sends no $/cancelRequest, before or after the
// answer {}. ended fails on any line the sidecar wrote that this test never
// read. host_internal_test.go holds the deterministic check of the order.
func TestShutdownWhileAHandlerWaitsOnTheHostSendsOnlyItsAnswer(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "read", func(ctx context.Context, _ struct{}) (struct{}, error) {
		_, err := sidecar.HostFrom(ctx).Read(ctx, prod(t), "apps")
		return struct{}{}, err
	})
	h := start(t, s)
	h.initialize()
	h.request("read", map[string]any{})
	h.call() // the host/read request; never answered
	id := h.request("shutdown", map[string]any{})
	if r := h.answer(id)["result"]; !reflect.DeepEqual(r, map[string]any{}) {
		t.Fatalf("shutdown answered %v", r)
	}
	if err := h.ended(); err != nil {
		t.Fatalf("the session ended with %v", err)
	}
}
