import { useContext, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import {
  EXTENSION_LOG_LEVELS,
  extensionLogs,
  type ExtensionLogLevel,
  type ExtensionLogLine,
  type ExtensionRuntime,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice } from "./ExtensionResults";
import { plainText } from "./displayText";
import { extensionLabel } from "./inventoryStore";

/** How many lines the tab holds; the host keeps its own `capacity`. */
const HELD = 1000;
const POLL_MS = 2000;

const pad = (value: number, width = 2) => String(value).padStart(width, "0");
/** A local wall-clock time, 24-hour, to the millisecond unless asked for whole seconds. */
export function clock(ms: number, milliseconds = true) {
  const time = new Date(ms);
  const seconds = `${pad(time.getHours())}:${pad(time.getMinutes())}:${pad(time.getSeconds())}`;
  return milliseconds ? `${seconds}.${pad(time.getMilliseconds(), 3)}` : seconds;
}

/** Who wrote a line: the app's own process on its stderr, or srelens about it. */
export const writer = (source: ExtensionLogLine["source"]) => (source === "sidecar" ? "app" : "srelens");

/**
 * One line of an app's log (#575), as one row of machine text that does not wrap. The
 * fields are separated by spaces, not only by layout, so a copied line reads as one.
 * `level` is left out where every line has the same one, as in the Inspector's errors.
 */
export function LogLineRow({ line, level = true }: { line: ExtensionLogLine; level?: boolean }) {
  return (
    <div className="extension-app-log-line">
      <time className="extension-log-time" dateTime={new Date(line.at).toISOString()}>
        {clock(line.at)}
      </time>{" "}
      {level && (
        <>
          {/* The level is a word; its tint only helps. */}
          <span className="extension-log-level" data-level={line.level}>
            {line.level}
          </span>{" "}
        </>
      )}
      <span className="extension-log-source">{writer(line.source)}</span>{" "}
      {/* The host has redacted it, but it is still the app's text: its control and
          direction characters are drawn as escapes, so it cannot reorder the row. */}
      <span className="extension-log-text">{plainText(line.text)}</span>
    </div>
  );
}

/**
 * An installed app's log (#575): what its process wrote on stderr and what srelens wrote
 * about it. Read on open and then every two seconds for the lines after the last one held.
 */
export function ExtensionLogs({ plugin }: { plugin: InstalledExtension }) {
  const { Button } = useContext(ExtensionControls);
  const id = plugin.manifest.id;
  const levelsLabel = useId();
  const [level, setLevel] = useState<ExtensionLogLevel>("trace");
  const [lines, setLines] = useState<ExtensionLogLine[]>([]);
  /** Whether the lines held answer the chosen level; false until its first read comes back. */
  const [ready, setReady] = useState(false);
  const [store, setStore] = useState<{ runtime: ExtensionRuntime; capacity: number; dropped: number }>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  /** The last line's `seq`; undefined until the first read, which asks from the start. */
  const last = useRef<number | undefined>(undefined);

  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = () => {
      const after = last.current;
      extensionLogs(id, { after, minLevel: level }).then(
        (answer) => {
          if (!live) return;
          const fresh = answer.lines.filter((line) => after === undefined || line.seq > after);
          last.current = fresh.at(-1)?.seq ?? after ?? 0;
          setLines((held) => [...held, ...fresh].slice(-HELD));
          setStore({ runtime: answer.runtime, capacity: answer.capacity, dropped: answer.dropped });
          setReady(true);
          timer = setTimeout(poll, POLL_MS);
        },
        // No poll while a failure stands: the reader chooses when to try again.
        (e: unknown) => live && setError(e instanceof Error ? e.message : String(e)),
      );
    };
    poll();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [id, level, attempt]);

  function choose(next: ExtensionLogLevel) {
    if (next === level) return;
    last.current = 0;
    setLines([]);
    setReady(false);
    setError(undefined);
    setLevel(next);
  }
  // Resumes after the last line held, if any: those lines are still the log's.
  function retry() {
    setError(undefined);
    setAttempt((count) => count + 1);
  }

  const region = useRef<HTMLDivElement>(null);
  /** Whether the reader is at the bottom, where new lines keep them. */
  const following = useRef(true);
  useLayoutEffect(() => {
    const node = region.current;
    if (node && following.current) node.scrollTop = node.scrollHeight;
  }, [lines]);

  const dropped = store?.dropped ?? 0;
  return (
    <div className="extension-logs">
      {store && (
        <p className="extension-message">
          {`Kept in srelens's memory only: never written to disk, and never sent to an agent or anywhere else. The last ${store.capacity.toLocaleString("en-US")} lines are kept${
            dropped > 0
              ? `; ${dropped.toLocaleString("en-US")} older ${dropped === 1 ? "line was" : "lines were"} dropped.`
              : "."
          }`}
        </p>
      )}
      <div className="extension-log-levels" role="group" aria-labelledby={levelsLabel}>
        <span id={levelsLabel}>Minimum level</span>
        {EXTENSION_LOG_LEVELS.map((each) => (
          <Button key={each} variant="ghost" size="xs" aria-pressed={each === level} onClick={() => choose(each)}>
            {each}
          </Button>
        ))}
      </div>
      {error !== undefined && <ErrorNotice title="Could not read this app's log" message={error} retry={retry} />}
      {lines.length > 0 ? (
        // Focusable: it scrolls both ways, and a keyboard must be able to reach the rest.
        <div
          ref={region}
          className="extension-app-log"
          role="log"
          aria-label={`${extensionLabel(plugin)} log`}
          tabIndex={0}
          onScroll={(event) => {
            const node = event.currentTarget;
            following.current = node.scrollHeight - node.scrollTop - node.clientHeight <= 4;
          }}
        >
          {lines.map((line) => (
            <LogLineRow key={line.seq} line={line} />
          ))}
        </div>
      ) : error !== undefined ? null : !ready ? (
        <p role="status" className="extension-message">
          Loading the log…
        </p>
      ) : (
        <p className="extension-message">
          {store?.runtime === "declarative"
            ? "This app has no process, so nothing writes to its log."
            : level === "trace"
              ? "Nothing logged yet."
              : `No lines at ${level} or above.`}
        </p>
      )}
    </div>
  );
}
