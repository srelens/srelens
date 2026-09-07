import { useMemo, useState } from "react";
import { appLogPath, describeError, isTauri, notify, readAppLog, revealAppLog } from "@srelens/core";
import {
  EmptyState,
  FilterBar,
  IconButton,
  LoadingState,
  LogLine,
  Screen,
  Select,
} from "@srelens/ui-kit";
import { FailureState } from "../lib/errorCopy";
import { useResource } from "../lib/useResource";
import { Icons } from "../lib/icons";
import { LEVELS, filterLines, parseAppLog, type Level } from "../lib/appLogLines";
import { groupNumber } from "../lib/numbers";

const LEVEL_OPTIONS = [
  { value: "all", label: "All levels" },
  ...LEVELS.map((level) => ({ value: level, label: level })),
];

/**
 * srelens's own log: the tail of the rotating file it writes as it runs,
 * filtered by level and text, with a way to refresh it, copy its path, or open
 * it where it lives.
 *
 * This is the screen someone reaches *after* something went wrong — a cluster
 * that would not connect, an RBAC denial, a panic — so the load is deliberately
 * unclever: read the whole tail on mount, filter it in memory, and offer a
 * refresh rather than a tail-follow. Nothing here streams, because the
 * interesting entry was written before the screen was opened.
 *
 * The file only exists where srelens itself is running, so the browser build
 * has no log to show and says so rather than failing a read: `isTauri` is
 * checked inside the loader as well as in the render, so the web branch never
 * so much as schedules a command the backend cannot answer.
 */
export function AppLog(_props: { route: string }) {
  const [text, setText] = useState("");
  const [level, setLevel] = useState<Level | "all">("all");

  // The path is fetched with the text, in one round trip, because both are
  // needed the moment the screen settles and neither is worth its own state.
  const log = useResource(
    () =>
      isTauri()
        ? Promise.all([readAppLog(), appLogPath()])
        : Promise.resolve<[string, string]>(["", ""]),
    [],
    ([raw]) => raw.trim() === "",
  );

  const [raw = "", path = ""] = log.data ?? [];
  const lines = useMemo(() => parseAppLog(raw), [raw]);
  const { lines: filtered, total } = useMemo(
    () => filterLines(lines, text, level),
    [lines, text, level],
  );

  /**
   * Open the log where it lives — and say so when the machine will not.
   *
   * `revealAppLog` is a bare `await invokeCommand("reveal_app_log")` with no
   * catch of its own, and the command behind it returns `Result<(), String>`:
   * no file manager, a sandbox denial, a log directory it cannot resolve. Fired
   * as `void revealAppLog()` those were unhandled rejections with nothing on
   * screen — the one control here that did not report its own failure.
   *
   * A TOAST, not the banner below: this gesture has no slot on the screen, and
   * nothing the reader was reading has gone wrong. Same rule
   * `NewForwardDialog` settled on for a browser that will not open.
   */
  async function reveal() {
    try {
      await revealAppLog();
    } catch (e) {
      notify.error(
        "Couldn't reveal the application log",
        describeError(e, { domain: "local" }).detail,
      );
    }
  }

  async function copyPath() {
    try {
      await navigator.clipboard.writeText(path);
    } catch {
      // No clipboard on a non-secure origin, and nothing to recover: the path
      // is one refresh away from being read off the reveal dialog instead.
    }
  }

  if (!isTauri()) {
    return (
      <Screen title="Application log" eyebrow="srelens" fill>
        <EmptyState
          title="The application log is a file on the machine running srelens"
          hint="Read it there — in a container, its output is what docker logs prints."
          // `fill` hands the body the whole area and leaves the centring to
          // whatever is in it; without this the card sits at the top edge.
          className="flex-1"
        />
      </Screen>
    );
  }

  // `log.status` is "empty" for a file with nothing in it, but the rendering
  // branch is `lines.length`: a file holding only whitespace loads as "ready"
  // and still has no entries to draw.
  const loaded = log.status === "ready" || log.status === "empty";

  return (
    <Screen
      title="Application log"
      eyebrow="srelens"
      fill
      actions={
        <>
          <IconButton
            icon={Icons.refresh}
            label="Refresh"
            onClick={log.reload}
            disabled={log.status === "loading"}
          />
          <IconButton
            icon={Icons.copy}
            label="Copy path"
            // The name stays short; the tooltip says which path, since the
            // screen has nowhere else to show it.
            title={path ? `Copy path (${path})` : "Copy path"}
            onClick={() => void copyPath()}
            disabled={!path}
          />
          <IconButton icon={Icons.reveal} label="Reveal" onClick={() => void reveal()} />
        </>
      }
    >
      <FilterBar
        value={text}
        onValueChange={setText}
        label="Filter log lines"
        placeholder="Filter log lines…"
      >
        <Select
          value={level}
          onValueChange={(value) => setLevel(value as Level | "all")}
          options={LEVEL_OPTIONS}
          aria-label="Log level"
        />
      </FilterBar>

      {log.status === "loading" && <LoadingState label="Loading the log" />}
      {log.status === "error" && (
        // `FailureState`, not the kit's raw `ErrorState`: the string Rust sent
        // goes through `describeError` and its original folds away behind a
        // disclosure, which is what every other screen in this area does with
        // the same class of value. This screen's own title stays — it is the
        // half the classification cannot know.
        <FailureState
          title="Could not read the application log"
          error={log.error}
          domain="local"
          onRetry={log.reload}
        />
      )}
      {loaded && (
        <div
          role="log"
          aria-label="Application log"
          className="scroll min-h-0 flex-1 py-1 font-mono text-[0.75rem] leading-relaxed"
        >
          {lines.length === 0 ? (
            <EmptyState
              title="No log entries yet"
              hint="srelens writes to this file as it runs; there is nothing in it so far."
            />
          ) : filtered.length === 0 ? (
            <EmptyState title="No lines match" />
          ) : (
            <>
              {total > filtered.length && (
                // The cap bit: more lines matched than fit on screen. Said
                // here rather than folded into the empty state above, because
                // this is the one place a truncated log must not look
                // complete — silence here would be the same defect the empty
                // state's wrong wording used to cause the other way round.
                <div className="px-2.5 py-1 text-[0.75rem] text-muted">
                  Showing the newest {groupNumber(filtered.length)} of {groupNumber(total)} lines
                </div>
              )}
              {filtered.map((line, i) => (
                // The index is the key: two identical lines a second apart are
                // ordinary in a log, so nothing in the entry is a stable
                // identity, and the list is rebuilt whole on every filter change.
                <LogLine
                  key={i}
                  ts={line.ts}
                  source={line.source}
                  level={line.level}
                  message={line.message}
                />
              ))}
            </>
          )}
        </div>
      )}
    </Screen>
  );
}
