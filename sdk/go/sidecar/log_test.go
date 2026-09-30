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

func TestALineIsCutToWhatSrelensKeepsOnACharacterBoundary(t *testing.T) {
	lines := logLines(slog.LevelInfo, "s", strings.Repeat("é", 3000))
	if len(lines) != 1 || len(lines[0]) > logLineBytes || len(lines[0]) <= logLineBytes-4 || !utf8.ValidString(lines[0]) {
		t.Fatalf("%d lines, the first %d bytes", len(lines), len(lines[0]))
	}
}
