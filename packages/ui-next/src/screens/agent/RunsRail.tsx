import { useEffect, useState } from "react";
import { listSkills, relativeTime, type SkillMeta } from "@srelens/core";
import { Alert, RawError, Section, Switch, cx } from "@srelens/ui-kit";
import {
  getActiveRunKey,
  openSavedRun,
  restoreRuns,
  selectRun,
  setSkillActive,
  useAgentRun,
  useRunSummaries,
} from "../../lib/agentRun";
import { LOADING, type Read } from "../../lib/read";

/** §5's rail width, and this screen's alone — see `SideRail`'s note on why the
 *  width is a number per screen rather than a scale. */
export const AGENT_RAIL_WIDTH = 312;

/**
 * `/agent`'s right rail (§5): what else there is to look at beside this run —
 * with two of the mock's three sections drawn honestly short of what it
 * shows, rather than faked.
 *
 * **`Recent runs` draws a title and when it was last touched, and nothing
 * else.** §5 draws `<cluster> · <started> · <n> calls` and a duration
 * alongside each one; `listSessions` answers `SessionMeta { id, title,
 * createdAt, updatedAt }` — no cluster, no call count, no duration (#386).
 * Rendering a placeholder or a fabricated `0 calls` in their place would tell
 * the reader srelens counted something it never did.
 *
 * **`Skills` draws a name and a description, and a switch for THIS run — no
 * usage count.** §5's `used <n>×` has no counter behind it anywhere in
 * srelens (#387). The switch reads and writes `lib/agentRun.ts`'s
 * `activeSkills` through `setSkillActive` — the SAME set the composer's own
 * `/` menu writes to — so flipping a skill on here and picking the same
 * skill from the composer are two doors onto one fact, never two copies of
 * it that can disagree. Never a stored preference (`Session.skills` stays
 * "always empty for now").
 *
 * **`MCP clients` says why it is empty, rather than being empty.** srelens
 * generates the config an MCP client reads; it has no channel back from a
 * client telling it who is connected (#369). A blank section under that
 * heading would read as broken rather than as a boundary the app is honest
 * about.
 */
/**
 * A compact stamp for a rail row: `14:04` for something from today, `1 Sep
 * 14:04` for anything older.
 *
 * `relativeTime` alone answers "how long ago" and not "when", which is the
 * question a reader has when matching a conversation against something else
 * they were doing. `absoluteTimestamp` answers "when" at full length — month,
 * day, year, seconds — which does not fit a 312px row beside a title.
 */
function clockStamp(at: number, now: number): string {
  const d = new Date(at);
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return d.toLocaleString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    ...(sameDay ? {} : { month: "short", day: "numeric" }),
  });
}

