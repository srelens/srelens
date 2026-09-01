import { useCallback, useSyncExternalStore } from "react";
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
import { runKeyFor, runLabelFor, type AskContext } from "./askContext";

/**
 * The one agent run this window is holding — every turn asked and answered,
 * every tool call inside them, and the gates an agent's mutations are waiting
 * on — so the console dock and the `/agent` screen can be two views of the
 * same conversation rather than two copies of it.
 *
 * Shaped after `helmOps.ts` and `sessions.ts`: module-level state, a listener
 * set, `emit()` iterating it directly — the same house style every sibling
 * store here uses (`helmOps.ts`, `sessions.ts`, `peekWidth.ts`,
 * `sectionFolds.ts`), none of which copies the set first either — and a
 * snapshot that keeps its reference until something in it actually changed,
 * so `useSyncExternalStore` has a stable value to compare.
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
  /**
   * `settled` is the state srelens has when the BACKEND says a request stopped
   * waiting and srelens is not the one who answered it — a timeout, an answer
   * given on another surface, or the MCP server going away.
   *
   * It exists because the other three cannot say that honestly. `pending` is a
   * claim the request is still the reader's to answer, which stops being true
   * the moment `mcp://confirm-resolved` arrives. `denied` would put the
   * reader's name on a refusal they did not make. And srelens genuinely does
   * not learn HOW it settled — `mcp://confirm-resolved` carries an id and
   * nothing else — so the state must not imply which.
   */
  outcome: "pending" | "approved" | "denied" | "settled";
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
  /**
   * Non-terminal `error` events from the stream — the backend emits some and
   * then carries on: "image attachments are only supported with the Codex
   * agent" falls straight through (`assistant.rs:541-546`), and a failed
   * image decode or write `continue`s the loop (`:555-580`). Recorded here
   * rather than being allowed to overwrite the turn, because the answer that
   * follows one of these is a real answer.
   *
   * Whether the turn is an ERROR is decided once, when the stream is over: no
   * text and a note means the note is all there was; text means the note is a
   * warning alongside a real reply. Deciding it on arrival is what made a
   * successful, markdown-formatted answer render red with its tool-call rows
   * dropped.
   */
  notes?: string[];
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

/**
 * The window's conversations, one per SUBJECT — keyed by {@link runKeyFor}.
 *
 * **Why this is a map and not one run.** It was one run: the dock and
 * `/agent` were two views of a single conversation, on the reasoning that a
 * question asked from a screen must not vanish from the history it belongs
 * to. That reasoning still holds INSIDE a run. What it got wrong is that a
 * reader on a StatefulSets list and a reader on a pod's logs are not asking
 * about the same thing, and one chat for both meant every question inherited
 * whatever the last one was about. Reported from use, not from review.
 *
 * Empty until the first question: a run is created by asking, never by
 * navigating, so browsing does not litter the rail with conversations nobody
 * had.
 */
const runs = new Map<string, RunState>();

/** Which run the surfaces show by default — the last one asked into. `null`
 *  before any question, when there is nothing to show. */
let activeKey: string | null = null;

/**
 * A run, plus the CLI bookkeeping that belongs to it rather than to the
 * window. `session`, `resume` and `stoppedGeneration` were module-level
 * singletons when there was one conversation; with several they have to be
 * per-run or one run's Stop cancels another's turn and one CLI's `resume`
 * token reaches another conversation.
 */
