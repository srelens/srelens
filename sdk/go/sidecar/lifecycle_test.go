package sidecar_test

import (
	"context"
	"errors"
	"io"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

// The harness's schema check is live: it refuses lines a sidecar may not write.
func TestTheSchemaCheckRefusesWhatASidecarMayNotWrite(t *testing.T) {
	if !schemaAllows(t, `{"jsonrpc":"2.0","id":1,"result":{}}`) {
		t.Fatal("the schema refuses a plain answer")
	}
	for _, bad := range []string{
		`{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1,"message":"x"}}`,
		`{"jsonrpc":"1.0","id":1,"result":{}}`,
		`{"jsonrpc":"2.0","method":"host/read","params":{}}`,
		`{"jsonrpc":"2.0","id":"c-1","method":"host/read","params":{"capability":"pods","context":{"clusterId":"\u00a0","namespace":null}}}`,
	} {
		if schemaAllows(t, bad) {
			t.Errorf("the schema allows %s", bad)
		}
	}
}

func TestInitializeNamesTheVersionAndTheSidecar(t *testing.T) {
	h := start(t, sidecar.New("scanner", "1.2.0"))
	result := h.initialize()
	want := map[string]any{"apiVersion": "0.1.0", "sidecar": map[string]any{"name": "scanner", "version": "1.2.0"}}
	if !reflect.DeepEqual(result, want) {
		t.Fatalf("initialize answered %v, want %v", result, want)
	}
	if err := h.finish(); err != nil {
		t.Fatalf("the session ended with %v", err)
	}
}

func TestWithNoVersionInCommonInitializeSaysWhichItSpeaks(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	id := h.request("initialize", initializeParams([]string{"9.9.9"}, h.DataDir, defaultLimits()))
	e := errorOf(t, h.answer(id))
	if e.code != -32001 || !strings.Contains(e.message, "9.9.9") || !strings.Contains(e.message, "0.1.0") {
		t.Fatalf("%+v", e)
	}
	if !reflect.DeepEqual(e.data, map[string]any{"supported": []any{"0.1.0", "0.2.0"}}) {
		t.Fatalf("data %v", e.data)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestNothingIsServedBeforeInitializeNorInitializedTwice(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	id := h.request("health", map[string]any{})
	if e := errorOf(t, h.answer(id)); e.code != -32600 || !strings.Contains(e.message, "not initialized") {
		t.Fatalf("%+v", e)
	}
	h.initialize()
	id = h.request("initialize", initializeParams([]string{"0.1.0"}, h.DataDir, defaultLimits()))
	if e := errorOf(t, h.answer(id)); e.code != -32600 || !strings.Contains(e.message, "already initialized") {
		t.Fatalf("%+v", e)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestHealthAndDeactivateAreAnsweredWithNothing(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	h.initialize()
	for _, method := range []string{"health", "deactivate"} {
		id := h.request(method, map[string]any{})
		if r := h.answer(id)["result"]; !reflect.DeepEqual(r, map[string]any{}) {
			t.Fatalf("%s answered %v", method, r)
		}
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestAnUnknownMethodIsNotFound(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	h.initialize()
	id := h.request("nope", map[string]any{})
	if e := errorOf(t, h.answer(id)); e.code != -32601 || !strings.Contains(e.message, "`nope`") {
		t.Fatalf("%+v", e)
	}
	if err := h.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestShutdownIsAnsweredAndThenTheSessionEnds(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	h.initialize()
	id := h.request("shutdown", map[string]any{})
	if r := h.answer(id)["result"]; !reflect.DeepEqual(r, map[string]any{}) {
		t.Fatalf("shutdown answered %v", r)
	}
	if err := h.ended(); err != nil {
		t.Fatalf("the session ended with %v", err)
	}
}

func TestTheSessionEndsWhenItsInputDoesAndBlankLinesAreSkipped(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	h.sendRaw("")
	h.sendRaw("   ")
	h.initialize()
	if err := h.finish(); err != nil {
		t.Fatalf("the session ended with %v", err)
	}
}

func TestALineThatIsNotJSONRPCEndsTheSessionWithAnError(t *testing.T) {
	h := start(t, sidecar.New("t", "1"))
	h.initialize()
	h.sendRaw("not json")
	if err := h.ended(); !errors.Is(err, sidecar.ErrProtocol) {
		t.Fatalf("the session ended with %v, want ErrProtocol", err)
	}
}

// failingWriter fails every Write, and records whether it was Closed, so
// tests can tell Run closed it even though the write failed.
type failingWriter struct {
	closed atomic.Bool
}

func (*failingWriter) Write([]byte) (int, error) { return 0, errors.New("the pipe broke") }

func (fw *failingWriter) Close() error {
	fw.closed.Store(true)
	return nil
}

func TestASidecarWhoseOutputFailsEndsWithAnIOError(t *testing.T) {
	inR, inW := io.Pipe()
	defer inW.Close()
	fw := &failingWriter{}
	done := make(chan error, 1)
	go func() { done <- sidecar.New("t", "1").Run(context.Background(), inR, fw) }()
	line := mustJSON(t, map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize",
		"params": initializeParams([]string{"0.1.0"}, t.TempDir(), defaultLimits())})
	go io.WriteString(inW, line+"\n")
	select {
	case err := <-done:
		if err == nil || !strings.Contains(err.Error(), "the pipe broke") {
			t.Fatalf("the session ended with %v", err)
		}
	case <-time.After(wait):
		t.Fatal("the session did not end")
	}
	if !fw.closed.Load() {
		t.Fatal("Run did not close w after the write to it failed")
	}
}

func TestRunEndsWhenItsContextIsDone(t *testing.T) {
	inR, inW := io.Pipe()
	defer inW.Close()
	_, outW := io.Pipe()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- sidecar.New("t", "1").Run(ctx, inR, outW) }()
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("Run returned %v", err)
		}
	case <-time.After(wait):
		t.Fatal("Run did not return")
	}
}