export function RunsRail() {
  // A rejected read is neither "loading" nor "read, and empty" — see
  // `lib/read.ts`. `.catch(() => setSessions([]))` used to say "No recent
  // runs yet." for a `listSessions` that never actually answered, and the
  // same for `listSkills`'s "No skills saved yet.": both are confident false
  // statements standing in for a failure this rail never told the reader
  // about (I6).
  const [skills, setSkills] = useState<Read<SkillMeta[]>>(LOADING);
  // NOTE: the agent picker was briefly here, then moved into the composer's
  // own footer beside `+`. Choosing the agent is part of asking, and the
  // composer is on every screen while this rail is on one.
  // THIS window's conversations, not the sessions classic wrote to disk. See
  // the section's own comment for why that list is gone.
  const runs = useRunSummaries();
  const activeKey = getActiveRunKey();
  /** Why the reader's own history is missing, when it is. Not swallowed: an
   *  empty list that should not be empty is exactly the "history is not
   *  correct" this section has already been wrong about once. */
  const [historyError, setHistoryError] = useState<unknown>(null);
  /** One saved conversation that would not open — separate from the listing
   *  failure above it, which is about reading the index rather than a file. */
  const [openError, setOpenError] = useState<unknown>(null);

  useEffect(() => {
    // Saved conversations, read once per mount. Metadata only — a transcript
    // is loaded when the reader opens it.
    void restoreRuns().catch((e: unknown) => setHistoryError(e));
  }, []);
  // The one set the composer's own `/` menu writes to as well — read here,
  // never copied into local state, so the two controls cannot disagree about
  // which skills are active for this run.
  const { activeSkills } = useAgentRun();

  useEffect(() => {
    listSkills()
      .then((v) => setSkills({ kind: "ready", value: v }))
      .catch((e) => setSkills({ kind: "error", error: e }));
  }, []);

  const now = Date.now();

  return (
    <>
      {/*
        This lists the conversations THIS window is holding, and every entry
        opens. It used to list `listSessions()` — the sessions CLASSIC wrote to
        disk — which was wrong three ways at once, all three reported from use:
        the entries were 14 to 22 days old and from a different UI; nothing in
        the new design persists, so a reader's actual conversations were never
        in it (#395); and the rows were plain `<div>`s with no handler, so
        clicking one did nothing.

        A list you cannot open, of conversations you did not have, under a
        heading that says they are recent, is worse than no list.

        Classic's saved sessions are a real thing and worth offering — but
        offering them means `loadSession` hydrating a transcript, which is
        #395's work. Until then they are not shown rather than shown and inert.
      */}
      <Section title="Recent runs" smallCaps padded={false}>
        {historyError !== null && (
          <Alert tone="sev" className="mx-3" title="Saved conversations could not be read">
            <RawError text={String(historyError)} />
          </Alert>
        )}
        {openError !== null && (
          <Alert
            tone="sev"
            className="mx-3"
            title="That conversation could not be opened"
            onDismiss={() => setOpenError(null)}
          >
            <RawError text={String(openError)} />
          </Alert>
        )}
        {runs.length === 0 ? (
          <p className="min-w-0 break-words px-3 text-xs text-muted">
            No questions yet. Asking one from any screen starts a conversation about that thing, and
            it appears here.
          </p>
        ) : (
          <div className="flex min-w-0 flex-col">
            {runs.map((r) => (
              <button
                key={r.key}
                type="button"
                aria-current={r.key === activeKey ? "true" : undefined}
                onClick={() => {
                  // A row on disk has to be LOADED; one in memory is just a
                  // switch. Same control either way, because to the reader
                  // they are the same thing: a conversation they had.
                  if (r.savedId !== undefined) {
                    // Caught, not fire-and-forget. A file listed a moment ago
                    // can be gone, truncated or unreadable by the time it is
                    // opened, and the rejection had nowhere to go: the click
                    // appeared to do nothing and the rejection went unhandled.
                    setOpenError(null);
                    void openSavedRun(r.savedId).catch((e: unknown) => setOpenError(e));
                  } else selectRun(r.key);
                }}
                // Edge to edge, divided by a hairline, no rounded tile and no
                // gap — the design's own list idiom (`Section`'s
                // `padded={false}`), and what §5 draws. Inset rows with gaps
                // between them read as cards, which this is not.
                className={cx(
                  "flex min-w-0 flex-col gap-0.5 border-b border-rule px-3 py-1.5 text-left last:border-b-0 hover:bg-sunk",
                  r.key === activeKey && "bg-sunk",
                )}
              >
                {/* The question first — it is what a reader recognises. The
                    SUBJECT sits under it, because a question alone does not
                    always say what it was about. Both rows read the same way
                    whether the conversation is live or on disk. */}
                <span className="min-w-0 truncate text-sm">{r.label}</span>
                <span className="min-w-0 truncate text-xs text-muted">
                  {r.subject !== undefined && `${r.subject} · `}
                  {r.busy
                    ? "answering…"
                    : r.savedId !== undefined
                      ? "saved"
                      : `${r.turns} question${r.turns === 1 ? "" : "s"}`}{" "}
                  · {clockStamp(r.at, now)} · {relativeTime(r.at, now)}
                </span>
              </button>
            ))}
          </div>
        )}
      </Section>
      <Section title="Skills" smallCaps padded={false}>
        {skills.kind === "loading" ? null : skills.kind === "error" ? (
          <Alert tone="sev" className="mx-3" title="Saved skills could not be checked">
            <RawError text={String(skills.error)} />
          </Alert>
        ) : skills.value.length === 0 ? (
          <p className="min-w-0 break-words px-3 text-xs text-muted">No skills saved yet.</p>
        ) : (
          <div className="flex min-w-0 flex-col">
            {skills.value.map((s) => (
              <div
                key={s.name}
                className="flex min-w-0 items-start justify-between gap-2 border-b border-rule px-3 py-1.5 last:border-b-0"
              >
                <div className="min-w-0 flex-1">
                  <p className="min-w-0 truncate text-sm font-medium">{s.name}</p>
                  <p className="min-w-0 break-words text-xs text-muted">{s.description}</p>
                </div>
                <Switch
                  on={activeSkills.includes(s.name)}
                  onChange={(next) => setSkillActive(s.name, next)}
                  ariaLabel={`Activate ${s.name} for this run`}
                />
              </div>
            ))}
          </div>
        )}
      </Section>
      <Section title="MCP clients" smallCaps padded={false}>
        {/* Banded like the two above it, so all three heads read as one column
            of bands. The paragraph keeps a readable inset of its own — the
            section stops indenting it, which is not the same as text touching
            the edge. */}
        <p className="min-w-0 break-words px-3 py-2 text-xs text-muted">
          srelens generates the config an MCP client reads — it does not know which clients are
          connected right now (#369).
        </p>
      </Section>
    </>
  );
}
