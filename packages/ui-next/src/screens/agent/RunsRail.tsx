import { useEffect, useState } from "react";
import { listSkills, relativeTime, type SkillMeta } from "@srelens/core";
import { Alert, RawError, Section, Switch, cx } from "@srelens/ui-kit";
import {
  getActiveRunKey,
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
export function RunsRail() {
  // A rejected read is neither "loading" nor "read, and empty" — see
  // `lib/read.ts`. `.catch(() => setSessions([]))` used to say "No recent
  // runs yet." for a `listSessions` that never actually answered, and the
  // same for `listSkills`'s "No skills saved yet.": both are confident false
  // statements standing in for a failure this rail never told the reader
  // about (I6).
  const [skills, setSkills] = useState<Read<SkillMeta[]>>(LOADING);
  // THIS window's conversations, not the sessions classic wrote to disk. See
  // the section's own comment for why that list is gone.
  const runs = useRunSummaries();
  const activeKey = getActiveRunKey();
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
      <Section title="Recent runs" smallCaps>
        {runs.length === 0 ? (
          <p className="min-w-0 break-words text-xs text-muted">
            No questions yet. Asking one from any screen starts a conversation about that thing, and
            it appears here.
          </p>
        ) : (
          <div className="flex min-w-0 flex-col gap-1">
            {runs.map((r) => (
              <button
                key={r.key}
                type="button"
                aria-current={r.key === activeKey ? "true" : undefined}
                onClick={() => selectRun(r.key)}
                className={cx(
                  "flex min-w-0 flex-col gap-0.5 rounded-tile px-2 py-1.5 text-left hover:bg-sunk",
                  r.key === activeKey && "bg-sunk",
                )}
              >
                <span className="min-w-0 truncate text-sm">{r.label}</span>
                <span className="min-w-0 truncate text-xs text-muted">
                  {r.busy ? "answering…" : `${r.turns} question${r.turns === 1 ? "" : "s"}`} ·{" "}
                  {relativeTime(r.at, now)}
                </span>
              </button>
            ))}
          </div>
        )}
      </Section>
      <Section title="Skills" smallCaps>
        {skills.kind === "loading" ? null : skills.kind === "error" ? (
          <Alert tone="sev" title="Saved skills could not be checked">
            <RawError text={String(skills.error)} />
          </Alert>
        ) : skills.value.length === 0 ? (
          <p className="min-w-0 break-words text-xs text-muted">No skills saved yet.</p>
        ) : (
          <div className="flex min-w-0 flex-col gap-3">
            {skills.value.map((s) => (
              <div key={s.name} className="flex min-w-0 items-start justify-between gap-2">
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
      <Section title="MCP clients" smallCaps>
        <p className="min-w-0 break-words text-xs text-muted">
          srelens generates the config an MCP client reads — it does not know which clients are
          connected right now (#369).
        </p>
      </Section>
    </>
  );
}
