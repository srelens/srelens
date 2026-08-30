import { useEffect, useState } from "react";
import { listSessions, listSkills, relativeTime, type SessionMeta, type SkillMeta } from "@srelens/core";
import { Section, Switch } from "@srelens/ui-kit";

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
 * srelens (#387); the switch itself is real; it activates a skill the way
 * the composer's own `/` menu does, for the run that is open right now, never
 * a stored preference (`Session.skills` stays "always empty for now").
 *
 * **`MCP clients` says why it is empty, rather than being empty.** srelens
 * generates the config an MCP client reads; it has no channel back from a
 * client telling it who is connected (#369). A blank section under that
 * heading would read as broken rather than as a boundary the app is honest
 * about.
 */
export function RunsRail() {
  const [sessions, setSessions] = useState<SessionMeta[] | null>(null);
  const [skills, setSkills] = useState<SkillMeta[] | null>(null);
  // Which skills are active for the run open right now — plain component
  // state, exactly like the composer's own `activeSkills`: nothing here is
  // persisted, and there is nowhere it would be persisted to.
  const [active, setActive] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    listSessions()
      .then(setSessions)
      .catch(() => setSessions([]));
  }, []);
  useEffect(() => {
    listSkills()
      .then(setSkills)
      .catch(() => setSkills([]));
  }, []);

  function toggle(name: string, on: boolean) {
    setActive((prev) => {
      const next = new Set(prev);
      if (on) next.add(name);
      else next.delete(name);
      return next;
    });
  }

  const now = Date.now();

  return (
    <>
      <Section title="Recent runs" smallCaps>
        {sessions === null ? null : sessions.length === 0 ? (
          <p className="min-w-0 break-words text-xs text-muted">No recent runs yet.</p>
        ) : (
          <div className="flex min-w-0 flex-col gap-2">
            {sessions.map((s) => (
              <div key={s.id} className="flex min-w-0 flex-col gap-0.5">
                <span className="min-w-0 truncate break-words text-sm">{s.title}</span>
                <span className="min-w-0 truncate text-xs text-muted">{relativeTime(s.updatedAt, now)}</span>
              </div>
            ))}
          </div>
        )}
      </Section>
      <Section title="Skills" smallCaps>
        {skills === null ? null : skills.length === 0 ? (
          <p className="min-w-0 break-words text-xs text-muted">No skills saved yet.</p>
        ) : (
          <div className="flex min-w-0 flex-col gap-3">
            {skills.map((s) => (
              <div key={s.name} className="flex min-w-0 items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <p className="min-w-0 truncate break-words text-sm font-medium">{s.name}</p>
                  <p className="min-w-0 break-words text-xs text-muted">{s.description}</p>
                </div>
                <Switch
                  on={active.has(s.name)}
                  onChange={(next) => toggle(s.name, next)}
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
