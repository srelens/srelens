package sidecar_test

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/srelens/srelens/sdk/go/sidecar"
)

// wait is how long a test waits for the sidecar to write: far above what the
// SDK should need, so a wait this long is a failure.
const wait = 5 * time.Second

var (
	schemaOnce     sync.Once
	sidecarMessage *jsonschema.Schema
	schemaErr      error
)

// sidecarMessageSchema is the committed schema's SidecarMessage: every line a
// sidecar may write.
func sidecarMessageSchema(t testing.TB) *jsonschema.Schema {
	t.Helper()
	schemaOnce.Do(func() {
		f, err := os.Open(filepath.Join("..", "..", "..", "schemas", "sidecar-protocol.v0.1.json"))
		if err != nil {
			schemaErr = err
			return
		}
		defer f.Close()
		doc, err := jsonschema.UnmarshalJSON(f)
		if err != nil {
			schemaErr = err
			return
		}
		id, _ := doc.(map[string]any)["$id"].(string)
		c := jsonschema.NewCompiler()
		if schemaErr = c.AddResource(id, doc); schemaErr != nil {
			return
		}
		sidecarMessage, schemaErr = c.Compile(id + "#/definitions/SidecarMessage")
	})
	if schemaErr != nil {
		t.Fatalf("the protocol schema: %v", schemaErr)
	}
	return sidecarMessage
}

// schemaAllows reports whether the schema lets a sidecar write line.
func schemaAllows(t testing.TB, line string) bool {
	t.Helper()
	inst, err := jsonschema.UnmarshalJSON(strings.NewReader(line))
	if err != nil {
		return false
	}
	return sidecarMessageSchema(t).Validate(inst) == nil
}

// checkLine fails t unless line is one JSON object the schema lets a sidecar
// write; the object.
func checkLine(t testing.TB, line string) map[string]any {
	t.Helper()
	inst, err := jsonschema.UnmarshalJSON(strings.NewReader(line))
	if err != nil {
		t.Fatalf("the sidecar wrote %q, which is not JSON: %v", line, err)
	}
	if err := sidecarMessageSchema(t).Validate(inst); err != nil {
		t.Fatalf("the sidecar wrote %s, which the schema refuses: %v", line, err)
	}
	var m map[string]any
	if err := json.Unmarshal([]byte(line), &m); err != nil {
		t.Fatalf("the sidecar wrote %s, which is not an object: %v", line, err)
	}
	return m
}

func mustJSON(t testing.TB, v any) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatalf("marshal %v: %v", v, err)
	}
	return string(b)
}

// fakeHost drives Sidecar.Run over in-memory pipes, as srelens's supervisor
// would, and holds every line the sidecar writes to the committed schema --
// including anything written after the test's last read, which finish and
// ended drain and check.
type fakeHost struct {
	t       *testing.T
	in      *io.PipeWriter // the sidecar's stdin
	lines   chan string    // what the sidecar wrote, line by line; closed at its end
	done    chan error     // Run's result
	nextID  uint64
	DataDir string
}

func start(t *testing.T, s *sidecar.Sidecar) *fakeHost {
	t.Helper()
	inR, inW := io.Pipe()
	outR, outW := io.Pipe()
	h := &fakeHost{t: t, in: inW, lines: make(chan string, 1024), done: make(chan error, 1), DataDir: t.TempDir()}
	go func() { h.done <- s.Run(context.Background(), inR, outW) }()
	go func() {
		br := bufio.NewReader(outR)
		for {
			line, err := br.ReadString('\n')
			if line != "" {
				h.lines <- strings.TrimSuffix(line, "\n")
			}
			if err != nil {
				close(h.lines)
				return
			}
		}
	}()
	t.Cleanup(func() {
		inW.Close()
		outR.Close()
	})
	return h
}

func (h *fakeHost) sendRaw(line string) {
	h.t.Helper()
	if _, err := io.WriteString(h.in, line+"\n"); err != nil {
		h.t.Fatalf("writing to the sidecar: %v", err)
	}
}

// request sends a request; its id.
func (h *fakeHost) request(method string, params any) uint64 {
	h.t.Helper()
	h.nextID++
	h.sendRaw(mustJSON(h.t, map[string]any{"jsonrpc": "2.0", "id": h.nextID, "method": method, "params": params}))
	return h.nextID
}

func (h *fakeHost) notify(method string, params any) {
	h.t.Helper()
	h.sendRaw(mustJSON(h.t, map[string]any{"jsonrpc": "2.0", "method": method, "params": params}))
}

