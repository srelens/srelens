import { useEffect, useMemo, useState } from "react";
import { deleteSkill, listSkills, loadSkill, plural, type SkillMeta } from "@srelens/core";
import { Badge, Button, ConfirmDialog, EmptyState, Switch, TextInput } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { estimateTokens, formatTokens, setSkillDefault, useSkillDefaults } from "../../lib/skillDefaults";
import { useSkillUses } from "../../lib/skillUses";

/** A skill as this pane lists it: what it is, and what it costs to have on. */
interface Listed extends SkillMeta {
  /** About how many tokens its instructions are, or `null` if they could not be read. */
  tokens: number | null;
}

type Read = { state: "loading" } | { state: "failed"; error: unknown } | { state: "ready"; skills: Listed[] };

/**
 * Settings → Skills (#851): the skills the agent can be given, and which of
 * them it is given without being asked.
 *
 * A skill is a short runbook — how to triage a crash loop, what to check
 * before a rollout is called stuck. They could be switched on for one
 * conversation from the agent screen and nowhere kept; this is where the kept
 * choice is made, and where a reader sees what that choice costs.
 *
 * **The cost is said as it is.** A skill that is on has its whole
 * instructions sent with every question — that is how the agent run applies
 * skills today — so the figure at the top is what the reader's choices add to
 * each question, and it grows with every switch they turn on. It is an
 * estimate (a quarter of the characters) and is marked as one.
 *
 * What is NOT here, because nothing behind it exists yet: skills loaded only
 * when a task calls for them, a registry to install from, adding a skill from
 * a folder. Those are drawn in the design this follows; a control for them
 * here would be a control that does nothing.
 */