type RunState = {
  run: AgentRun;
  /** The CLI's own session id for THIS conversation — `null` until its first
   *  turn, and again whenever `sendChat` says to clear it (see its doc). Also
   *  what keeps runs from colliding in the backend, which keys its child
   *  processes by session. */
  session: string | null;
  /** The agent CLI's own conversation id, passed back as `resume` so a
   *  follow-up turn keeps its context. A REJECTED `sendChat` says nothing
   *  about this and leaves it exactly as it was. */
  resume: string | null;
  /**
   * A generation `stopAgentRun` was asked to stop before its `askAgent` had
   * even reached `sendChat` — `session` is created lazily inside `askAgent`,
   * AFTER `busy` and `generation` are already committed, so a Stop arriving
   * in that window finds no session to hand `cancelChat`. Recorded here and
   * honoured by `askAgent` once its own `startChat()` resolves.
   */
  stoppedGeneration: number | null;
  /** What the rail calls this run, and what it is about — pinned when the run
   *  is created, so a later navigation cannot relabel a conversation. */
  label: string;
  /** When it was last asked into — for DISPLAY ("3 minutes ago"). */
  at: number;
  /**
   * Monotonic touch counter, for ORDERING.
   *
   * Not `at`: two questions asked in the same millisecond tie on wall-clock,
   * and a tie makes "most recent" undefined — the rail would order two
   * conversations by whichever happened to be inserted first. Caught by a test
   * that asked twice without an await between.
   */
  order: number;
};

const EMPTY_RUN: AgentRun = {
  turns: [],
  gates: [],
  busy: false,
  generation: 0,
  agentKind: "claude",
  activeSkills: [],
};

/**
 * Which CLI to drive is a preference about srelens, not a property of one
 * conversation, so it lives beside the map rather than inside a run — see
 * ruling AC. Changing it invalidates every run's `resume`, because those
 * tokens belong to the CLI that issued them.
 */
let agentKind = "claude";

/**
 * Skill names to apply to the next question — window-wide, beside
 * {@link agentKind} and for the same reason (ruling AD).
 *
 * It was per-run, from when there was one run. With runs keyed by subject
 * that breaks twice: there is no run to write into before the first question,
 * which is exactly when the rail offers the switch; and the rail sits on
 * `/agent` showing whichever run the reader selected while the dock may be on
 * another, so "which run" has no answer at the moment of picking.
 *
 * Ruling S's requirement is untouched — ONE set, two writers (the composer's
 * `/` menu, the rail's switch), one reader (`askAgent`) — so the two controls
 * still cannot disagree. Only the scope widened.
 */
let activeSkills: string[] = [];

/**
 * What the surfaces read before anyone has asked anything.
 *
 * A CONSTANT would not do: `agentKind` and `activeSkills` are window-wide and
 * a reader can set both before their first question — the picker and the rail's
 * switch are both live then — so a frozen empty run would show neither. Nor
 * can it be derived per read: `useSyncExternalStore` compares snapshots by
 * identity and a fresh object every read re-renders forever. So it is rebuilt
 * only when one of those globals actually changes, and its identity is stable
 * in between.
 */
let emptyRun: AgentRun = { ...EMPTY_RUN };

function refreshEmptyRun() {
  const next: AgentRun = { ...EMPTY_RUN, agentKind, activeSkills };
  if (!sameRun(next, emptyRun)) emptyRun = next;
}
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
/** Monotonic, so recency never ties. See {@link RunState.order}. */
let touchSeq = 0;

const listeners = new Set<() => void>();

/** Bumped on every change, so derived snapshots know when to recompute
 *  without comparing deeply. */
let changeStamp = 0;

