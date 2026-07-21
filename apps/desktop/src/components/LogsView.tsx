import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Clock, Download, DownloadCloud, History, Pause, Play, RefreshCw, WrapText } from "lucide-react";
import { podLogs, podsForSelector } from "../lib/workloads";
import { getObject } from "../lib/manifest";
import { startLogStream, type LogStream, type LogTarget, type LogStatus } from "../lib/logsStream";
import { saveTextFile } from "../lib/files";
import { Spinner, Select, IconButton, TextInput, avatarColor } from "../ui";
import { computeLogWindow } from "./logWindow";

/** What a logs view is following: a single pod, or every pod of a workload. */
export type LogsSource =
  | { type: "pod"; pod: string }
  | { type: "workload"; kind: string; name: string };

/** Sentinel option value meaning "all pods" / "all containers" / "all time". */
const ALL = "__all__";
/** Cap the live-tail buffer so a chatty stream can't grow without bound. */
const MAX_LINES = 5000;

/** How many trailing lines to fetch/tail. */
const TAIL_OPTIONS = [100, 200, 500, 1000, 5000];
/** `sinceSeconds` windows; `ALL` omits the bound entirely. */
const SINCE_OPTIONS: { value: string; label: string; seconds?: number }[] = [
  { value: ALL, label: "All time" },
  { value: "300", label: "Last 5m", seconds: 300 },
  { value: "900", label: "Last 15m", seconds: 900 },
  { value: "3600", label: "Last 1h", seconds: 3600 },
  { value: "21600", label: "Last 6h", seconds: 21600 },
];

/** One buffered log line, keeping its source tag separate so it can be coloured. */
interface LogEntry {
  /** Source tag ("pod/container"); empty for a single target. */
  source: string;
  line: string;
}

/** Classify a klog-style line (I/W/E0629 …) for colourising. */
function lineLevel(line: string): "error" | "warn" | "info" | "plain" {
  if (/^E\d{4}\b/.test(line) || /\b(error|fatal|panic)\b/i.test(line)) return "error";
  if (/^W\d{4}\b/.test(line) || /\bwarn(ing)?\b/i.test(line)) return "warn";
  if (/^I\d{4}\b/.test(line)) return "info";
  return "plain";
}

const LEVEL_CLASS: Record<string, string> = {
  error: "text-red-600 dark:text-red-400",
  warn: "text-amber-600 dark:text-amber-400",
  info: "text-foreground/90",
  plain: "text-foreground/80",
};