export function SkillsPane() {
  const [read, setRead] = useState<Read>({ state: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [query, setQuery] = useState("");
  const [removing, setRemoving] = useState<Listed | null>(null);
  const [removeError, setRemoveError] = useState<unknown>(null);
  const on = useSkillDefaults();
  const uses = useSkillUses();

  useEffect(() => {
    let current = true;
    setRead({ state: "loading" });
    void (async () => {
      try {
        const metas = await listSkills();
        // Sized one by one, and one that cannot be read is listed without a
        // size rather than costing the reader the whole list.
        const bodies = await Promise.allSettled(metas.map((m) => loadSkill(m.name)));
        if (!current) return;
        setRead({
          state: "ready",
          skills: metas.map((m, i) => {
            const body = bodies[i];
            return { ...m, tokens: body.status === "fulfilled" ? estimateTokens(body.value.body) : null };
          }),
        });
      } catch (error) {
        if (current) setRead({ state: "failed", error });
      }
    })();
    return () => {
      current = false;
    };
  }, [attempt]);

  const skills = read.state === "ready" ? read.skills : [];
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return skills;
    return skills.filter((s) => s.name.toLowerCase().includes(needle) || s.description.toLowerCase().includes(needle));
  }, [skills, query]);

  const enabled = skills.filter((s) => on.includes(s.name));
  const cost = enabled.reduce((sum, s) => sum + (s.tokens ?? 0), 0);
  const unsized = enabled.filter((s) => s.tokens === null).length;

  async function remove(skill: Listed) {
    setRemoveError(null);
    try {
      await deleteSkill(skill.name);
      // A deleted skill is not left on: its name would otherwise stay in the
      // kept set, counted as a choice the reader can no longer see or undo.
      setSkillDefault(skill.name, false);
      setRemoving(null);
      setAttempt((n) => n + 1);
    } catch (error) {
      setRemoveError(error);
      setRemoving(null);
    }
  }

  return (
    <div className="min-w-0 text-[0.8125rem]">
      <section className="border-b border-rule px-4 py-3" aria-label="About skills">
        <h2 className="font-semibold text-ink">Skills</h2>
        <p className="mt-1 max-w-[70ch] text-muted">
          Short runbooks the agent follows: how to triage a crash loop, what to check before a rollout is called
          stuck. A skill that is on is sent with every question, in every new conversation, until you turn it off.
          You can still switch one on or off for a single conversation from the agent screen.
        </p>
      </section>

      <section className="border-b border-rule px-4 py-3" aria-label="What the skills that are on cost">
        <p className="flex flex-wrap items-baseline gap-x-2">
          <span className="eyebrow">Added to every question</span>
          <span className="num text-[1.125rem] text-ink" data-slot="skills-cost">
            ≈{formatTokens(cost)}
          </span>
          <span className="text-muted">
            tokens, from {plural(enabled.length, "skill")} that {enabled.length === 1 ? "is" : "are"} on
          </span>
        </p>
        {unsized > 0 && (
          <p className="mt-1 text-[0.75rem] text-muted">
            {plural(unsized, "skill")} could not be read and {unsized === 1 ? "is" : "are"} not counted.
          </p>
        )}
        <p className="mt-1 text-[0.75rem] text-muted">
          An estimate: about a quarter of the characters in each skill's instructions.
        </p>
      </section>

      <section className="flex flex-wrap items-center gap-2 border-b border-rule px-4 py-2" aria-label="Find a skill">
        <span className="font-semibold">Installed ({skills.length})</span>
        <span className="min-w-[12rem] flex-1">
          <TextInput value={query} onValueChange={setQuery} placeholder="Search installed skills…" aria-label="Search installed skills" />
        </span>
        <Button variant="secondary" disabled={read.state === "loading"} onClick={() => setAttempt((n) => n + 1)}>
          {read.state === "loading" ? "Refreshing…" : "Refresh"}
        </Button>
      </section>

      {removeError !== null && (
        <div className="px-4 py-3">
          <FailureAlert tone="sev" title="Could not remove the skill" error={removeError} />
        </div>
      )}

      {read.state === "failed" ? (
        <div className="flex flex-col items-start gap-2 px-4 py-3">
          <FailureAlert tone="sev" title="Could not load skills" error={read.error} />
          <Button variant="secondary" onClick={() => setAttempt((n) => n + 1)}>
            Try again
          </Button>
        </div>
      ) : read.state === "loading" && skills.length === 0 ? (
        <p className="px-4 py-3 text-muted" role="status">
          Loading skills…
        </p>
      ) : shown.length === 0 ? (
        <EmptyState
          compact
          title={skills.length === 0 ? "No skills" : "No skill matches"}
          hint={skills.length === 0 ? "Skills you save from the agent screen appear here." : `Nothing installed matches “${query.trim()}”.`}
        />
      ) : (
        <ul className="flex flex-col gap-2 px-4 py-3" aria-label="Installed skills">
          {shown.map((skill) => {
            const isOn = on.includes(skill.name);
            const used = uses[skill.name] ?? 0;
            return (
              <li key={skill.name} className="rounded-md border border-rule px-3 py-2.5" data-skill={skill.name}>
                <div className="flex items-start gap-3">
                  <div className="min-w-0 flex-1">
                    <p className="flex flex-wrap items-center gap-2">
                      <span className="font-mono font-semibold text-ink">{skill.name}</span>
                      <Badge tone="muted">{skill.builtin ? "Bundled" : "User"}</Badge>
                    </p>
                    <p className="mt-1 text-ink-soft">{skill.description || "No description."}</p>
                    <p className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[0.75rem] text-muted">
                      {skill.builtin ? (
                        <span>Ships with srelens</span>
                      ) : (
                        <Button variant="secondary" size="xs" onClick={() => setRemoving(skill)}>
                          Uninstall
                        </Button>
                      )}
                      {used > 0 && <span>used {used}×</span>}
                    </p>
                  </div>
                  <div className="flex shrink-0 flex-col items-end gap-1.5">
                    <Switch
                      on={isOn}
                      ariaLabel={`${skill.name}: on for every new conversation`}
                      onChange={(next) => setSkillDefault(skill.name, next)}
                    />
                    <span className="num text-[0.75rem] text-muted" data-slot="skill-tokens">
                      {skill.tokens === null ? "size unknown" : `≈${formatTokens(skill.tokens)} tokens when on`}
                    </span>
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {removing && (
        <ConfirmDialog
          title={`Uninstall ${removing.name}?`}
          message="The skill's file is deleted. Conversations that used it keep their history."
          confirmLabel="Uninstall"
          danger
          onConfirm={() => void remove(removing)}
          onCancel={() => setRemoving(null)}
        />
      )}
    </div>
  );
}
