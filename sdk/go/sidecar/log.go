package sidecar

import (
	"bytes"
	"context"
	"io"
	"log/slog"
	"strconv"
	"strings"
	"sync"
	"unicode/utf8"
)

// A sidecar's log is its stderr, one line per record, which srelens keeps in
// the app's log at the level the line starts with
// (docs/extensions/sidecar-protocol.md, "Logs"). A line longer than 4 KiB
// would be dropped whole, so each is cut to fit.

// logLineBytes is the longest line srelens keeps.
const logLineBytes = 4096

// logHandler is a slog.Handler writing "LEVEL target: message key=value …".
// target is the sidecar's name, unless a top-level "target" attribute names
// another.
type logHandler struct {
	mu     *sync.Mutex
	w      io.Writer
	level  slog.Leveler
	target string
	pairs  []string // from WithAttrs, formatted
	group  string   // "a.b." from WithGroup
}

func newLogHandler(w io.Writer, level slog.Leveler, target string) *logHandler {
	return &logHandler{mu: &sync.Mutex{}, w: w, level: level, target: target}
}

func (h *logHandler) Enabled(_ context.Context, level slog.Level) bool {
	return level >= h.level.Level()
}

func (h *logHandler) Handle(_ context.Context, r slog.Record) error {
	target := h.target
	pairs := append([]string(nil), h.pairs...)
	r.Attrs(func(a slog.Attr) bool {
		if h.group == "" && a.Key == "target" {
			target = a.Value.Resolve().String()
			return true
		}
		pairs = appendAttr(pairs, h.group, a)
		return true
	})
	text := r.Message
	if len(pairs) > 0 {
		text += " " + strings.Join(pairs, " ")
	}
	var buf bytes.Buffer
	for _, line := range logLines(r.Level, target, text) {
		buf.WriteString(line)
		buf.WriteByte('\n')
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	_, err := h.w.Write(buf.Bytes())
	return err
}

func (h *logHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	next := *h
	next.pairs = append([]string(nil), h.pairs...)
	for _, a := range attrs {
		if h.group == "" && a.Key == "target" {
			next.target = a.Value.Resolve().String()
			continue
		}
		next.pairs = appendAttr(next.pairs, h.group, a)
	}
	return &next
}

func (h *logHandler) WithGroup(name string) slog.Handler {
	if name == "" {
		return h
	}
	next := *h
	next.group = h.group + name + "."
	return &next
}

// appendAttr formats a as key=value (a group's attributes as group.key=value).
func appendAttr(pairs []string, prefix string, a slog.Attr) []string {
	v := a.Value.Resolve()
	if v.Kind() == slog.KindGroup {
		for _, inner := range v.Group() {
			pairs = appendAttr(pairs, prefix+a.Key+".", inner)
		}
		return pairs
	}
	if a.Key == "" {
		return pairs
	}
	value := v.String()
	if value == "" || strings.ContainsAny(value, " =\"\n\t") {
		value = strconv.Quote(value)
	}
	return append(pairs, prefix+a.Key+"="+value)
}

// logLines are the stderr lines for one record: "LEVEL target: text", one
// per line of text, each cut to logLineBytes on a UTF-8 boundary.
func logLines(level slog.Level, target, text string) []string {
	text = strings.TrimSuffix(strings.ReplaceAll(text, "\r\n", "\n"), "\n")
	parts := strings.Split(text, "\n")
	lines := make([]string, len(parts))
	for i, part := range parts {
		lines[i] = cutLine(levelName(level) + " " + target + ": " + part)
	}
	return lines
}

func levelName(level slog.Level) string {
	switch {
	case level < slog.LevelInfo:
		return "DEBUG"
	case level < slog.LevelWarn:
		return "INFO"
	case level < slog.LevelError:
		return "WARN"
	}
	return "ERROR"
}

func cutLine(line string) string {
	if len(line) <= logLineBytes {
		return line
	}
	end := logLineBytes
	for end > 0 && !utf8.RuneStart(line[end]) {
		end--
	}
	return line[:end]
}
