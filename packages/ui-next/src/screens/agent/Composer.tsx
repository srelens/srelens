import { useEffect, useState } from "react";
import {
  describeError,
  getPrompt,
  listAgents,
  listPrompts,
  listSkills,
  type AgentInfo,
  type PromptSummary,
  type SkillMeta,
} from "@srelens/core";
import { Button, Popover, TextInput } from "@srelens/ui-kit";
import { askAgent, chooseAgent, setSkillActive, stopAgentRun, useAgentRun } from "../../lib/agentRun";

/**
 * The one composer — mounted by the full `/agent` screen alone. The console
 * dock never mounts this: §F's dock has no picker, no Stop and no prompt
 * menu, so `ConsoleDock` supplies its own single-line prompt instead, and
 * `askAgent` is called straight from `shell/Console.tsx`. This is still the
 * spec's "one store, one submit path" (see `Transcript`'s own doc for the
 * sibling half of that) — the ONE submit path just happens to run through
 * this component's own `submit`, not through a second copy of it. It owns the
 * input, the agent picker, the `/` menu of prompts and skills, and Send/Stop;
 * the conversation itself (`turns`, `gates`, `busy`, `generation`) stays in
 * `useAgentRun`.
 *
 * **Every kind srelens can ever drive, named without `listAgents()`.** A
 * fresh install reports an EMPTY list, not four entries marked unavailable —
 * so the "install one to get started" sentence below can't be built from
 * its result. `AgentInfo["kind"]` is a closed union (`chat.ts`'s `AgentKind`)
 * and this is its label for every member, kept here rather than derived.
 */
const AGENT_KIND_LABELS = ["Claude", "Codex", "Cursor", "srelens"];

/**
 * The `/` menu: srelens's diagnostic prompts and saved skills, each under its
 * own heading, exactly as classic's `AssistantConversation.tsx` groups them.
 * A hand-rolled positioned panel rather than the kit's `Popover` — it has to
 * open from the composer's OWN INPUT VALUE (a `/`-prefixed token), not a
 * trigger click, and fighting Radix's open/anchor model for that is exactly
 * what classic's own comment on this menu warns against.
 */
