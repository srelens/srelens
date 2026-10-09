import { useEffect, useMemo, useRef, useState } from "react";
import {
  deleteSkill,
  listSkills,
  loadSkill,
  parseSkillFile,
  plural,
  revealSkill,
  saveSkill,
  skillNameProblem,
  skillsDirPath,
  type SkillMeta,
} from "@srelens/core";
import { Badge, Button, ConfirmDialog, Dialog, EmptyState, Field, Switch, TextInput } from "@srelens/ui-kit";
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
 * **Two kinds of skill, kept apart.** The ones that ship with srelens are
 * preinstalled: they are part of the app, cannot be edited or removed here,
 * and can only be switched on or off. The reader's own are installed by the
 * reader — from a file they have, or written in place — and are theirs to
 * edit and uninstall. A reader's skill may not take a preinstalled skill's
 * name, so the two never stand in for each other from this pane.
 *
 * **A reader's skill is a file, and the pane says where.** The folder the
 * files are kept in is shown and can be opened, and each of the reader's
 * skills shows its own file. Edit one in any editor and Refresh reads it
 * again.
 *
 * What is NOT here, because nothing behind it exists yet: skills loaded only
 * when a task calls for them, and a registry to install from. Those are drawn in the design this follows; a control for them
 * here would be a control that does nothing.
 */
export function SkillsPane() {
  const [read, setRead] = useState<Read>({ state: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [query, setQuery] = useState("");
  const [removing, setRemoving] = useState<Listed | null>(null);
  const [removeError, setRemoveError] = useState<unknown>(null);
  const [folder, setFolder] = useState<string | null>(null);
  const [fileError, setFileError] = useState<{ title: string; cause: unknown } | null>(null);
  const [installing, setInstalling] = useState(false);
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

  // Where the files are. Asked once: it does not move while the app runs. A
  // failure costs the path line and nothing else — the list does not need it.
  useEffect(() => {
    let current = true;
    skillsDirPath().then(
      (dir) => current && setFolder(dir),
      () => current && setFolder(null),
    );
    return () => {
      current = false;
    };
  }, []);

  function reveal(name?: string) {
    setFileError(null);
    revealSkill(name).catch((cause: unknown) => setFileError({ title: "Could not open the skills folder", cause }));
  }

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
      // A copy of a bundled skill is different: the skill is still there, as
      // it shipped, and stays as the reader had it — on or off.
      if (!skill.overridesBuiltin) setSkillDefault(skill.name, false);
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
        <Button variant="primary" disabled={read.state !== "ready"} onClick={() => setInstalling(true)}>
          Install skill…
        </Button>
      </section>

      <section className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-rule px-4 py-2" aria-label="Where skill files are kept">
        <span className="text-muted">Skills you install are kept in</span>
        {folder === null ? (
          <span className="text-muted">a folder that could not be found.</span>
        ) : (
          // Selectable, so it can be copied into a terminal or an editor.
          <code className="min-w-0 select-all break-all font-mono text-[0.75rem] text-ink" data-slot="skills-folder">
            {folder}
          </code>
        )}
        <Button variant="secondary" size="xs" onClick={() => reveal()}>
          Open folder
        </Button>
        <span className="basis-full text-[0.75rem] text-muted">
          Each of your skills is one Markdown file. Edit one in any editor, then press Refresh. Preinstalled skills
          are part of the app and are not in this folder.
        </span>
      </section>

      {fileError !== null && (
        <div className="px-4 py-3">
          <FailureAlert tone="sev" title={fileError.title} error={fileError.cause} />
        </div>
      )}

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
          hint={skills.length === 0 ? "Install one with Install skill…" : `Nothing installed matches “${query.trim()}”.`}
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
                      <Badge tone="muted">{skill.builtin ? "Preinstalled" : "Installed by you"}</Badge>
                      {skill.overridesBuiltin && <Badge tone="info">Replaces a preinstalled skill</Badge>}
                    </p>
                    <p className="mt-1 text-ink-soft">{skill.description || "No description."}</p>
                    {/* The file itself, where there is one: what to open to
                        change this skill by hand. */}
                    {skill.path ? (
                      <p className="mt-1 select-all break-all font-mono text-[0.75rem] text-muted" data-slot="skill-path">
                        {skill.path}
                      </p>
                    ) : (
                      skill.builtin && (
                        <p className="mt-1 text-[0.75rem] text-muted">
                          Ships with srelens. It is part of the app and cannot be edited or uninstalled.
                        </p>
                      )
                    )}
                    <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[0.75rem] text-muted">
                      {/* A preinstalled skill has nothing to press but its
                          switch. */}
                      {!skill.builtin && (
                        <>
                          <Button variant="secondary" size="xs" onClick={() => reveal(skill.name)}>
                            Open folder
                          </Button>
                          <Button variant="secondary" size="xs" onClick={() => setRemoving(skill)}>
                            {skill.overridesBuiltin ? "Restore preinstalled version" : "Uninstall"}
                          </Button>
                        </>
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

      {installing && (
        <InstallSkillDialog
          installed={skills}
          onClose={() => setInstalling(false)}
          onInstalled={() => {
            setInstalling(false);
            setAttempt((n) => n + 1);
          }}
        />
      )}

      {removing && (
        <ConfirmDialog
          title={removing.overridesBuiltin ? `Restore the preinstalled ${removing.name}?` : `Uninstall ${removing.name}?`}
          message={
            removing.overridesBuiltin
              ? "Your file is deleted, with any changes you made to it, and the version that ships with srelens is used again."
              : "The skill's file is deleted. Conversations that used it keep their history."
          }
          confirmLabel={removing.overridesBuiltin ? "Restore" : "Uninstall"}
          danger
          onConfirm={() => void remove(removing)}
          onCancel={() => setRemoving(null)}
        />
      )}
    </div>
  );
}

/** A chosen file's text. `FileReader`, which every webview this runs in has. */
function readText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error ?? new Error("The file could not be read."));
    reader.readAsText(file);
  });
}