// recv is the next line the sidecar wrote, which must be a valid SidecarMessage.
func (h *fakeHost) recv() map[string]any {
	h.t.Helper()
	select {
	case line, ok := <-h.lines:
		if !ok {
			h.t.Fatal("the sidecar's stdout ended")
		}
		return checkLine(h.t, line)
	case <-time.After(wait):
		h.t.Fatal("the sidecar wrote nothing in time")
	}
	return nil
}

// nextWithin is the next line if the sidecar writes one within d.
func (h *fakeHost) nextWithin(d time.Duration) (map[string]any, bool) {
	h.t.Helper()
	select {
	case line, ok := <-h.lines:
		if !ok {
			return nil, false
		}
		return checkLine(h.t, line), true
	case <-time.After(d):
		return nil, false
	}
}

// answer is the next line, which must answer request id.
func (h *fakeHost) answer(id uint64) map[string]any {
	h.t.Helper()
	m := h.recv()
	if got, _ := m["id"].(float64); got != float64(id) || m["method"] != nil {
		h.t.Fatalf("expected the answer to %d, got %v", id, m)
	}
	return m
}

// call is the next line, which must be a host/* call.
func (h *fakeHost) call() map[string]any {
	h.t.Helper()
	m := h.recv()
	method, _ := m["method"].(string)
	if !strings.HasPrefix(method, "host/") || m["id"] == nil {
		h.t.Fatalf("expected a host/* call, got %v", m)
	}
	return m
}

// reply answers the sidecar's call id (its own id, as it sent it) with
// result, or with rpcErr when that is not nil.
func (h *fakeHost) reply(id any, result any, rpcErr map[string]any) {
	h.t.Helper()
	line := map[string]any{"jsonrpc": "2.0", "id": id}
	if rpcErr != nil {
		line["error"] = rpcErr
	} else {
		line["result"] = result
	}
	h.sendRaw(mustJSON(h.t, line))
}

// initialize initializes and activates the sidecar, as the supervisor does,
// under the default limits; initialize's result.
func (h *fakeHost) initialize() map[string]any {
	h.t.Helper()
	return h.initializeWith(defaultLimits())
}

func (h *fakeHost) initializeWith(limits map[string]any) map[string]any {
	h.t.Helper()
	id := h.request("initialize", initializeParams([]string{"0.1.0"}, h.DataDir, limits))
	result, ok := h.answer(id)["result"].(map[string]any)
	if !ok {
		h.t.Fatal("initialize was refused")
	}
	activate := h.request("activate", map[string]any{})
	if r := h.answer(activate)["result"]; !reflect.DeepEqual(r, map[string]any{}) {
		h.t.Fatalf("activate answered %v", r)
	}
	return result
}

// finish closes the sidecar's stdin, waits for the session to end, and fails
// on any line the sidecar wrote that no test read; Run's result.
func (h *fakeHost) finish() error {
	h.t.Helper()
	h.in.Close()
	return h.ended()
}

// ended waits for the session to end without closing stdin (after
// shutdown), then fails on any line no test read; Run's result.
func (h *fakeHost) ended() error {
	h.t.Helper()
	var err error
	select {
	case err = <-h.done:
	case <-time.After(wait):
		h.t.Fatal("the session did not end in time")
	}
	select {
	case line, ok := <-h.lines:
		if ok {
			checkLine(h.t, line)
			h.t.Fatalf("the sidecar wrote %s, which the test never read", line)
		}
	case <-time.After(wait):
		h.t.Fatal("the sidecar's stdout did not end in time")
	}
	return err
}

func defaultLimits() map[string]any {
	return map[string]any{
		"requestTimeoutMs": 30000, "maxConcurrentRequests": 8, "maxStreams": 5,
		"memoryBytes": 268435456, "cpus": 1.0, "dataBytes": 1073741824, "dataEntries": 100000,
	}
}

func initializeParams(offered []string, dataDir string, limits map[string]any) map[string]any {
	return map[string]any{
		"apiVersions":   offered,
		"host":          map[string]any{"name": "srelens", "version": "0.15.0"},
		"limits":        limits,
		"dataDirectory": dataDir,
	}
}

type rpcError struct {
	code    float64
	message string
	data    any
}

// errorOf is the error answer m carries; it fails t if m is not one.
func errorOf(t testing.TB, m map[string]any) rpcError {
	t.Helper()
	e, ok := m["error"].(map[string]any)
	if !ok {
		t.Fatalf("expected an error answer, got %v", m)
	}
	code, _ := e["code"].(float64)
	message, _ := e["message"].(string)
	return rpcError{code: code, message: message, data: e["data"]}
}
