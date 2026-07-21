import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Check, Copy, FolderOpen, RefreshCw } from "lucide-react";
import { appLogPath, readAppLog, revealAppLog } from "../lib/appLog";
import { Button, IconButton, Select, Spinner, TextInput } from "../ui";

/** Log levels emitted by tauri-plugin-log, most→least severe. */
const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;
type Level = (typeof LEVELS)[number];

const LEVEL_CLASS: Record<Level, string> = {
  ERROR: "text-red-600 dark:text-red-400",
  WARN: "text-amber-600 dark:text-amber-400",
  INFO: "text-foreground/80",
  DEBUG: "text-foreground/60",
  TRACE: "text-foreground/50",
};

/** Cap rendered lines so a large log can't balloon the DOM. */
const MAX_RENDERED = 5000;

/** The level of a `[date][time][target][LEVEL] message` line (INFO if absent). */
export function logLineLevel(line: string): Level {
  const match = line.match(/\]\[(TRACE|DEBUG|INFO|WARN|ERROR)\]/);
  return (match?.[1] as Level) ?? "INFO";
}

/**
 * View srelens's own application log: read the tail of the rotating log file,
 * filter by level and text, refresh, copy its path, or reveal it in the file
 * manager. For diagnosing issues (connection failures, RBAC denials, …) after
 * they happen.
 */
export function AppLogView() {
  const [raw, setRaw] = useState("");
  const [path, setPath] = useState("");
  const [level, setLevel] = useState<string>("all");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    setError("");
    Promise.all([readAppLog(), appLogPath()])
      .then(([text, logPath]) => {
        setRaw(text);
        setPath(logPath);
      })
      .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => load(), [load]);

  const lines = useMemo(() => (raw ? raw.split("\n").filter(Boolean) : []), [raw]);
  const filtered = useMemo(() => {
    const query = search.toLowerCase();
    const matches = lines.filter((line) => {
      if (level !== "all" && logLineLevel(line) !== level) return false;
      if (query && !line.toLowerCase().includes(query)) return false;
      return true;
    });
    // Keep the most recent when a large log exceeds the render cap.
    return matches.length > MAX_RENDERED ? matches.slice(matches.length - MAX_RENDERED) : matches;
  }, [lines, level, search]);

  async function copyPath() {
    try {
      await navigator.clipboard.writeText(path);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard may be unavailable; the path is shown for manual copy.
    }
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={level}
          onValueChange={setLevel}
          options={[{ value: "all", label: "All levels" }, ...LEVELS.map((l) => ({ value: l, label: l }))]}
          aria-label="Log level"
        />
        <div className="relative w-48">
          <TextInput value={search} onValueChange={setSearch} placeholder="Search log…" aria-label="Search log" />
        </div>
        {(search || level !== "all") && (
          <span className="tabular-nums text-xs text-muted-foreground">
            {filtered.length}/{lines.length}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1">
          {loading && <Spinner label="Loading log" />}
          <IconButton icon={RefreshCw} label="Refresh" onClick={load} disabled={loading} />
          <Button variant="outline" size="sm" onClick={() => void revealAppLog()}>
            <FolderOpen aria-hidden="true" className="size-3.5" />
            Reveal
          </Button>
        </div>
      </div>

      {path && (
        <button
          type="button"
          onClick={() => void copyPath()}
          title="Copy log file path"
          className="flex items-center gap-1.5 self-start rounded font-mono text-[11px] text-muted-foreground hover:text-foreground"
        >
          {copied ? <Check aria-hidden="true" className="size-3 text-emerald-600 dark:text-emerald-400" /> : <Copy aria-hidden="true" className="size-3" />}
          <span className="max-w-[52ch] truncate">{path}</span>
        </button>
      )}

      <div
        role="log"
        aria-label="Application log"
        className="min-h-[16rem] flex-1 overflow-auto rounded border border-border bg-card p-2 font-mono text-xs leading-relaxed"
      >
        {error ? (
          <div className="p-2 text-red-600 dark:text-red-400">Error: {error}</div>
        ) : filtered.length > 0 ? (
          filtered.map((line, i) => (
            <div key={i} className={LEVEL_CLASS[logLineLevel(line)]}>
              {line}
            </div>
          ))
        ) : (
          <div className="p-2 text-muted-foreground">
            {loading ? "Loading…" : lines.length > 0 ? "No matching lines" : "No log entries yet."}
          </div>
        )}
      </div>
    </div>
  );
}
