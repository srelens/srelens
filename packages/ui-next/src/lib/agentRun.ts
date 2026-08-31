import { useSyncExternalStore } from "react";
import {
  cancelChat,
  describeError,
  listAgents,
  loadSkill,
  sendChat,
  startChat,
  type AgentEvent,
  type Skill,
  type ToolStatus,
} from "@srelens/core";

/**
 * The one agent run this window is holding — every turn asked and answered,
 * every tool call inside them, and the gates an agent's mutations are waiting
 * on — so the console dock and the `/agent` screen can be two views of the
 * same conversation rather than two copies of it.
 *
 * Shaped after `helmOps.ts` and `sessions.ts`: module-level state, a listener
 * set, `emit()` copying it before iterating so a listener that unsubscribes
 * mid-notification does not upset the loop, and a snapshot that keeps its
 * reference until something in it actually changed, so `useSyncExternalStore`
 * has a stable value to compare.
 *
 * **Gates live here, not on `Turn`.** A gate arrives from an app-wide
 * listener (`AgentConsent`), not from the chat stream this store reads, so
 * there is no turn to hang it off without guessing which one asked for it.
 * `noteGate` merges by id instead: an id already present is updated in
 * place — the same request moving from `pending` to `approved` or `denied` —
 * never appended twice.
 */

/** The three states of Kubernetes trouble srelens's tool calls can end in. */
export type ToolCallRecord = {
  id: string;
  tool: string;
  args: unknown;
  status: ToolStatus | null;
  /**
   * Round trip srelens observed, ms. Absent until `toolResult` lands — a
   * duration nobody measured yet is not the same fact as a duration of zero,
   * and rendering the two alike would tell the reader a call finished
   * instantly when it has not finished at all.
   */
  ms?: number;
};

/** One MCP confirm request, as `AgentConsent` reports it. */
export type GateRecord = {
  id: string;
  tool: string;
  args: unknown;
  outcome: "pending" | "approved" | "denied";
  /**
   * When this gate was resolved, `Date.now()`-shaped — absent while it is
   * still `pending`. Stamped by `AgentConsent` at the moment `outcome`
   * actually changes, never here and never at render time: this module has
   * no window onto when an answer arrived, and a time taken from anywhere
   * else would report when something drew rather than when the gate itself
   * resolved.
   */
  at?: number;
};

/** One turn of the conversation — the reader's question, or the agent's
 *  answer (or, when the stream itself failed, the reason it stopped). */
export type Turn = {
  id: number;
  role: "user" | "agent" | "error";
  text: string;
  calls: ToolCallRecord[];
  thoughts?: string;
  images?: string[];
  at: number;
};

/** The whole conversation this window is holding. */
export type AgentRun = {
  turns: Turn[];
  gates: GateRecord[];
  busy: boolean;
  /** Turn generation, for `cancelChat` — the backend matches a Stop against
   *  the generation its own `sendChat` was sent with, so a Stop that arrives
   *  before that send is honored and a stale Stop left over from an earlier
   *  turn is dropped. */
  generation: number;
  agentKind: string;
  /**
   * Skill names active for the run open right now — never persisted
   * (`Session.skills` stays "always empty for now"), and dropped whenever
   * the run clears (`clearAgentRun`).
   *
   * **One set, two writers, one reader.** The composer's `/` menu and the
   * rail's own switch (`RunsRail`) both write here through
   * {@link setSkillActive}; `askAgent` is the only reader, folding whichever
   * names are active into the guidance it sends. Living here rather than in
   * either component's own state is what keeps the two controls from
   * disagreeing about which skills are on — a `Composer`-local copy is
   * exactly the split that once left the rail's switch flipping nothing.
   */
  activeSkills: string[];
  error?: string;
};

let run: AgentRun = {
  turns: [],
  gates: [],
  busy: false,
  generation: 0,
  agentKind: "claude",
  activeSkills: [],
};

/** The CLI's own session id for this conversation — `null` until the first
 *  turn, and again whenever `sendChat` says to clear it (see its doc). */