function SlashMenu({
  prompts,
  skills,
  onPickPrompt,
  onPickSkill,
}: {
  prompts: PromptSummary[];
  skills: SkillMeta[];
  onPickPrompt: (p: PromptSummary) => void;
  onPickSkill: (s: SkillMeta) => void;
}) {
  return (
    <div className="absolute bottom-full left-0 z-50 mb-1 max-h-64 w-80 min-w-0 overflow-y-auto rounded-card border border-rule bg-raised p-1 shadow-lg">
      {prompts.length === 0 && skills.length === 0 ? (
        <p className="px-2 py-1.5 text-xs text-muted">No matches.</p>
      ) : (
        <>
          {prompts.length > 0 && (
            <div>
              <p className="px-2 pt-1 text-[10px] font-semibold uppercase tracking-wide text-muted">Prompts</p>
              {prompts.map((p) => (
                <button
                  key={p.name}
                  type="button"
                  className="flex w-full min-w-0 flex-col items-start gap-0.5 rounded-tile px-2 py-1.5 text-left hover:bg-sunk"
                  onClick={() => onPickPrompt(p)}
                >
                  <span className="min-w-0 truncate font-mono text-xs font-medium">{p.name}</span>
                  <span className="min-w-0 w-full truncate break-words text-xs text-muted">{p.description}</span>
                </button>
              ))}
            </div>
          )}
          {skills.length > 0 && (
            <div>
              <p className="px-2 pt-1 text-[10px] font-semibold uppercase tracking-wide text-muted">Skills</p>
              {skills.map((s) => (
                <button
                  key={s.name}
                  type="button"
                  className="flex w-full min-w-0 flex-col items-start gap-0.5 rounded-tile px-2 py-1.5 text-left hover:bg-sunk"
                  onClick={() => onPickSkill(s)}
                >
                  <span className="min-w-0 truncate text-xs font-medium">{s.name}</span>
                  <span className="min-w-0 w-full truncate break-words text-xs text-muted">{s.description}</span>
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

/**
 * The agent picker: a popover over `agents`, ALREADY FILTERED to
 * `available && !gated` by the caller. Never rendered with a `disabled`
 * entry for a gated agent (Ruling: an agent that is installed but gated must
 * not be offered) — the whole point of filtering before this component ever
 * sees the list, rather than filtering here and risking a call site that
 * forgets to.
 */
function AgentPicker({
  agents,
  selectedKind,
  onSelect,
}: {
  agents: AgentInfo[];
  selectedKind: string;
  onSelect: (kind: string) => void;
}) {
  const current = agents.find((a) => a.kind === selectedKind);
  return (
    <Popover label="Choose agent" trigger={<span className="truncate">{current?.label ?? "Agent"}</span>}>
      {(close) => (
        <div role="listbox" className="flex min-w-0 flex-col">
          {agents.map((a) => {
            const selected = a.kind === selectedKind;
            return (
              <button
                key={a.kind}
                type="button"
                role="option"
                aria-selected={selected}
                onClick={() => {
                  onSelect(a.kind);
                  close();
                }}
                className="flex min-w-0 items-center gap-2 rounded-tile px-2 py-1.5 text-left text-sm hover:bg-sunk"
              >
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
              </button>
            );
          })}
        </div>
      )}
    </Popover>
  );
}

/** The required argument every builtin prompt declares (Ruling M) — the one
 *  this composer knows how to fill from its own `context` prop. Any OTHER
 *  required argument means firing `getPrompt` would only ever be refused
 *  (`assistant_prompts.rs` rejects a missing/blank required argument), so
 *  `pickPrompt` below never calls it for one — a menu entry that always
 *  fails is worse than one that says why it can't run. */
const KNOWN_ARG = "context";

/**
 * The required arguments a prompt declares that this composer has no way to
 * fill — either a required argument OTHER than `context`, or `context`
 * itself when nothing has actually been selected.
 *
 * A blank `context` is not a filled one (G1): the prop's TYPE is `string`,
 * but `""` satisfies that type while carrying none of the value a real
 * cluster name would, and the backend's own check
 * (`assistant_prompts.rs:637-641`, `trim().is_empty()`) refuses it exactly
 * as it refuses a missing argument. Folding it into the same "unfillable"
 * list — rather than trusting `arguments` by name alone — means a reader
 * with no cluster selected gets the same local explanation a prompt with a
 * genuinely unfillable argument gets, not a call fired only to be bounced.
 */
function unfillableArgs(p: PromptSummary, context: string): string[] {
  return p.arguments
    .filter((a) => a.required)
    .filter((a) => a.name !== KNOWN_ARG || context.trim() === "")
    .map((a) => a.name);
}

export function Composer({ context }: { context: string }) {
  const { busy, agentKind, activeSkills } = useAgentRun();
  const [input, setInput] = useState("");
  // Three states, not two: `null` is "the read hasn't landed yet", `[]` is
  // "it landed and there is nothing installed" — a boolean can't tell those
  // apart, and collapsing them is exactly what put "No agent is available"
  // on screen for a tick on every mount, agent or no agent (Ruling N).
  const [agents, setAgents] = useState<AgentInfo[] | null>(null);
  const [prompts, setPrompts] = useState<PromptSummary[]>([]);
  const [skills, setSkills] = useState<SkillMeta[]>([]);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [promptError, setPromptError] = useState<string | undefined>(undefined);

  useEffect(() => {
    listAgents()
      .then(setAgents)
      .catch(() => setAgents([]));
  }, []);
  useEffect(() => {
    listPrompts()
      .then(setPrompts)
      .catch(() => setPrompts([]));
  }, []);
  useEffect(() => {
    listSkills()
      .then(setSkills)
      .catch(() => setSkills([]));
  }, []);

  // An agent that is `available` but `gated` must not be offered — Codex and
  // Cursor are installed-but-gated today, and offering one would put the
  // reader in a conversation that cannot start.
  const offered = agents === null ? [] : agents.filter((a) => a.available && !a.gated);
  const agentsLoaded = agents !== null;

  // The store's `agentKind` can name a kind that just went missing from
  // `offered` (nothing loaded yet, or the last-picked kind un-gated to
  // gated) — reconcile it to the first one this composer can actually
  // offer, rather than leaving the picker pointed at a kind it never shows.
  useEffect(() => {
    if (offered.length > 0 && !offered.some((a) => a.kind === agentKind)) {
      chooseAgent(offered[0].kind);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agents]);

  const slashMatch = /^\/(\S*)$/.exec(input);
  const menuOpen = slashMatch !== null && !menuDismissed && (prompts.length > 0 || skills.length > 0);
  const query = (slashMatch?.[1] ?? "").toLowerCase();
  const filteredPrompts = query ? prompts.filter((p) => p.name.toLowerCase().includes(query)) : prompts;
  const filteredSkills = query ? skills.filter((s) => s.name.toLowerCase().includes(query)) : skills;

  function handleChange(value: string) {
    setInput(value);
    setMenuDismissed(false);
    setPromptError(undefined);
  }

  /** Renders `p` into the input for review — never auto-sent. A prompt
   *  declaring a required argument this composer has no source for is never
   *  called at all (Ruling M): it says so instead of firing a `getPrompt`
   *  the backend will only ever refuse. A rejection from a call this DOES
   *  make surfaces through `describeError`, never the raw backend string,
   *  and leaves the input untouched. */
  async function pickPrompt(p: PromptSummary) {
    const missing = unfillableArgs(p, context);
    if (missing.length > 0) {
      setPromptError(
        missing.includes(KNOWN_ARG)
          ? `"${p.name}" needs a cluster in context — select one first.`
          : `"${p.name}" needs ${missing.join(", ")} — open it outside the composer.`,
      );
      setMenuDismissed(true);
      return;
    }
    try {
      const text = await getPrompt(p.name, { context });
      setInput(text);
      setMenuDismissed(true);
      setPromptError(undefined);
    } catch (e) {
      setPromptError(describeError(e).detail);
    }
  }

  /** Activates a skill for THIS RUN only, through the shared store
   *  (`setSkillActive`) — never a stored preference (`Session.skills` stays
   *  "always empty for now"). The store, not this component, is what drops
   *  it once the conversation it was picked for is gone (`clearAgentRun`),
   *  so it survives this composer unmounting and reaches `RunsRail`'s own
   *  switch on the same run. */
  function pickSkill(s: SkillMeta) {
    setSkillActive(s.name, true);
    setMenuDismissed(true);
  }

  function removeSkill(name: string) {
    setSkillActive(name, false);
  }

  async function submit() {
    const question = input.trim();
    if (!question || busy) return;
    setInput("");
    await askAgent(question);
  }

  // Stop must survive whatever `agents`/`offered` are doing — a turn already
  // running does not stop being real because the picker's list emptied out
  // from under it (P3): a reader who loses Send when nothing can be offered
  // must never also lose the only way to stop what IS running.
  if (busy && offered.length === 0) {
    return (
      <div className="flex min-w-0 items-center justify-between gap-2">
        <p className="min-w-0 break-words text-sm text-muted">
          {agentsLoaded
            ? `No agent is available. srelens can drive ${AGENT_KIND_LABELS.join(", ")} — install one to get started.`
            : "Loading agents…"}
        </p>
        <Button type="button" variant="secondary" size="sm" onClick={() => stopAgentRun()}>
          Stop
        </Button>
      </div>
    );
  }

  // The read hasn't landed yet — say nothing about "no agent" until it has
  // (Ruling N), rather than asserting an absence this hasn't checked for.
  if (!agentsLoaded) {
    return <p className="min-w-0 break-words text-sm text-muted">Loading agents…</p>;
  }

  // With no agent available at all, say what srelens can drive and offer no
  // send control — not a disabled one. A control that can never work is
  // worse than its absence.
  if (offered.length === 0) {
    return (
      <p className="min-w-0 break-words text-sm text-muted">
        No agent is available. srelens can drive {AGENT_KIND_LABELS.join(", ")} — install one to get
        started.
      </p>
    );
  }

  const selectedKind = offered.some((a) => a.kind === agentKind) ? agentKind : offered[0].kind;

  return (
    <div className="flex min-w-0 flex-col gap-2">
      {activeSkills.length > 0 && (
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          {activeSkills.map((name) => (
            <span
              key={name}
              className="inline-flex max-w-[10rem] min-w-0 items-center gap-1 rounded-tile border border-accent-line bg-accent-wash px-2 py-1 text-xs text-accent"
            >
              <span className="min-w-0 truncate break-words">{name}</span>
              <button
                type="button"
                aria-label={`Remove skill ${name}`}
                onClick={() => removeSkill(name)}
                className="shrink-0 opacity-70 hover:opacity-100"
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
      {promptError && <p className="min-w-0 break-words text-xs text-sev">{promptError}</p>}
      <div className="relative min-w-0">
        {menuOpen && (
          <SlashMenu
            prompts={filteredPrompts}
            skills={filteredSkills}
            onPickPrompt={(p) => void pickPrompt(p)}
            onPickSkill={pickSkill}
          />
        )}
        <TextInput
          value={input}
          onValueChange={handleChange}
          onEnter={() => {
            if (!menuOpen) void submit();
          }}
          onEscape={() => {
            if (menuOpen) setMenuDismissed(true);
          }}
          placeholder="Ask about this cluster…   /  for prompts & skills"
          disabled={busy}
          aria-label="Ask the agent"
        />
      </div>
      <div className="flex min-w-0 items-center gap-2">
        <AgentPicker
          agents={offered}
          selectedKind={selectedKind}
          onSelect={(kind) => chooseAgent(kind)}
        />
        <div className="flex-1" />
        {busy ? (
          <Button type="button" variant="secondary" size="sm" onClick={() => stopAgentRun()}>
            Stop
          </Button>
        ) : (
          <Button type="button" variant="primary" size="sm" onClick={() => void submit()} disabled={!input.trim()}>
            Send
          </Button>
        )}
      </div>
    </div>
  );
}
