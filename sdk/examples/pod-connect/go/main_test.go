package main

import (
	"bufio"
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"slices"
	"strings"
	"testing"
	"time"
)

// fakeHost plays srelens: it runs the sidecar over pipes, as the supervisor
// does, and speaks the sidecar protocol's lines by hand.
type fakeHost struct {
	t      *testing.T
	in     *io.PipeWriter
	lines  chan map[string]any
	nextID int
	// skipped holds the methods of the lines until passed over.
	skipped []string
}

func start(t *testing.T) *fakeHost {
	t.Helper()
	inR, inW := io.Pipe()
	outR, outW := io.Pipe()
	h := &fakeHost{t: t, in: inW, lines: make(chan map[string]any, 64)}
	go func() { _ = newSidecar().Run(context.Background(), inR, outW) }()
	go func() {
		sc := bufio.NewScanner(outR)
		sc.Buffer(nil, 1<<20)
		for sc.Scan() {
			var m map[string]any
			if err := json.Unmarshal(sc.Bytes(), &m); err != nil {
				t.Errorf("the sidecar wrote %q: %v", sc.Text(), err)
			}
			h.lines <- m
		}
		close(h.lines)
	}()
	t.Cleanup(func() { inW.Close(); outR.Close() })
	h.answer(h.request("initialize", map[string]any{
		"apiVersions": []string{"0.3.0"},
		"host":        map[string]any{"name": "srelens", "version": "0.15.0"},
		"limits": map[string]any{
			"requestTimeoutMs": 30000, "maxConcurrentRequests": 8, "maxStreams": 5,
			"memoryBytes": 268435456, "cpus": 1.0, "dataBytes": 1073741824, "dataEntries": 100000,
		},
		"dataDirectory": t.TempDir(),
	}))
	h.answer(h.request("activate", map[string]any{}))
	return h
}

func (h *fakeHost) send(line map[string]any) {
	h.t.Helper()
	line["jsonrpc"] = "2.0"
	b, _ := json.Marshal(line)
	if _, err := h.in.Write(append(b, '\n')); err != nil {
		h.t.Fatal(err)
	}
}

func (h *fakeHost) request(method string, params any) int {
	h.nextID++
	h.send(map[string]any{"id": h.nextID, "method": method, "params": params})
	return h.nextID
}

// next is the next line the sidecar wrote, of any kind.
func (h *fakeHost) next() map[string]any {
	h.t.Helper()
	select {
	case m, ok := <-h.lines:
		if !ok {
			h.t.Fatal("the sidecar's stdout ended")
		}
		return m
	case <-time.After(5 * time.Second):
		h.t.Fatal("the sidecar wrote nothing in time")
	}
	return nil
}

// until is the next line for which want is true; the lines before it
// (credit, close) are not what the test is about.
func (h *fakeHost) until(want func(map[string]any) bool) map[string]any {
	h.t.Helper()
	for {
		m := h.next()
		if want(m) {
			return m
		}
		if method, ok := m["method"].(string); ok {
			h.skipped = append(h.skipped, method)
		}
	}
}

func (h *fakeHost) answer(id int) map[string]any {
	return h.until(func(m map[string]any) bool { return m["method"] == nil && m["id"] == float64(id) })
}

// connect is the host/connect the sidecar sends.
func (h *fakeHost) connect() map[string]any {
	return h.until(func(m map[string]any) bool { return m["method"] == "host/connect" })
}

var fetchParams = map[string]any{"cluster": "kind-dev", "namespace": "team", "object": "web", "pod": "web-0"}

func TestFetchReturnsTheFirstLineThePodSends(t *testing.T) {
	h := start(t)
	id := h.request("fetch", fetchParams)

	call := h.connect()
	params, _ := call["params"].(map[string]any)
	if params["capability"] != "connect" || params["name"] != "web" || params["pod"] != "web-0" {
		t.Fatalf("host/connect carried %v", params)
	}
	h.send(map[string]any{"id": call["id"], "result": map[string]any{"pod": "web-0"}})

	sent := h.until(func(m map[string]any) bool { return m["method"] == "tunnel/data" })
	got, _ := base64.StdEncoding.DecodeString(sent["params"].(map[string]any)["data"].(string))
	if string(got) != "GET / HTTP/1.0\r\n\r\n" {
		t.Fatalf("the sidecar sent %q", got)
	}
	h.send(map[string]any{"method": "tunnel/data", "params": map[string]any{
		"tunnel": params["tunnel"], "data": base64.StdEncoding.EncodeToString([]byte("HTTP/1.0 200 OK\r\nServer: t\r\n\r\nbody")),
	}})

	res, _ := h.answer(id)["result"].(map[string]any)
	if res["line"] != "HTTP/1.0 200 OK" {
		t.Fatalf("fetch answered %v", res)
	}
	// The conn was closed before the answer was written.
	if !slices.Contains(h.skipped, "tunnel/close") {
		t.Fatalf("the sidecar never closed the tunnel; it wrote %v", h.skipped)
	}
}

func TestFetchReportsTheHostsRefusal(t *testing.T) {
	h := start(t)
	id := h.request("fetch", fetchParams)
	h.send(map[string]any{"id": h.connect()["id"], "error": map[string]any{"code": -32003, "message": "web-0 is not in scope"}})

	e, _ := h.answer(id)["error"].(map[string]any)
	if msg, _ := e["message"].(string); !strings.Contains(msg, "web-0 is not in scope") {
		t.Fatalf("fetch answered %v", e)
	}
}

func TestFetchReportsWhyTheHostClosedTheTunnel(t *testing.T) {
	h := start(t)
	id := h.request("fetch", fetchParams)
	call := h.connect()
	h.send(map[string]any{"id": call["id"], "result": map[string]any{"pod": "web-0"}})
	h.send(map[string]any{"method": "tunnel/close", "params": map[string]any{
		"tunnel": call["params"].(map[string]any)["tunnel"], "message": "pod web-0 left the scope",
	}})

	e, _ := h.answer(id)["error"].(map[string]any)
	if msg, _ := e["message"].(string); !strings.Contains(msg, "pod web-0 left the scope") {
		t.Fatalf("fetch answered %v", e)
	}
}