let session: string | null = null;
/** The agent CLI's own conversation id, passed back as `resume` so a
 *  follow-up turn keeps its context. A REJECTED `sendChat` says nothing about
 *  this and leaves it exactly as it was. */
let resume: string | null = null;
/**
 * A generation `stopAgentRun` was asked to stop before its `askAgent` had
 * even reached `sendChat` — `session` is created lazily inside `askAgent`,
 * AFTER `busy` and `generation` are already committed, so a Stop that
 * arrives in that window finds no session to hand `cancelChat`. Recorded
 * here instead, and honored by `askAgent` itself once its own `startChat()`
 * resolves. `null` when nothing is waiting on this; set to the run's current
 * generation by `stopAgentRun`'s early-session-less branch, and cleared the
 * moment the next question starts (a later generation can never equal an
 * older stopped one, but a stale value sitting here after its turn is long
 * gone is not a fact this module wants to be carrying around).
 */
let stoppedGeneration: number | null = null;
/** Ids are the store's own, not the backend's. */
let turnSeq = 0;

const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

/**
 * The one way the snapshot changes: take the rebuilt run only if something in
 * it actually differs from the current one. Identity is the snapshot's
 * contract, and every call site rebuilds its slice of the run with a fresh
 * object or array — `updateTurn`'s `{ ...t, text: t.text + e.text }` for a
 * zero-length delta, `noteGate` re-merging a gate whose outcome did not
 * change, a `clearAgentRun` on a run that is already empty — none of which
 * change any value a reader could see. Centralized here rather than at each
 * call site: a guard `chooseAgent` remembers to add and `updateTurn` forgets
 * is no guard at all, just one that fails silently for tool calls and text.
 */
function commit(next: AgentRun) {
  if (sameRun(next, run)) return;
  run = next;
  emit();
}

function sameRun(a: AgentRun, b: AgentRun): boolean {
  return (
    a === b ||
    (a.busy === b.busy &&
      a.generation === b.generation &&
      a.agentKind === b.agentKind &&
      a.error === b.error &&
      sameStrings(a.activeSkills, b.activeSkills) &&
      sameList(a.turns, b.turns, sameTurn) &&
      sameList(a.gates, b.gates, sameGate))
  );
}

function sameList<T>(a: T[], b: T[], sameItem: (x: T, y: T) => boolean): boolean {
  return a === b || (a.length === b.length && a.every((item, i) => sameItem(item, b[i])));
}

function sameTurn(a: Turn, b: Turn): boolean {
  return (
    a === b ||
    (a.id === b.id &&
      a.role === b.role &&
      a.text === b.text &&
      a.thoughts === b.thoughts &&
      a.at === b.at &&
      sameStrings(a.images, b.images) &&
      sameList(a.calls, b.calls, sameCall))
  );
}

function sameCall(a: ToolCallRecord, b: ToolCallRecord): boolean {
  return (
    a === b ||
    (a.id === b.id && a.tool === b.tool && a.status === b.status && a.ms === b.ms && a.args === b.args)
  );
}

function sameGate(a: GateRecord, b: GateRecord): boolean {
  return (
    a === b ||
    (a.id === b.id && a.tool === b.tool && a.outcome === b.outcome && a.args === b.args && a.at === b.at)
  );
}