/** A compact toolbar toggle with a pressed state (previous/timestamps modes). */
function ToggleButton({
  icon: Icon,
  label,
  text,
  active,
  onClick,
  disabled,
  title,
}: {
  icon: React.ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  label: string;
  text: string;
  active: boolean;
  onClick: () => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={active}
      title={title ?? label}
      onClick={onClick}
      disabled={disabled}
      className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 transition-colors disabled:opacity-40 ${
        active
          ? "bg-primary/15 text-primary"
          : "text-muted-foreground hover:bg-accent hover:text-foreground"
      }`}
    >
      <Icon className="size-3.5" aria-hidden={true} />
      {text}
    </button>
  );
}

/**
 * Pod / workload logs in a themed, scrollable panel. Supports picking a single
 * pod or all pods of a workload, a single container or all containers, text
 * search, line-wrap, previous-instance (post-crash) logs, timestamps, tail and
 * since windows, per-source colourising, downloading the buffer or a full
 * all-containers dump, and a live-tail (follow) mode.
 */
export function LogsView({
  context,
  namespace,
  source,
  initialContainer,
}: {
  context: string;
  namespace: string;
  source: LogsSource;
  /** Preselect this container instead of "all" (from a per-container action). */
  initialContainer?: string;
}) {
  const srcType = source.type;
  const srcPod = source.type === "pod" ? source.pod : "";
  const srcKind = source.type === "workload" ? source.kind : "";
  const srcName = source.type === "workload" ? source.name : "";

  const [pods, setPods] = useState<string[]>(srcType === "pod" ? [srcPod] : []);
  const [containersByPod, setContainersByPod] = useState<Record<string, string[]>>({});
  const [pod, setPod] = useState<string>(srcType === "pod" ? srcPod : ALL);
  const [container, setContainer] = useState<string>(initialContainer || ALL);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [error, setError] = useState("");
  const [streamError, setStreamError] = useState("");
  const [loading, setLoading] = useState(false);
  const [wrap, setWrap] = useState(false);
  const [follow, setFollow] = useState(false);
  const [previous, setPrevious] = useState(false);
  const [timestamps, setTimestamps] = useState(false);
  const [tailLines, setTailLines] = useState(200);
  const [sinceValue, setSinceValue] = useState(ALL);
  const [streamStatus, setStreamStatus] = useState<LogStatus | "connecting">("connecting");
  const [search, setSearch] = useState("");
  const [saveError, setSaveError] = useState("");
  const [savingAll, setSavingAll] = useState(false);
  const entriesRef = useRef<LogEntry[]>([]);
  const streamRef = useRef<LogStream | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const rowsRef = useRef<HTMLDivElement | null>(null);
  const autoScrollRef = useRef(true);
  // Viewport/row metrics driving list virtualisation (see computeLogWindow).
  const [metrics, setMetrics] = useState({ scrollTop: 0, viewportHeight: 0, rowHeight: 0 });

  const sinceSeconds = useMemo(
    () => SINCE_OPTIONS.find((o) => o.value === sinceValue)?.seconds,
    [sinceValue],
  );

  // Discover the candidate pods. For a workload, resolve its selector → pods.
  useEffect(() => {
    let active = true;
    if (srcType === "pod") {
      setPods([srcPod]);
      setPod(srcPod);
      return;
    }
    void (async () => {
      const o = await getObject(context, srcKind, namespace, srcName);
      const spec = (o.object?.spec ?? {}) as { selector?: { matchLabels?: Record<string, string> } };
      const out = await podsForSelector(context, namespace, spec.selector?.matchLabels ?? {});
      if (!active) return;
      setPods((out.pods ?? []).map((p) => p.name));
      setPod(ALL);
    })();
    return () => {
      active = false;
    };
  }, [context, namespace, srcType, srcPod, srcKind, srcName]);

  // Discover containers for whichever pods are in scope.
  useEffect(() => {
    let active = true;
    const targets = pod === ALL ? pods : [pod];
    void (async () => {
      const entries = await Promise.all(
        targets
          .filter((p) => p && !containersByPod[p])
          .map(async (p) => {
            const o = await getObject(context, "Pod", namespace, p);
            const cs = ((o.object?.spec ?? {}) as { containers?: { name: string }[] }).containers ?? [];
            return [p, cs.map((c) => c.name)] as const;
          }),
      );
      if (!active || entries.length === 0) return;
      setContainersByPod((prev) => ({ ...prev, ...Object.fromEntries(entries) }));
    })();
    return () => {
      active = false;
    };
  }, [context, namespace, pod, pods, containersByPod]);

  // Union of container names across the in-scope pods.
  const containerOptions = useMemo(() => {
    const targets = pod === ALL ? pods : [pod];
    const set = new Set<string>();
    targets.forEach((p) => (containersByPod[p] ?? []).forEach((c) => set.add(c)));
    return [...set];
  }, [pod, pods, containersByPod]);

  // The concrete (pod, container) pairs in scope, with a source label when more
  // than one — shared by snapshot fetch and live-tail.
  const targetPods = useMemo(
    () => (pod === ALL ? pods : [pod]).filter(Boolean),
    [pod, pods],
  );
  const targetsReady =
    targetPods.length > 0 &&
    targetPods.every((targetPod) => Object.prototype.hasOwnProperty.call(containersByPod, targetPod));

  const targets = useMemo<LogTarget[]>(() => {
    const list: LogTarget[] = [];
    for (const p of targetPods) {
      const discovered = containersByPod[p];
      const cs = container === ALL
        ? discovered && discovered.length > 0 ? discovered : [undefined]
        : [container];
      const multi = targetPods.length > 1 || cs.length > 1;
      for (const c of cs) {
        list.push({ pod: p, container: c, label: multi ? `${p}${c ? `/${c}` : ""}` : "" });
      }
    }
    return list;
  }, [targetPods, container, containersByPod]);

  const setBuffer = (next: LogEntry[]) => {
    entriesRef.current = next;
    setEntries(next);
  };

  // Snapshot fetch (used when not following).
  const load = useCallback(() => {
    let active = true;
    setLoading(true);
    setError("");
    void (async () => {
      const collected: LogEntry[] = [];
      let firstError = "";
      for (const t of targets) {
        const out = await podLogs(context, namespace, t.pod, undefined, {
          container: t.container,
          previous,
          timestamps,
          tailLines,
          sinceSeconds,
        });
        if (out.error) {
          if (!firstError) firstError = out.error;
          continue;
        }
        const text = out.logs ?? "";
        if (!text) continue;
        text.split("\n").forEach((l) => collected.push({ source: t.label ?? "", line: l }));
      }
      if (!active) return;
      setBuffer(collected);
      setError(collected.length === 0 && firstError ? firstError : "");
      setLoading(false);
    })();
    return () => {
      active = false;
    };
  }, [context, namespace, targets, previous, timestamps, tailLines, sinceSeconds]);

  useEffect(() => {
    if (follow) return;
    return load();
  }, [follow, load]);

  // Live-tail: open a multiplexed stream and append lines as they arrive.
  useEffect(() => {
    if (!follow) return;
    if (!targetsReady || targets.length === 0) {
      setLoading(true);
      setStreamStatus("connecting");
      return;
    }
    let stopped = false;
    setBuffer([]);
    setError("");
    setStreamError("");
    setLoading(true);
    setStreamStatus("connecting");
    void startLogStream(
      context,
      namespace,
      targets,
      (sourceTag, line) => {
        const next = [...entriesRef.current, { source: sourceTag, line }];
        if (next.length > MAX_LINES) next.splice(0, next.length - MAX_LINES);
        setBuffer(next);
      },
      (status) => {
        if (!stopped) setStreamStatus(status);
      },
      { timestamps, sinceSeconds, tailLines },
    ).then((s) => {
      setLoading(false);
      if (stopped) s.stop();
      else streamRef.current = s;
    }).catch((cause: unknown) => {
      if (stopped) return;
      setLoading(false);
      setStreamError(cause instanceof Error ? cause.message : String(cause));
      setFollow(false);
    });
    return () => {
      stopped = true;
      streamRef.current?.stop();
      streamRef.current = null;
    };
  }, [follow, context, namespace, targets, targetsReady, timestamps, sinceSeconds, tailLines]);

  const visible = useMemo(() => {
    if (!search) return entries;
    const q = search.toLowerCase();
    return entries.filter(
      (e) => e.line.toLowerCase().includes(q) || e.source.toLowerCase().includes(q),
    );
  }, [entries, search]);

  useLayoutEffect(() => {
    const viewport = scrollRef.current;
    if (!viewport || !autoScrollRef.current) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [visible, follow]);

  // Sample the viewport height and a single row's height so the render can
  // window the list. Re-measures on resize and whenever the buffer size or wrap
  // mode changes (which can change row height). Degrades to 0 in jsdom (no
  // layout), which computeLogWindow treats as "render everything".
  const measure = useCallback(() => {
    const viewport = scrollRef.current;
    if (!viewport) return;
    const firstRow = rowsRef.current?.querySelector<HTMLElement>("[data-log-row]");
    setMetrics({
      scrollTop: viewport.scrollTop,
      viewportHeight: viewport.clientHeight,
      rowHeight: firstRow ? firstRow.getBoundingClientRect().height : 0,
    });
  }, []);

  useLayoutEffect(() => {
    measure();
    const viewport = scrollRef.current;
    if (!viewport || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [measure, visible.length, wrap]);

  function toggleFollow() {
    setFollow((current) => {
      const next = !current;
      if (next) {
        autoScrollRef.current = true;
        setStreamError("");
      }
      return next;
    });
  }

  // Previous-instance logs are terminated-container snapshots — the API can't
  // follow them, so switching them on drops out of live-tail.
  function togglePrevious() {
    setPrevious((current) => {
      const next = !current;
      if (next) setFollow(false);
      return next;
    });
  }

  function trackScroll() {
    const viewport = scrollRef.current;
    if (!viewport) return;
    autoScrollRef.current = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 48;
    // Keep the virtualisation window in step with the scroll position.
    setMetrics((m) => ({ ...m, scrollTop: viewport.scrollTop, viewportHeight: viewport.clientHeight }));
  }

  const flatten = (list: LogEntry[]) =>
    list.map((e) => (e.source ? `${e.source} | ${e.line}` : e.line)).join("\n");

  function download() {
    const base = srcType === "pod" ? srcPod : srcName;
    setSaveError("");
    void saveTextFile(`${base || "logs"}.log`, flatten(entries)).catch((e) => setSaveError(String(e)));
  }

  // Full dump: every container of every in-scope pod, ignoring the container
  // filter, each section headed `==> pod/container <==` (tail-style).
  function downloadAll() {
    const base = srcType === "pod" ? srcPod : srcName;
    setSaveError("");
    setSavingAll(true);
    void (async () => {
      const parts: string[] = [];
      for (const p of targetPods) {
        const discovered = containersByPod[p] ?? [];
        const containers = discovered.length > 0 ? discovered : [undefined];
        for (const c of containers) {
          const out = await podLogs(context, namespace, p, undefined, {
            container: c,
            previous,
            timestamps,
            tailLines,
            sinceSeconds,
          });
          parts.push(`==> ${p}${c ? `/${c}` : ""} <==`);
          parts.push(out.error ? `(error: ${out.error})` : out.logs ?? "");
          parts.push("");
        }
      }
      await saveTextFile(`${base || "logs"}-all.log`, parts.join("\n"));
    })()
      .catch((e) => setSaveError(String(e)))
      .finally(() => setSavingAll(false));
  }

  const title = srcType === "pod" ? srcPod : `${srcKind}/${srcName}`;

  // Only render the on-screen slice of a long, unwrapped buffer (fixed-height
  // rows); wrapped or short buffers render in full.
  const win = computeLogWindow({
    total: visible.length,
    scrollTop: metrics.scrollTop,
    viewportHeight: metrics.viewportHeight,
    rowHeight: metrics.rowHeight,
    wrap,
  });
  const windowRows = win.virtualized ? visible.slice(win.start, win.end) : visible;

  return (
    <div className="flex h-full flex-col bg-card text-card-foreground">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-1.5 text-xs">
        <span className="font-medium text-muted-foreground">{title} · logs</span>

        {srcType === "workload" && pods.length > 0 && (
          <Select
            value={pod}
            onValueChange={setPod}
            options={[{ value: ALL, label: `All pods (${pods.length})` }, ...pods.map((p) => ({ value: p }))]}
            aria-label="Pod"
          />
        )}

        {containerOptions.length > 0 && (
          <Select
            value={container}
            onValueChange={setContainer}
            options={[
              { value: ALL, label: `All containers (${containerOptions.length})` },
              ...containerOptions.map((c) => ({ value: c })),
            ]}
            aria-label="Container"
          />
        )}

        <Select
          value={String(tailLines)}
          onValueChange={(v) => setTailLines(Number(v))}
          options={TAIL_OPTIONS.map((n) => ({ value: String(n), label: `${n} lines` }))}
          aria-label="Tail lines"
        />

        <Select
          value={sinceValue}
          onValueChange={setSinceValue}
          options={SINCE_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
          aria-label="Since"
        />

        <ToggleButton
          icon={History}
          label="Previous instance logs"
          text="prev"
          active={previous}
          onClick={togglePrevious}
          title="Logs from the previous, crashed instance"
        />

        <div className="relative w-44">
          <TextInput value={search} onValueChange={setSearch} placeholder="Search logs…" aria-label="Search logs" />
        </div>
        {search && (
          <span className="tabular-nums text-muted-foreground">
            {visible.length}/{entries.length}
          </span>
        )}

        <div className="ml-auto flex items-center gap-1">
          {loading && <Spinner label="Loading logs" />}
          {follow && streamStatus === "reconnecting" && (
            <span className="text-amber-600 dark:text-amber-400">reconnecting…</span>
          )}
          {follow && streamStatus === "connecting" && (
            <span className="text-muted-foreground">connecting…</span>
          )}
          {follow && !loading && streamStatus === "live" && (
            <span className="flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
              <span className="size-1.5 rounded-full bg-emerald-500" />
              live
            </span>
          )}
          <ToggleButton
            icon={Clock}
            label="Timestamps"
            text="ts"
            active={timestamps}
            onClick={() => setTimestamps((t) => !t)}
            title="Prefix each line with a timestamp"
          />
          <IconButton
            icon={follow ? Pause : Play}
            label={follow ? "Pause live tail" : "Live tail"}
            onClick={toggleFollow}
            disabled={previous}
            title={previous ? "Live tail is unavailable for previous logs" : undefined}
          />
          <IconButton icon={WrapText} label={wrap ? "Disable wrap" : "Wrap lines"} onClick={() => setWrap((w) => !w)} />
          {saveError && <span className="text-red-600 dark:text-red-400">save failed</span>}
          <IconButton icon={Download} label="Download" onClick={download} disabled={entries.length === 0} />
          <IconButton
            icon={DownloadCloud}
            label="Download all containers"
            onClick={downloadAll}
            disabled={savingAll || targetPods.length === 0}
            title="Download every container's logs for the in-scope pods"
          />
          <IconButton icon={RefreshCw} label="Refresh" onClick={() => load()} disabled={follow || loading} />
        </div>
      </div>

      <div
        ref={scrollRef}
        role="log"
        aria-label="Pod logs"
        aria-live={follow ? "polite" : "off"}
        onScroll={trackScroll}
        className="min-h-0 flex-1 overflow-auto font-mono text-xs leading-relaxed"
      >
        {streamError || error ? (
          <div className="p-3 text-red-600 dark:text-red-400">Error: {streamError || error}</div>
        ) : visible.length > 0 ? (
          <div ref={rowsRef} className={wrap ? "whitespace-pre-wrap break-all p-2" : "min-w-max p-2"}>
            {win.topPad > 0 && <div style={{ height: win.topPad }} aria-hidden="true" />}
            {windowRows.map((e, i) => {
              const idx = win.start + i;
              return (
                <div key={idx} data-log-row className={LEVEL_CLASS[lineLevel(e.line)]}>
                  {e.source && (
                    <span className="font-medium" style={{ color: avatarColor(e.source) }}>
                      {e.source} |{" "}
                    </span>
                  )}
                  {e.line || " "}
                </div>
              );
            })}
            {win.bottomPad > 0 && <div style={{ height: win.bottomPad }} aria-hidden="true" />}
          </div>
        ) : (
          <div className="p-3 text-muted-foreground">
            {loading ? "Loading…" : search ? "No matching lines" : follow ? "Waiting for logs…" : "(no logs)"}
          </div>
        )}
      </div>
    </div>
  );
}
