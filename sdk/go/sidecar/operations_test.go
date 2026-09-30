package sidecar_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

type addInput struct {
	A int `json:"a"`
	B int `json:"b"`
}

type sum struct {
	Sum int `json:"sum"`
}

func TestAnOperationGetsItsTypedInputAndAnswersItsOutput(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "add", func(_ context.Context, in addInput) (sum, error) {
		return sum{Sum: in.A + in.B}, nil
	})
	h := start(t, s)
	h.initialize()
	id := h.request("add", map[string]any{"a": 1, "b": 2})
	if r := h.answer(id)["result"]; !reflect.DeepEqual(r, map[string]any{"sum": float64(3)}) {
		t.Fatalf("answered %v", r)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestInputThatDoesNotDecodeIsInvalidParams(t *testing.T) {
	s := sidecar.New("t", "1")
	ran := atomic.Bool{}
	sidecar.Operation(s, "add", func(_ context.Context, in addInput) (sum, error) {
		ran.Store(true)
		return sum{}, nil
	})
	h := start(t, s)
	h.initialize()
	id := h.request("add", map[string]any{"a": "one"})
	if e := errorOf(t, h.answer(id)); e.code != -32602 {
		t.Fatalf("%+v", e)
	}
	if ran.Load() {
		t.Fatal("the handler ran on input that does not decode")
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAHandlersErrorIsAnsweredAsItSaid(t *testing.T) {
	s := sidecar.New("t", "1")
	failures := map[string]error{
		"quota":   sidecar.NewError(-32050, "quota").WithData(map[string]int{"left": 0}),
		"bad":     sidecar.InvalidParams("`name` is required"),
		"wrapped": fmt.Errorf("scan: %w", sidecar.InvalidParams("bad image")),
		"plain":   errors.New("disk full"),
	}
	for name, err := range failures {
		sidecar.Operation(s, name, func(context.Context, struct{}) (struct{}, error) { return struct{}{}, err })
	}
	sidecar.Operation(s, "cluster", func(context.Context, struct{}) (struct{}, error) {
		_, err := sidecar.NewCallContext("", nil)
		return struct{}{}, err
	})
	h := start(t, s)
	h.initialize()
	for _, c := range []struct {
		method  string
		code    float64
		message string
		data    any
	}{
		{"quota", -32050, "quota", map[string]any{"left": float64(0)}},
		{"bad", -32602, "`name` is required", nil},
		{"wrapped", -32602, "bad image", nil},
		{"plain", -32603, "disk full", nil},
		{"cluster", -32602, "`clusterId` must name a cluster, in at most 4096 bytes", nil},
	} {
		id := h.request(c.method, map[string]any{})
		e := errorOf(t, h.answer(id))
		if e.code != c.code || e.message != c.message || !reflect.DeepEqual(e.data, c.data) {
			t.Errorf("%s answered %+v", c.method, e)
		}
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAHandlerThatPanicsIsAnsweredInternalErrorAndTheSidecarGoesOn(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "boom", func(context.Context, struct{}) (struct{}, error) { panic("the index was out of range") })
	h := start(t, s)
	h.initialize()
	id := h.request("boom", map[string]any{})
	if e := errorOf(t, h.answer(id)); e.code != -32603 || e.message != "the handler for `boom` panicked" {
		t.Fatalf("%+v", e)
	}
	id = h.request("health", map[string]any{})
	h.answer(id)
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAResultOverTheMessageLimitIsAnsweredWithWhyInstead(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "big", func(context.Context, struct{}) (string, error) {
		return strings.Repeat("x", 5<<20), nil
	})
	h := start(t, s)
	h.initialize()
	id := h.request("big", map[string]any{})
	if e := errorOf(t, h.answer(id)); e.code != -32603 || !strings.Contains(e.message, "4 MiB") {
		t.Fatalf("%+v", e)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestACancelledRequestIsAnsweredRequestCancelledOnceAndItsHandlerIsTold(t *testing.T) {
	s := sidecar.New("t", "1")
	started := make(chan struct{})
	cause := make(chan error, 1)
	sidecar.Operation(s, "wait", func(ctx context.Context, _ struct{}) (string, error) {
		close(started)
		<-ctx.Done()
		cause <- context.Cause(ctx)
		return "late", nil
	})
	h := start(t, s)
	h.initialize()
	id := h.request("wait", map[string]any{})
	<-started
	h.notify("$/cancelRequest", map[string]any{"id": id})
	if e := errorOf(t, h.answer(id)); e.code != -32800 {
		t.Fatalf("%+v", e)
	}
	if got := <-cause; !errors.Is(got, sidecar.ErrCancelled) {
		t.Fatalf("the handler's cause was %v", got)
	}
	// The late result is dropped: the next line answers health.
	health := h.request("health", map[string]any{})
	h.answer(health)
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAHandlersContextIsDoneOnceItReturns(t *testing.T) {
	s := sidecar.New("t", "1")
	cause := make(chan error, 1)
	sidecar.Operation(s, "leak", func(ctx context.Context, _ struct{}) (struct{}, error) {
		go func() {
			<-ctx.Done()
			cause <- context.Cause(ctx)
		}()
		return struct{}{}, nil
	})
	h := start(t, s)
	h.initialize()
	h.answer(h.request("leak", map[string]any{}))
	select {
	case got := <-cause:
		if !errors.Is(got, sidecar.ErrHandlerReturned) {
			t.Fatalf("cause %v", got)
		}
	case <-time.After(wait):
		t.Fatal("the leaked goroutine's context was never done")
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

// Go's scheduler preempts a spinning goroutine, and a goroutine blocked in a
// sleep or a system call gives up its thread: health is answered on the
// reader, which no handler can starve.
func TestHealthIsAnsweredWhileEveryHandlerBlocksOrSpins(t *testing.T) {
	s := sidecar.New("t", "1")
	var release atomic.Bool
	started := make(chan struct{}, 8)
	sidecar.Operation(s, "sleep", func(context.Context, struct{}) (struct{}, error) {
		started <- struct{}{}
		for !release.Load() {
			time.Sleep(10 * time.Millisecond)
		}
		return struct{}{}, nil
	})
	sidecar.Operation(s, "spin", func(context.Context, struct{}) (struct{}, error) {
		started <- struct{}{}
		for !release.Load() {
		}
		return struct{}{}, nil
	})
	h := start(t, s)
	h.initialize()
	ids := map[float64]bool{}
	for i := 0; i < 4; i++ {
		ids[float64(h.request("sleep", map[string]any{}))] = true
		ids[float64(h.request("spin", map[string]any{}))] = true
	}
	for i := 0; i < 8; i++ {
		<-started
	}
	asked := time.Now()
	health := h.answer(h.request("health", map[string]any{}))
	if took := time.Since(asked); took > 500*time.Millisecond {
		t.Fatalf("health took %v", took)
	}
	if !reflect.DeepEqual(health["result"], map[string]any{}) {
		t.Fatalf("health answered %v", health)
	}
	release.Store(true)
	for i := 0; i < 8; i++ {
		m := h.recv()
		if !ids[m["id"].(float64)] {
			t.Fatalf("unexpected %v", m)
		}
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

type told struct {
	DataDir    string `json:"dataDir"`
	APIVersion string `json:"apiVersion"`
	MaxReq     uint64 `json:"maxRequests"`
}

func TestTheContextCarriesWhatInitializeSaid(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "told", func(ctx context.Context, _ struct{}) (told, error) {
		return told{sidecar.DataDir(ctx), sidecar.APIVersion(ctx), sidecar.Limits(ctx).MaxConcurrentRequests}, nil
	})
	h := start(t, s)
	h.initialize()
	r := h.answer(h.request("told", map[string]any{}))["result"]
	want := map[string]any{"dataDir": h.DataDir, "apiVersion": "0.1.0", "maxRequests": float64(8)}
	if !reflect.DeepEqual(r, want) {
		t.Fatalf("%v, want %v", r, want)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAContextTheSDKDidNotMakeHasNoSession(t *testing.T) {
	ctx := context.Background()
	if sidecar.DataDir(ctx) != "" || sidecar.APIVersion(ctx) != "" || sidecar.Limits(ctx).MaxConcurrentRequests != 0 {
		t.Fatal("a background context carries session values")
	}
}

func mustPanic(t *testing.T, what string, f func()) {
	t.Helper()
	defer func() {
		if recover() == nil {
			t.Errorf("%s did not panic", what)
		}
	}()
	f()
}

// TestATypedNilErrorIsAnsweredInternalErrorInsteadOfCrashing reproduces the
// bug where a handler that returns a typed-nil *sidecar.Error (a non-nil
// error interface holding a nil *Error) crashed the whole sidecar: asRPCError
// found the *Error via errors.As and dereferenced it unchecked.
func TestATypedNilErrorIsAnsweredInternalErrorInsteadOfCrashing(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "nilerr", func(context.Context, struct{}) (struct{}, error) {
		var serr *sidecar.Error
		return struct{}{}, serr
	})
	h := start(t, s)
	h.initialize()
	id := h.request("nilerr", map[string]any{})
	want := "the handler for `nilerr` returned a nil *sidecar.Error"
	if e := errorOf(t, h.answer(id)); e.code != -32603 || e.message != want {
		t.Fatalf("%+v, want message %q", e, want)
	}
	id = h.request("health", map[string]any{})
	h.answer(id)
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

// TestAnErrorWithInvalidJSONDataDropsTheDataAndStillAnswers reproduces the
// bug where a handler's *sidecar.Error carrying Data that is not valid JSON
// made outbox.send's json.Marshal fail; session.send only handled the
// over-limit case, so the answer was silently dropped and srelens waited
// until its timeout.
func TestAnErrorWithInvalidJSONDataDropsTheDataAndStillAnswers(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "baddata", func(context.Context, struct{}) (struct{}, error) {
		return struct{}{}, &sidecar.Error{Code: -32050, Message: "quota", Data: json.RawMessage("not json")}
	})
	h := start(t, s)
	h.initialize()
	id := h.request("baddata", map[string]any{})
	e := errorOf(t, h.answer(id))
	if e.code != -32050 || e.message != "quota" || e.data != nil {
		t.Fatalf("%+v", e)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

// panickyError's Error method panics: a handler could return one instead of
// a well-behaved error.
type panickyError struct{}

func (panickyError) Error() string { panic("boom from Error()") }

// TestAHandlersErrorThatPanicsWhenReadIsAnsweredInternalErrorAndTheSidecarGoesOn
// reproduces the bug where asRPCError, called outside guarded, propagated a
// panic from a handler's own error type and crashed the sidecar.
func TestAHandlersErrorThatPanicsWhenReadIsAnsweredInternalErrorAndTheSidecarGoesOn(t *testing.T) {
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "panicky", func(context.Context, struct{}) (struct{}, error) {
		return struct{}{}, panickyError{}
	})
	h := start(t, s)
	h.initialize()
	id := h.request("panicky", map[string]any{})
	if e := errorOf(t, h.answer(id)); e.code != -32603 {
		t.Fatalf("%+v", e)
	}
	id = h.request("health", map[string]any{})
	h.answer(id)
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestANameThatIsReservedMalformedOrTakenCannotBeRegistered(t *testing.T) {
	noop := func(context.Context, struct{}) (struct{}, error) { return struct{}{}, nil }
	for _, name := range []string{"health", "initialize", "a_b", "", "stream/x"} {
		mustPanic(t, "registering "+name, func() { sidecar.Operation(sidecar.New("t", "1"), name, noop) })
	}
	s := sidecar.New("t", "1")
	sidecar.Operation(s, "greet", noop)
	mustPanic(t, "registering greet twice", func() { sidecar.Operation(s, "greet", noop) })
}