function sameStrings(a: string[] | undefined, b: string[] | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** Subscribe to store changes (for `useSyncExternalStore`). */
export function subscribeAgentRun(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * The conversation as it stands.
 *
 * A stable reference until something in it changes: `useSyncExternalStore`
 * compares snapshots by identity and re-renders forever when handed a fresh
 * object on every read.
 */
export function getAgentRun(): AgentRun {
  return run;
}

/** The store, subscribed. */
export function useAgentRun(): AgentRun {
  return useSyncExternalStore(subscribeAgentRun, getAgentRun, getAgentRun);
}

/** Find, and update, the turn with this id — a no-op if the turn is no
 *  longer in the run (cleared, or this write's own turn superseded by a
 *  later one), rather than resurrecting it or emitting a change nothing
 *  actually saw. */
function updateTurn(id: number, updater: (t: Turn) => Turn) {
  const idx = run.turns.findIndex((t) => t.id === id);
  if (idx === -1) return;
  const turns = run.turns.slice();
  turns[idx] = updater(turns[idx]);
  commit({ ...run, turns });
}

/** Turn this turn into the error it ended on, said the way a reader can use
 *  it — never the raw backend string. */
function markTurnError(id: number, reason: unknown) {
  updateTurn(id, (t) => ({ ...t, role: "error", text: describeError(reason).detail }));
}

/**
 * Fetches each active skill's body and folds them into a guidance block sent
 * ahead of the question — never recorded in the turn itself (`askAgent`
 * stamps the reader's own turn with `question` alone), so what is
 * TRANSMITTED and what is RECORDED can differ without either turning into
 * the other's job. Matches classic's own `loadSkillsGuidance`
 * (`AssistantConversation.tsx`) wording exactly.
 *
 * `allSettled` rather than `all`: an active skill can go missing (deleted
 * from disk after being activated), and one `loadSkill` rejection must not
 * cost the turn every OTHER skill's guidance along with it.
 */
async function loadSkillsGuidance(names: string[]): Promise<string> {
  if (names.length === 0) return "";
  const results = await Promise.allSettled(names.map((name) => loadSkill(name)));
  const bodies = results
    .filter((r): r is PromiseFulfilledResult<Skill> => r.status === "fulfilled")
    .map((r) => r.value.body);
  if (bodies.length === 0) return "";
  return `Apply these skills:\n\n${bodies.join("\n\n")}\n\n`;
}

/**
 * Ask the current agent one question, under whichever skills' guidance are
 * active for this run — the store's own business, per this module's
 * transmitted-vs-recorded split (see `loadSkillsGuidance`): this fetches and
 * prepends their bodies, and the turn recorded in `run.turns` holds
 * `question` alone. A component that prepended the guidance itself would
 * either leak it into the visible transcript or force this function to grow
 * a second, display-only parameter — the same split, computed in the wrong
 * layer.
 *
 * **`opts.skills` is an explicit override, not the ordinary path.** The
 * ordinary path reads `run.activeSkills` — the one set `setSkillActive`'s two
 * writers (the composer's `/` menu, the rail's switch) both write to — so a
 * ordinary caller (the console dock's own `askAgent(question)`, every screen's
 * `ask()`) picks up whatever is active without saying so itself. `opts.skills`
 * exists for a caller that has a list in hand it wants sent instead, which
 * today is only ever this module's own tests.
 *
 * The reader's turn (and an empty placeholder for the agent's) land in the
 * run before anything is awaited, so "records the reader's question before
 * the agent has said anything" is true the instant this starts, not once a
 * round trip has come back. `session` is started lazily, once, on the first
 * question; every question after reuses it and carries `resume` so the CLI
 * picks the conversation back up.
 */
export async function askAgent(question: string, opts?: { images?: string[]; skills?: string[] }): Promise<void> {
  const images = opts?.images;
  const skills = opts?.skills ?? run.activeSkills;
  const userTurn: Turn = { id: ++turnSeq, role: "user", text: question, calls: [], images, at: Date.now() };
  const agentTurnId = ++turnSeq;
  const agentTurn: Turn = { id: agentTurnId, role: "agent", text: "", calls: [], at: Date.now() };
  const myGeneration = run.generation + 1;
  const agentKind = run.agentKind;

  commit({
    ...run,
    turns: [...run.turns, userTurn, agentTurn],
    busy: true,
    generation: myGeneration,
    error: undefined,
  });
  // A stop recorded against an OLDER generation belongs to a turn already
  // gone; this one hasn't been asked to stop by anyone yet.
  stoppedGeneration = null;

  // Keyed by the backend's own call id, and scoped to this one question: two
  // calls from two different questions never share an id, so there is
  // nothing to clean up once this function returns.
  const callStarts = new Map<string, number>();

  function onEvent(e: AgentEvent) {
    switch (e.type) {
      case "textDelta":
        updateTurn(agentTurnId, (t) => ({ ...t, text: t.text + e.text }));
        return;
      case "thinking":
        updateTurn(agentTurnId, (t) => ({ ...t, thoughts: (t.thoughts ?? "") + e.text }));
        return;
      case "toolCallStart":
        // Stamped at the START, not the result — see `ms`'s doc. Measuring
        // from here is the whole point: it is the round trip srelens itself
        // observed, not whatever the tool reports server-side.
        callStarts.set(e.id, performance.now());
        updateTurn(agentTurnId, (t) => ({
          ...t,
          calls: [...t.calls, { id: e.id, tool: e.tool, args: e.args, status: null }],
        }));
        return;
      case "toolResult": {
        const startedAt = callStarts.get(e.id);
        callStarts.delete(e.id);
        const ms = startedAt === undefined ? undefined : performance.now() - startedAt;
        updateTurn(agentTurnId, (t) => ({
          ...t,
          calls: t.calls.map((c) => (c.id === e.id ? { ...c, status: e.status, ms } : c)),
        }));
        return;
      }
      case "turnDone":
        // The stream's own end-of-turn marker. Nothing to do with it: `busy`
        // comes down once `sendChat`'s promise itself settles, below, which
        // is the call that actually owns the turn's lifetime.
        return;
      case "error":
        markTurnError(agentTurnId, e.message);
        return;
    }
  }

  try {
    if (!session) session = await startChat();
    // Cold start: `stopAgentRun` was called before this had a session to hand
    // `cancelChat`, and recorded the generation it was asked to stop instead.
    // Honor it now rather than sending a question nobody wants answered.
    if (stoppedGeneration === myGeneration) return;
    const guidance = await loadSkillsGuidance(skills);
    const agents = await listAgents();
    // An agent that is `available` but `gated` must not be offered — mirrors
    // `Composer`'s own filter (Codex and Cursor are installed-but-gated
    // today). Composer reconciles the PICKER against this same set, but the
    // dock never mounts Composer at all (§F), so `agentKind` reaching here
    // can still name a kind nothing in `offered` matches.
    const offered = agents.filter((a) => a.available && !a.gated);
    let resolvedKind = agentKind;
    let agentPath = offered.find((a) => a.kind === agentKind)?.path ?? "";
    if (!agentPath) {
      if (offered.length === 0) {
        // No agent this run could possibly reach — fail the turn rather than
        // handing `sendChat` an empty path it can only fail to spawn. §F's
        // States section: the no-agent case "does not offer a send that
        // cannot work".
        markTurnError(
          agentTurnId,
          new Error(
            "No agent is available to ask. Install Claude, Codex or Cursor, or configure srelens's own agent in Settings, then try again.",
          ),
        );
        return;
      }
      // `agentKind` names nothing installed — fall back to the first agent
      // this run can actually reach, and record the choice via the same
      // field `chooseAgent` writes, so the picker (wherever one is mounted)
      // agrees with what actually ran rather than still pointing at a kind
      // that never sent anything.
      resolvedKind = offered[0].kind;
      agentPath = offered[0].path ?? "";
      if (run.generation === myGeneration) commit({ ...run, agentKind: resolvedKind });
    }
    const result = await sendChat(
      session,
      `${guidance}${question}`,
      agentPath,
      onEvent,
      images,
      resolvedKind,
      myGeneration,
      resume,
    );
    // A later question already moved the conversation on; this answer no
    // longer says anything about where the resume token stands.
    if (run.generation === myGeneration) resume = result;
  } catch (err) {
    // A REJECTION says nothing about `resume` — it is left exactly as it was.
    markTurnError(agentTurnId, err);
  } finally {
    if (run.generation === myGeneration) commit({ ...run, busy: false });
  }
}

/** Stop the turn in flight, if there is one. A Stop that reaches the backend
 *  after that turn already finished, or before its `sendChat` even landed, is
 *  the backend's own business — see `cancelChat`'s doc — and this only ever
 *  asks for the generation the run itself is on right now.
 *
 *  **A turn that hasn't reached `sendChat` yet has no session to hand
 *  `cancelChat`.** `askAgent` commits `busy` and `generation` before it
 *  creates one (cold start: `startChat()` is still spawning the CLI), so a
 *  Stop pressed in that window used to see `!session`, return, and let the
 *  question go out anyway. Recorded as `stoppedGeneration` instead, and
 *  `askAgent` itself honors it once its own `startChat()` resolves — this
 *  takes the run out of `busy` right away so the surface reflects the stop
 *  immediately rather than waiting on that resolution. */
export function stopAgentRun(): void {
  if (!run.busy) return;
  if (!session) {
    stoppedGeneration = run.generation;
    commit({ ...run, busy: false });
    return;
  }
  const activeSession = session;
  const generation = run.generation;
  void cancelChat(activeSession, generation).catch((err: unknown) => {
    commit({ ...run, error: describeError(err).detail });
  });
}

/** Start a fresh conversation: the reader is done with this one, not just
 *  looking away from it. Drops every turn, and the CLI session and resume
 *  token with them, so the next question opens a new session rather than
 *  quietly resuming the one just cleared. A turn still in flight is asked to
 *  stop, best-effort — its own answer, if one still lands, is stale and the
 *  generation check in `askAgent` drops it.
 *
 *  Drops `gates` too. A gate is a row in THIS conversation's transcript, not
 *  a fact independent of it — `Transcript` renders whatever is in `gates`
 *  whenever it is non-empty, with no check that a gate's turn is still among
 *  `turns`. Keeping them past a clear left every gate the reader had ever
 *  answered still drawn under the next, unrelated run, for as long as the
 *  window stayed open.
 *
 *  Drops `activeSkills` too — a skill picked for a run that no longer exists
 *  is not "still active", and this is the one place that is true regardless
 *  of which component (if any) is mounted to have noticed the run end. */
export function clearAgentRun(): void {
  if (run.busy && session) void cancelChat(session, run.generation).catch(() => {});
  session = null;
  resume = null;
  // A no-op — clearing a run that is already idle and empty — is left to
  // `commit`'s own guard rather than special-cased here.
  commit({ ...run, turns: [], gates: [], busy: false, error: undefined, activeSkills: [] });
}

/** Pick which agent CLI the next question is sent to. */
export function chooseAgent(kind: string): void {
  commit({ ...run, agentKind: kind });
}

/**
 * Activate, or deactivate, one skill for the run open right now — the write
 * side of {@link AgentRun.activeSkills}. Idempotent: activating an
 * already-active skill, or deactivating one that is not, commits nothing.
 *
 * Both of this store's writers call this and nothing else: the composer's
 * `/` menu picks a skill on (never off — see its own `removeSkill`, which
 * also calls this), and `RunsRail`'s switch calls it either way from its
 * `on`/`off` change handler.
 */
export function setSkillActive(name: string, active: boolean): void {
  const already = run.activeSkills.includes(name);
  if (active === already) return;
  const activeSkills = active ? [...run.activeSkills, name] : run.activeSkills.filter((n) => n !== name);
  commit({ ...run, activeSkills });
}

/** Record one MCP confirm request's outcome — merged by id, so a request
 *  moving from `pending` to `approved` or `denied` replaces its entry rather
 *  than sitting beside it. */
export function noteGate(record: GateRecord): void {
  const idx = run.gates.findIndex((g) => g.id === record.id);
  const gates = idx === -1 ? [...run.gates, record] : run.gates.map((g, i) => (i === idx ? record : g));
  commit({ ...run, gates });
}

/** Reset the module-level store between tests. */
export function resetAgentRun(): void {
  run = { turns: [], gates: [], busy: false, generation: 0, agentKind: "claude", activeSkills: [] };
  session = null;
  resume = null;
  stoppedGeneration = null;
  turnSeq = 0;
  listeners.clear();
}
