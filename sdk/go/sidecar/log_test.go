package sidecar

import (
	"bytes"
	"log/slog"
	"strings"
	"testing"
	"unicode/utf8"
)

func logged(level slog.Leveler, write func(*slog.Logger)) string {
	var buf bytes.Buffer
	write(slog.New(newLogHandler(&buf, level, "scanner")))
	return buf.String()
}

func TestARecordIsALineSrelensReadsAtItsLevel(t *testing.T) {
	got := logged(slog.LevelDebug, func(l *slog.Logger) {
		l.Debug("d")
		l.Info("3 images queued")
		l.Warn("slow")
		l.Error("failed")
	})
	want := "DEBUG scanner: d\nINFO scanner: 3 images queued\nWARN scanner: slow\nERROR scanner: failed\n"
	if got != want {
		t.Fatalf("%q, want %q", got, want)
	}
}

func TestARecordBelowTheLevelIsNotWritten(t *testing.T) {
	if got := logged(slog.LevelInfo, func(l *slog.Logger) { l.Debug("quiet") }); got != "" {
		t.Fatalf("%q", got)
	}
}

func TestARecordOfSeveralLinesIsWrittenOneLineAtATimeEachWithItsLevel(t *testing.T) {
	if got := logged(slog.LevelInfo, func(l *slog.Logger) { l.Error("first\nsecond") }); got != "ERROR scanner: first\nERROR scanner: second\n" {
		t.Fatalf("%q", got)
	}
	if got := logged(slog.LevelInfo, func(l *slog.Logger) { l.Info("") }); got != "INFO scanner: \n" {
		t.Fatalf("%q", got)
	}
}

func TestAttributesFollowTheMessageAndTargetNamesTheTarget(t *testing.T) {
	got := logged(slog.LevelInfo, func(l *slog.Logger) {
		l.Info("scan", "images", 3, "target", "scanner/db", "note", "two words")
		l.With("target", "cache").Info("hit")
		l.WithGroup("db").Info("query", "rows", 2)
	})
	want := "INFO scanner/db: scan images=3 note=\"two words\"\nINFO cache: hit\nINFO scanner: query db.rows=2\n"
	if got != want {
		t.Fatalf("%q, want %q", got, want)
	}
}

// A group with an empty key has its attributes inlined, as slog's own
// handlers do: no stray leading dot.
func TestAGroupWithAnEmptyKeyIsInlined(t *testing.T) {
	got := logged(slog.LevelInfo, func(l *slog.Logger) {
		l.Info("scan", slog.Group("", slog.Int("x", 1)))
		l.WithGroup("db").Info("query", slog.Group("", slog.Int("rows", 2)))
		l.With(slog.Group("", slog.String("zone", "a"))).Info("hit")
	})
	want := "INFO scanner: scan x=1\nINFO scanner: query db.rows=2\nINFO scanner: hit zone=a\n"
	if got != want {
		t.Fatalf("%q, want %q", got, want)
	}
}

// After the 8 bytes of "INFO s: ", 2-byte runes put the cut at 4 KiB on a
// rune's start; 3-byte runes put it inside one, and the cut must back off to
// that rune's start.
func TestALineIsCutToWhatSrelensKeepsOnACharacterBoundary(t *testing.T) {
	for _, r := range []string{"é", "€"} {
		lines := logLines(slog.LevelInfo, "s", strings.Repeat(r, 3000))
		if len(lines) != 1 || len(lines[0]) > logLineBytes || len(lines[0]) <= logLineBytes-4 || !utf8.ValidString(lines[0]) {
			t.Fatalf("%s: %d lines, the first %d bytes, valid UTF-8 %v", r, len(lines), len(lines[0]), utf8.ValidString(lines[0]))
		}
	}
}