/**
 * Install a skill of the reader's own: from a file they have, or written here.
 *
 * Choosing a file only fills the form. Nothing is installed until the reader
 * has seen the name, the description and the instructions and pressed
 * Install — a file picked by mistake, or one whose name is taken, is a form to
 * correct rather than a skill already written to disk.
 *
 * A preinstalled skill's name is refused: the shipped skills are not edited
 * from here, and a file under one of their names would stand in for it. A
 * name the reader has already used is allowed, and said to be a replacement
 * before it happens.
 */
function InstallSkillDialog({
  installed,
  onClose,
  onInstalled,
}: {
  installed: readonly Listed[];
  onClose: () => void;
  onInstalled: () => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; cause: unknown } | null>(null);
  const picker = useRef<HTMLInputElement>(null);

  const existing = installed.find((s) => s.name === name.trim());
  const nameProblem =
    name === ""
      ? null
      : (skillNameProblem(name.trim()) ??
        (existing?.builtin || existing?.overridesBuiltin
          ? "That name belongs to a preinstalled skill. Choose another."
          : null));
  const replaces = existing !== undefined && nameProblem === null;
  const ready = name.trim() !== "" && nameProblem === null && body.trim() !== "" && !busy;

  async function choose(file: File | undefined) {
    if (!file) return;
    setError(null);
    try {
      const skill = parseSkillFile(await readText(file), file.name);
      setName(skill.name);
      setDescription(skill.description);
      setBody(skill.body);
    } catch (cause) {
      setError({ title: `Could not read ${file.name}`, cause });
    }
  }

  async function install() {
    setBusy(true);
    setError(null);
    try {
      await saveSkill({ name: name.trim(), description: description.trim(), body });
      onInstalled();
    } catch (cause) {
      setError({ title: "Could not install the skill", cause });
      setBusy(false);
    }
  }

  return (
    <Dialog
      title="Install a skill"
      maxWidth={560}
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" disabled={!ready} onClick={() => void install()}>
            {busy ? "Installing…" : replaces ? "Replace" : "Install"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 p-3 text-[0.8125rem]">
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="secondary" size="sm" onClick={() => picker.current?.click()}>
            Choose a file…
          </Button>
          <span className="text-muted">A Markdown file with the skill's instructions. Or write one below.</span>
          <input
            ref={picker}
            type="file"
            accept=".md,.markdown,.txt,text/markdown,text/plain"
            aria-label="Skill file"
            className="sr-only"
            onChange={(e) => {
              void choose(e.target.files?.[0]);
              // Cleared, so choosing the same file again is still a change.
              e.target.value = "";
            }}
          />
        </div>
        <Field label="Name" hint="Letters, numbers, dots, dashes and underscores." error={nameProblem ?? undefined}>
          <TextInput value={name} onValueChange={setName} placeholder="team-rollback" invalid={nameProblem !== null} />
        </Field>
        <Field label="Description" hint="One line saying when to use it.">
          <TextInput value={description} onValueChange={setDescription} placeholder="How this team rolls back a release" />
        </Field>
        <Field label="Instructions" hint={body === "" ? "What the agent should do, step by step." : `≈${formatTokens(estimateTokens(body))} tokens when on`}>
          <textarea
            value={body}
            onChange={(e) => setBody(e.target.value)}
            rows={9}
            className="scroll w-full rounded-md border border-[var(--control-line)] bg-transparent px-2 py-1.5 font-mono text-[0.75rem] outline-none focus:border-[var(--accent)]"
          />
        </Field>
        {replaces && (
          <p className="text-muted" role="status">
            You already have a skill called {name.trim()}. Installing replaces it.
          </p>
        )}
        {error !== null && <FailureAlert tone="sev" title={error.title} error={error.cause} />}
      </div>
    </Dialog>
  );
}