function emit() {
  changeStamp += 1;
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
/**
 * The run for a key, created empty if this is its first question.
 *
 * Creation is deliberately here and not in a navigation handler: a run
 * belongs to a conversation, and a conversation starts when someone asks
 * something. Browsing thirty screens must not leave thirty empty chats in the
 * rail.
 */
function runFor(key: string, label: string): RunState {
  const existing = runs.get(key);
  if (existing) return existing;
  const created: RunState = {
    run: { ...EMPTY_RUN, agentKind, activeSkills },
    session: null,
    resume: null,
    stoppedGeneration: null,
    label,
    at: Date.now(),
    order: ++touchSeq,
  };
  runs.set(key, created);
  return created;
}

/** The run the surfaces show — the last one asked into, or an empty stand-in
 *  before any question, so a reader always has something coherent to render. */
function activeState(): RunState | null {
  return activeKey === null ? null : (runs.get(activeKey) ?? null);
}

/**
 * Take a rebuilt run for ONE key, only if something in it actually differs.
 *
 * Identity is the snapshot's contract and every call site rebuilds its slice
 * with a fresh object or array — `updateTurn`'s `{ ...t, text: t.text + e.text }`
 * for a zero-length delta, `noteGate` re-merging a gate whose outcome did not
 * change — none of which change any value a reader could see. Centralized
 * here rather than at each call site: a guard `chooseAgent` remembers and
 * `updateTurn` forgets is no guard at all.
 */
function commitTo(key: string, next: AgentRun) {
  const state = runs.get(key);
  if (!state) return;
  if (sameRun(next, state.run)) return;
  state.run = next;
  emit();
}

/** Kept for the surfaces that only ever mean "the run on screen". */
function commit(next: AgentRun) {
  if (activeKey === null) return;
  commitTo(activeKey, next);
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
      sameStrings(a.notes, b.notes) &&
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
  const state = activeState();
  return state ? state.run : emptyRun;
}

/** One named run, for a surface that shows a conversation other than the
 *  active one — the dock following its own route while `/agent` sits on the
 *  run the rail selected. */
export function getRun(key: string | null): AgentRun {
  if (key === null) return emptyRun;
  return runs.get(key)?.run ?? emptyRun;
}

/** The store, subscribed. */
export function useAgentRun(): AgentRun {
  return useSyncExternalStore(subscribeAgentRun, getAgentRun, getAgentRun);
}

/**
 * One named run, subscribed — what the dock uses, since it shows the run for
 * the route it is on rather than whichever was last asked into.
 *
 * `key` may name a run that does not exist yet (the reader has navigated
 * somewhere they have never asked about); that reads as the empty run, which
 * is exactly right — there is no conversation there yet.
 */
export function useRun(key: string | null): AgentRun {
  const read = useCallback(() => getRun(key), [key]);
  return useSyncExternalStore(subscribeAgentRun, read, read);
}

/** What the rail lists: every conversation this window holds, newest first. */
export type RunSummary = {
  key: string;
  label: string;
  /** For display. Ordering uses {@link RunState.order}, which cannot tie. */
  at: number;
  order: number;
  turns: number;
  busy: boolean;
};

export function getRunSummaries(): RunSummary[] {
  return [...runs.entries()]
    .map(([key, s]) => ({
      key,
      label: s.label,
      at: s.at,
      order: s.order,
      turns: s.run.turns.filter((t) => t.role === "user").length,
      busy: s.run.busy,
    }))
    .sort((a, b) => b.order - a.order);
}

/** The rail's list, subscribed. Rebuilt only when the store emits, so its
 *  identity is stable between changes. */
export function useRunSummaries(): RunSummary[] {
  return useSyncExternalStore(subscribeAgentRun, cachedSummaries, cachedSummaries);
}

let summaryCache: RunSummary[] = [];
let summaryStamp = -1;
/** `useSyncExternalStore` compares by identity, so a fresh array per read
 *  re-renders forever. Recomputed when the store's own change counter moves. */
function cachedSummaries(): RunSummary[] {
  if (summaryStamp !== changeStamp) {
    summaryCache = getRunSummaries();
    summaryStamp = changeStamp;
  }
  return summaryCache;
}

/** Which run the surfaces default to. */
export function getActiveRunKey(): string | null {
  return activeKey;
}

/** Show a different conversation — the rail's switch. */
export function selectRun(key: string): void {
  if (activeKey === key) return;
  // A key with no run yet is ACCEPTED, deliberately. The dock's "full view"
  // control selects its own subject before navigating, and the reader may not
  // have asked about that subject yet — refusing here would open `/agent` on
  // whichever unrelated conversation happened to be active, which is worse
  // than opening it empty on the right subject. `getAgentRun` reads an
  // unknown key as the empty run, and the rail does not list it, because a
  // run still comes into being by asking.
  activeKey = key;
  emit();
}

/** Find, and update, the turn with this id in ONE run — a no-op if the turn
 *  is no longer there (cleared, or superseded), rather than resurrecting it
 *  or emitting a change nothing actually saw. */
function updateTurnIn(key: string, id: number, updater: (t: Turn) => Turn) {
  const state = runs.get(key);
  if (!state) return;
  const idx = state.run.turns.findIndex((t) => t.id === id);
  if (idx === -1) return;
  const turns = state.run.turns.slice();
  turns[idx] = updater(turns[idx]);
  commitTo(key, { ...state.run, turns });
}

/** Turn this turn into the error it ended on, said the way a reader can use
 *  it — never the raw backend string. */
function markTurnErrorIn(key: string, id: number, reason: unknown) {
  updateTurnIn(key, id, (t) => ({ ...t, role: "error", text: describeError(reason).detail }));
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
 * One line telling the agent which cluster it is being asked about.
 *
 * Classic sends this (`AssistantConversation.tsx`'s `contextPreface`) and the
 * new design did not, which left a question like "what is unhealthy right
 * now?" with nothing naming the cluster on screen. Every MCP tool call takes
 * an explicit context, so an agent given none has to guess one — and guessing
 * a cluster is the failure this migration has already spent seven findings
 * preventing on its own surfaces.
 *
 * Sent, never recorded — the same transmitted-vs-recorded split
 * {@link loadSkillsGuidance} follows, so the transcript keeps showing the
 * reader's own words.
 */
function contextPreface(about: AskContext | undefined): string {
  const cluster = about?.cluster.trim() ?? "";
  if (cluster === "") return "";
  let text = `Current context: cluster ${cluster}`;
  if (about?.namespace) text += `, namespace ${about.namespace}`;
  if (about?.kind && about.name) text += `, ${about.kind} ${about.name}`;
  text += ".";
  // Said out loud, because it is the difference between answering the question
  // and going to look for its subject. "Summarise this stream" with only a
  // cluster named sent the agent searching four namespaces for a pod.
  // The reader's own narrowing, when the route has no subject of its own.
  // Said as scope rather than as a fact, because that is what it is: they set
  // the picker, and an agent not told about it sweeps every namespace in the
  // cluster instead.
  const narrowed = about?.namespaces ?? [];
  if (narrowed.length === 1) {
    text += ` The reader has this cluster narrowed to namespace ${narrowed[0]}; unless they say otherwise, that is the scope of the question.`;
  } else if (narrowed.length > 1) {
    text += ` The reader has this cluster narrowed to namespaces ${narrowed.join(", ")}; unless they say otherwise, those are the scope of the question.`;
  }
  if (about?.surface === "logs" && about.name) {
    text += ` The reader is looking at ${about.name}'s logs; a question about "this stream" means that pod's logs.`;
  }
  return `${text}\n\n`;
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
export async function askAgent(
  question: string,
  opts?: {
    images?: string[];
    skills?: string[];
    /**
     * What this question is about — cluster, and the resource on screen when
     * the route names one. Pinned by the CALLER at the gesture rather than
     * read here: both submit paths already hold it, and reading it in this
     * module at dispatch time would reintroduce exactly the read-it-later
     * defect the cluster-identity rule exists to stop.
     *
     * Structured rather than a bare cluster string, because a cluster alone is
     * not a target — see {@link AskContext}.
     */
    about?: AskContext;
    /**
     * The route the question was asked from, which together with `about`
     * decides WHICH conversation it belongs to (see {@link runKeyFor}).
     * Pinned by the caller for the same reason `about` is.
     */
    route?: string;
  },
): Promise<void> {
  // ONE turn at a time, refused here rather than at each door.
  //
  // The dock's input and the composer both disable themselves while busy, but
  // the six `ask()` chips across the app (`Logs`, `Events`, `Helm`,
  // `Overview`, `Terminals`, `DetailActions`) reach `registerSubmit`'s handler
  // and do not. A second turn on the same session is not merely untidy: the
  // backend keys its child processes by session in a `HashMap` and `insert`s
  // (`assistant.rs:727`), so the second send REPLACES the first child handle
  // and drops it without `kill_and_reap` — which that file's own doc
  // (`:285-287`) says leaves a zombie in the process table. The first CLI is
  // then untracked and uncancellable: `chat_cancel` removes by session and
  // finds only the newer child.
  //
  // Said out loud rather than silently swallowed. A chip that looks live and
  // does nothing is the defect this branch keeps finding in other shapes.
  const about = opts?.about ?? { cluster: "" };
  const route = opts?.route ?? "";
  const key = runKeyFor(about, route);

  // Still ONE turn at a time, and across EVERY run rather than per run — see
  // ruling AB. Per-run sessions would make the backend safe for concurrency
  // (`children` is keyed by session, so runs no longer replace each other's
  // child), but gate attribution is what forbids it: `ConfirmRequest` carries
  // no caller (#393), so `AgentConsent` decides ownership from "exactly one
  // run has a turn in flight". Two busy runs and that has nothing to choose
  // between them, and a gate drawn against the wrong conversation is the
  // defect the whole gate design exists to prevent.
  //
  // Said out loud rather than swallowed, and said in the run the reader is
  // ASKING from, which is the one they are looking at.
  const busyElsewhere = [...runs.values()].find((s) => s.run.busy);
  if (busyElsewhere) {
    const state = runFor(key, runLabelFor(about, route));
    activeKey = key;
    commitTo(key, {
      ...state.run,
      error:
        busyElsewhere.label === state.label
          ? "srelens is still answering the last question. Stop it, or wait for it to finish, before asking another."
          : `srelens is still answering a question about ${busyElsewhere.label}. Stop it, or wait for it to finish, before asking another.`,
    });
    return;
  }

  const state = runFor(key, runLabelFor(about, route));
  activeKey = key;
  state.at = Date.now();
  state.order = ++touchSeq;

  const images = opts?.images;
  const skills = opts?.skills ?? activeSkills;
  const userTurn: Turn = { id: ++turnSeq, role: "user", text: question, calls: [], images, at: Date.now() };
  const agentTurnId = ++turnSeq;
  const agentTurn: Turn = { id: agentTurnId, role: "agent", text: "", calls: [], at: Date.now() };
  const myGeneration = state.run.generation + 1;
  const myAgentKind = agentKind;

  commitTo(key, {
    ...state.run,
    turns: [...state.run.turns, userTurn, agentTurn],
    busy: true,
    generation: myGeneration,
    agentKind: myAgentKind,
    error: undefined,
  });
  // A stop recorded against an OLDER generation belongs to a turn already
  // gone; this one hasn't been asked to stop by anyone yet.
  state.stoppedGeneration = null;

  // Keyed by the backend's own call id, and scoped to this one question: two
  // calls from two different questions never share an id, so there is
  // nothing to clean up once this function returns.
  const callStarts = new Map<string, number>();

  function onEvent(e: AgentEvent) {
    switch (e.type) {
      case "textDelta":
        updateTurnIn(key, agentTurnId, (t) => ({ ...t, text: t.text + e.text }));
        return;
      case "thinking":
        updateTurnIn(key, agentTurnId, (t) => ({ ...t, thoughts: (t.thoughts ?? "") + e.text }));
        return;
      case "toolCallStart":
        // Stamped at the START, not the result — see `ms`'s doc. Measuring
        // from here is the whole point: it is the round trip srelens itself
        // observed, not whatever the tool reports server-side.
        callStarts.set(e.id, performance.now());
        updateTurnIn(key, agentTurnId, (t) => ({
          ...t,
          calls: [...t.calls, { id: e.id, tool: e.tool, args: e.args, status: null }],
        }));
        return;
      case "toolResult": {
        const startedAt = callStarts.get(e.id);
        callStarts.delete(e.id);
        const ms = startedAt === undefined ? undefined : performance.now() - startedAt;
        updateTurnIn(key, agentTurnId, (t) => ({
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
        // Not a verdict on the turn — see `Turn.notes`. The stream may well
        // go on to answer the question.
        updateTurnIn(key, agentTurnId, (t) => ({
          ...t,
          notes: [...(t.notes ?? []), describeError(e.message).detail],
        }));
        return;
    }
  }

  try {
    const started = state.session ?? (await startChat());
    // Checked BEFORE `session` is assigned, which is the half that matters.
    // A turn abandoned while `startChat` was still spawning — stopped, or its
    // run cleared — must not reassign the `session` that abandonment nulled on
    // purpose, because reassigning it here is exactly how a discarded question
    // used to go on and reach `sendChat`.
    //
    // Deliberately NOT `run.generation !== myGeneration`. That is true of an
    // ordinary superseded turn too — the reader asked twice in quick
    // succession — and both of those questions were asked on purpose. Only an
    // explicit stop or clear records a generation here, and only those two
    // mean "nobody wants this answered".
    if (state.stoppedGeneration === myGeneration) return;
    state.session = started;
    const preface = contextPreface(about);
    const guidance = await loadSkillsGuidance(skills);
    const agents = await listAgents();
    // The FIRST fix here guarded only the `startChat` window and was too
    // narrow: `loadSkillsGuidance` and `listAgents` are two more awaits, and a
    // clear landing in THAT window left this turn walking on to dispatch. It
    // also read the mutable global `session` at the call below, so the
    // discarded question either went out with `null` or — once the reader had
    // asked again — went out under the NEW question's session. Hence `started`,
    // captured above, and the recheck immediately before dispatch.
    // An agent that is `available` but `gated` must not be offered — mirrors
    // `Composer`'s own filter. Nothing is gated today: core documents `gated`
    // as "currently always `false` for every kind" (`chat.ts:25-29`) and
    // `is_gated` returns `false` unconditionally (`assistant.rs:28-30`), so
    // this half of the filter is a guard against a future gating, not a
    // branch anything currently reaches.
    //
    // Composer reconciles the PICKER against this same set, but the dock never
    // mounts Composer at all (§F), so `agentKind` reaching here can still name
    // a kind nothing in `offered` matches.
    const offered = agents.filter((a) => a.available && !a.gated);
    let resolvedKind = myAgentKind;
    let agentPath = offered.find((a) => a.kind === myAgentKind)?.path ?? "";
    if (!agentPath) {
      if (offered.length === 0) {
        // No agent this run could possibly reach — fail the turn rather than
        // handing `sendChat` an empty path it can only fail to spawn. §F's
        // States section: the no-agent case "does not offer a send that
        // cannot work".
        markTurnErrorIn(
          key,
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
      // The other agent's conversation does not transfer. `chooseAgent` drops
      // `resume` when the READER switches; this is the same switch made
      // automatically — the previous agent left `PATH`, say — and it was
      // leaving a Claude conversation id to be handed to Codex's `--resume`.
      // Closing the manual door and leaving the automatic one open is not
      // closing it.
      //
      // `session` is kept, unlike in `chooseAgent`: it is srelens's own id for
      // the turn about to be sent, no child process exists under it yet, and
      // dropping it here would strand the send this branch is preparing.
      state.resume = null;
      if (state.run.generation === myGeneration) commitTo(key, { ...state.run, agentKind: resolvedKind });
    }
    // Last thing before the question actually leaves. Every await above is a
    // window in which the reader can abandon this turn.
    if (state.stoppedGeneration === myGeneration) return;
    const result = await sendChat(
      started,
      `${preface}${guidance}${question}`,
      agentPath,
      onEvent,
      images,
      resolvedKind,
      myGeneration,
      state.resume,
    );
    // A later question already moved this conversation on; this answer no
    // longer says anything about where its resume token stands.
    if (state.run.generation === myGeneration) state.resume = result;
  } catch (err) {
    // A REJECTION says nothing about `resume` — it is left exactly as it was.
    markTurnErrorIn(key, agentTurnId, err);
  } finally {
    if (state.run.generation === myGeneration) {
      // The stream is over: now a note can be judged. Nothing said and a note
      // recorded means the note was the whole story, so the turn IS its
      // error. Text alongside it means the answer arrived and the note is a
      // warning the reader should still see.
      updateTurnIn(key, agentTurnId, (t) =>
        t.role === "agent" && t.text === "" && (t.notes?.length ?? 0) > 0
          ? { ...t, role: "error", text: t.notes!.join("\n"), notes: undefined }
          : t,
      );
      commitTo(key, { ...state.run, busy: false });
    }
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
  // The BUSY run, not the visible one. A reader can navigate away from a
  // streaming conversation and press Stop on the surface still showing it;
  // resolving "which run" by what is on screen would then cancel nothing, or
  // worse, the wrong thing. Only one run is ever busy (ruling AB), so this is
  // unambiguous.
  const entry = [...runs.entries()].find(([, st]) => st.run.busy);
  if (!entry) return;
  const [key, state] = entry;
  if (!state.session) {
    state.stoppedGeneration = state.run.generation;
    commitTo(key, { ...state.run, busy: false });
    return;
  }
  const activeSession = state.session;
  const generation = state.run.generation;
  void cancelChat(activeSession, generation).catch((err: unknown) => {
    commitTo(key, { ...state.run, error: describeError(err).detail });
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
export function clearAgentRun(target?: string): void {
  // A gesture about ONE conversation — the caller's own, since the dock and
  // `/agent` can be showing different runs. Defaults to the active run for
  // `/agent`'s header control; the dock passes its own route's key.
  const key = target ?? activeKey;
  if (key === null) return;
  const state = runs.get(key);
  if (!state) return;
  // Cancel with the generation the in-flight turn was SENT with, before the
  // bump below moves it — the backend matches a Stop against that.
  if (state.run.busy && state.session) {
    void cancelChat(state.session, state.run.generation).catch(() => {});
  }
  state.session = null;
  state.resume = null;
  // The generation is what makes the clear actually stick. `askAgent`'s
  // post-await guards are all `state.run.generation === myGeneration`, so
  // leaving it alone left a discarded turn still looking current: a pending
  // `startChat()` would resolve, reassign the `session` this just nulled, and
  // send the question the reader had thrown away; a running `sendChat()` would
  // repopulate `resume` after this cleared it. Bumping it fails every one of
  // those guards at once.
  //
  // Also recorded as a stop, for the window where the discarded turn has no
  // session yet and so was never handed to `cancelChat` at all.
  if (state.run.busy) state.stoppedGeneration = state.run.generation;
  // A no-op — clearing a run that is already idle and empty — is left to
  // `commitTo`'s own guard rather than special-cased here.
  commitTo(key, {
    ...state.run,
    turns: [],
    gates: [],
    busy: false,
    error: undefined,
    generation: state.run.busy ? state.run.generation + 1 : state.run.generation,
  });
}

/**
 * Forget a conversation entirely — the rail's own close, distinct from
 * `clearAgentRun`, which empties the run the reader is in but keeps it.
 *
 * Refused while it is streaming: dropping the state a turn is writing into
 * would leave its `sendChat` with nowhere to land and its child process
 * untracked, which is the shape of the round-2 P1.
 */
export function forgetRun(key: string): void {
  const state = runs.get(key);
  if (!state || state.run.busy) return;
  runs.delete(key);
  if (activeKey === key) {
    // Fall back to the most recent survivor rather than to nothing, so the
    // surfaces have a conversation to show.
    const next = getRunSummaries()[0];
    activeKey = next ? next.key : null;
  }
  emit();
}

/**
 * Pick which agent CLI the next question is sent to.
 *
 * **Drops the CLI conversation with it.** `session` and `resume` are the
 * PREVIOUS agent's, and they do not transfer: `resume` is that CLI's own
 * conversation id, so handing a Claude session id to Cursor's `--resume` asks
 * it to continue something it has never heard of, and switching away and back
 * would resume the older conversation as though the turns in between never
 * happened. The transcript stays — it is srelens's own record, and the reader
 * asked those questions — but the next question opens a fresh CLI session.
 *
 * Only on an actual change. `Composer`'s reconciliation effect calls this with
 * whatever it can offer, and a no-op call must not quietly end the
 * conversation the reader is in the middle of.
 */
export function chooseAgent(kind: string): void {
  if (kind === agentKind) return;
  // Not while a turn is in flight. Dropping `session` here would leave
  // `stopAgentRun` with nothing to hand `cancelChat` — the running CLI becomes
  // uncancellable — and the turn's own completion would then write its
  // `resume` token back under the newly chosen agent, which is the very
  // mixing this function exists to prevent. The picker is disabled while busy
  // for the same reason; this is the invariant behind that, held where every
  // caller passes rather than in the one component that happens to render a
  // picker today.
  const busy = [...runs.entries()].find(([, st]) => st.run.busy);
  if (busy) {
    const [busyKey, busyState] = busy;
    commitTo(busyKey, {
      ...busyState.run,
      error: "srelens is still answering. Stop the question in flight before switching agent.",
    });
    return;
  }
  agentKind = kind;
  refreshEmptyRun();
  // EVERY run's CLI conversation, not just the visible one — ruling AC.
  // `resume` belongs to the CLI that issued it, so leaving the other runs
  // holding tokens the new agent cannot use is the defect rounds 2 and 7 each
  // closed once. The transcripts stay: they are srelens's own record.
  for (const [key, state] of runs) {
    state.session = null;
    state.resume = null;
    commitTo(key, { ...state.run, agentKind: kind });
  }
  emit();
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
  const already = activeSkills.includes(name);
  if (active === already) return;
  activeSkills = active ? [...activeSkills, name] : activeSkills.filter((n) => n !== name);
  refreshEmptyRun();
  // Mirrored onto every run so `AgentRun.activeSkills` stays the one thing
  // readers render, rather than components learning a second accessor.
  for (const [key, state] of runs) commitTo(key, { ...state.run, activeSkills });
  emit();
}

/** Record one MCP confirm request's outcome — merged by id, so a request
 *  moving from `pending` to `approved` or `denied` replaces its entry rather
 *  than sitting beside it. */
export function noteGate(record: GateRecord): void {
  // The BUSY run owns the gate, not the visible one. A confirm arrives because
  // some agent called a tool, and that agent is the one with a turn in flight —
  // which the reader may well have navigated away from. Attributing by what is
  // on screen would draw another conversation's mutation into this one, which
  // is the defect the gate design exists to prevent.
  //
  // Exactly one run is ever busy (ruling AB), so this is unambiguous. No busy
  // run means srelens's own agent did not cause it — an external MCP client
  // did — and nothing is recorded, which is the #393 case.
  const entry = [...runs.entries()].find(([, st]) => st.run.busy);
  if (!entry) return;
  const [key, state] = entry;
  const idx = state.run.gates.findIndex((g) => g.id === record.id);
  const gates =
    idx === -1 ? [...state.run.gates, record] : state.run.gates.map((g, i) => (i === idx ? record : g));
  commitTo(key, { ...state.run, gates });
}

/** Which run holds a gate, so `AgentConsent` can settle the one it recorded
 *  rather than guessing. `null` when no run owns it. */
export function runKeyHoldingGate(id: string): string | null {
  for (const [key, state] of runs) {
    if (state.run.gates.some((g) => g.id === id)) return key;
  }
  return null;
}

/** Settle a gate in the run that owns it, wherever that is. */
export function noteGateIn(key: string, record: GateRecord): void {
  const state = runs.get(key);
  if (!state) return;
  const idx = state.run.gates.findIndex((g) => g.id === record.id);
  if (idx === -1) return;
  const gates = state.run.gates.map((g, i) => (i === idx ? record : g));
  commitTo(key, { ...state.run, gates });
}

/**
 * Put away a run-level failure the reader has read.
 *
 * `error` is the store's one channel for something that happened to the RUN
 * rather than to a turn — a refused submission, a `cancelChat` that did not
 * land — and it is rendered by both views. Without a way to dismiss it, the
 * sentence would sit there until the next question happened to clear it.
 */
export function dismissAgentError(target?: string): void {
  const key = target ?? activeKey;
  if (key === null) return;
  const state = runs.get(key);
  if (!state || state.run.error === undefined) return;
  commitTo(key, { ...state.run, error: undefined });
}

/** Reset the module-level store between tests. */
export function resetAgentRun(): void {
  runs.clear();
  activeKey = null;
  agentKind = "claude";
  activeSkills = [];
  touchSeq = 0;
  summaryStamp = -1;
  emptyRun = { ...EMPTY_RUN };
  turnSeq = 0;
  listeners.clear();
}
