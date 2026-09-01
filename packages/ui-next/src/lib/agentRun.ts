import { useCallback, useSyncExternalStore } from "react";
import {
  cancelChat,
  describeError,
  listAgents,
  loadSkill,
  listSessions,
  loadSession,
  saveSession,
  deleteSession,
  sendChat,
  startChat,
  type AgentEvent,
  type Session,
  type SessionMeta,
  type Skill,
  type ToolStatus,
} from "@srelens/core";
import { runKeyFor, runLabelFor, type AskContext } from "./askContext";
import { newId } from "./tabs";
import { titleFromQuestion } from "./runTitle";
import { stripDataUri } from "./pastedImages";

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
  /**
   * False when `at` is the CONVERSATION's timestamp rather than this turn's.
   *
   * Classic's stored messages carry no time of their own (`StoredMessage` is
   * id/role/text/toolCalls/images/thoughts), so a conversation restored from
   * one knows when it was last touched and not when each turn happened. The
   * clock is withheld rather than printing the same borrowed stamp under every
   * turn, which would be srelens claiming a time it was never told.
   */
  atRecorded?: boolean;
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
  /**
   * What this conversation is ABOUT, and the route it was asked from — kept so
   * a follow-up can continue it from a surface that is not that route.
   *
   * The full view is the case that needs it. There the dock shows whichever
   * conversation is selected, but it submitted with `route === "/agent"`, so
   * `askAgent` recomputed the destination as `<cluster>|/agent` and every
   * follow-up typed under a pod's transcript started a SEPARATE run — losing
   * that conversation's CLI resume with it.
   *
   * Absent for a conversation restored from a file written before this was
   * recorded; the dock falls back to its own route then, which is the old
   * behaviour rather than a crash.
   */
  subject?: { about: AskContext; route: string };
  /** What the rail calls this run, and what it is about — pinned when the run
   *  is created, so a later navigation cannot relabel a conversation. */
  label: string;
  /**
   * The id this run is saved under, stable for its lifetime.
   *
   * Not the run key: the key is derived from a subject and could in principle
   * be recomputed differently, and a saved conversation must not lose its file
   * because a route parser changed. Generated once, carried in the saved
   * envelope, and reused on rehydration.
   */
  id: string;
  /**
   * Disk writes for THIS run, serialised.
   *
   * Classic learned this the hard way (`AssistantConversation`'s
   * `persistChainRef`): a save still flushing when the next one starts can
   * land out of order, and a save racing a delete recreates a file that was
   * just removed. One chain per run, awaited by anything that touches the same
   * file.
   */
  saving: Promise<void>;
  /**
   * True when this run's `error` is only the "still answering" refusal — a
   * message about a condition somewhere ELSE, which stops being true the
   * moment that turn finishes.
   *
   * It used to sit there afterwards: the reader saw "srelens is still
   * answering the last question" above an answer that had visibly completed.
   * A failed `cancelChat` is a different thing and must NOT be cleared this
   * way, which is why this is a flag rather than a string comparison.
   */
  refusalOnly: boolean;
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

/**
 * Which saved-conversation load is the current one.
 *
 * Two clicks before the first `loadSession` returns and BOTH continuations
 * assigned `activeKey`, so whichever request happened to resolve last won — a
 * slow first load switched the transcript back after the reader had already
 * opened the second. Bumped by `openSavedRun` on entry AND by `selectRun`,
 * since an explicit selection is the reader saying which conversation they
 * want; compared before anything is applied.
 *
 * Declared here with the rest of the module's state rather than beside
 * `openSavedRun`: `selectRun` reads it 900 lines earlier, and a `let` used
 * above its own declaration is a temporal dead zone waiting for the first
 * caller that runs during module evaluation.
 */
let openSeq = 0;
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
    id: newRunId(),
    saving: Promise.resolve(),
    refusalOnly: false,
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
  /** What the conversation is ABOUT, when that is known — the subject key's
   *  own label. Absent for a row still on disk, whose envelope has not been
   *  read yet. */
  subject?: string;
  turns: number;
  busy: boolean;
  /** Set when this row is a conversation on disk that is not loaded yet — the
   *  rail opens it with {@link openSavedRun} rather than {@link selectRun}. */
  savedId?: string;
};

export function getRunSummaries(): RunSummary[] {
  const live: RunSummary[] = [...runs.entries()]
    // Nothing asked, nothing to list. A run can exist with no turns — one
    // created to carry a refusal, or one selected and then cleared — and a row
    // reading "0 questions" is a conversation the reader never had.
    .filter(([, s]) => s.run.turns.length > 0)
    .map(([key, s]) => ({
    key,
    // The QUESTION, like the saved rows — one naming scheme, not two. Live
    // rows used the subject ("cluster", "Pod/mongodb-0") while saved rows used
    // the question, so one conversation listed twice read as two unrelated
    // things, and a subject like "cluster" said nothing on its own.
    // Derived from the question rather than the raw text: the raw one
    // truncated mid-word in the rail and carried whatever opener the reader
    // typed.
    label: titleFromQuestion(s.run.turns.find((t) => t.role === "user")?.text ?? "") || s.label,
    // The subject travels alongside, since it is what the run is ABOUT and the
    // question alone does not always say.
    subject: s.label,
    at: s.at,
    order: s.order,
    turns: s.run.turns.filter((t) => t.role === "user").length,
    busy: s.run.busy,
    savedId: undefined,
  }));
  // Conversations on disk that this window has not opened yet. Listed so a
  // restart does not look like a fresh install, and marked with `savedId` so
  // the rail knows a click has to LOAD one rather than just switch to it.
  //
  // Deduped against the live runs BY FILE ID. A run asked in this window is
  // written to disk immediately, so the next `listSessions` sees it — and
  // without this it appeared twice: once as the live conversation and once as
  // its own saved copy, under two different names. Reported from use.
  const liveIds = new Set([...runs.values()].map((s) => s.id));
  const onDisk: RunSummary[] = saved
    .filter((m) => !liveIds.has(m.id))
    .map((m) => ({
      key: `saved|${m.id}`,
      label: m.title,
      at: m.updatedAt,
      // Behind every live run: those are this session's, and the reader was
      // just in them. Negative, so no live run ever sorts below a saved one.
      order: -1,
      turns: 0,
      busy: false,
      savedId: m.id,
    }));
  return [...live.sort((a, b) => b.order - a.order), ...onDisk.sort((a, b) => b.at - a.at)];
}

/** The rail's list, subscribed. Rebuilt only when the store emits, so its
 *  identity is stable between changes. */
/**
 * What the conversation at `key` is about, as a subscription.
 *
 * {@link getRunSubject} is the same read for an event handler. The full view
 * needs it during RENDER as well — to name the cluster the next question will
 * actually reach — and a plain module read there would not re-render when the
 * reader selects a different conversation.
 */
export function useRunSubject(key: string | null): { about: AskContext; route: string } | undefined {
  const read = useCallback(() => getRunSubject(key), [key]);
  return useSyncExternalStore(subscribeAgentRun, read, read);
}

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
/**
 * What the conversation at `key` is about, when it is known.
 *
 * For the full-view composer, which continues the SELECTED conversation and
 * cannot derive its subject from its own route — that route is `/agent`, which
 * is not a subject at all.
 */
export function getRunSubject(key: string | null): { about: AskContext; route: string } | undefined {
  return key === null ? undefined : runs.get(key)?.subject;
}

export function getActiveRunKey(): string | null {
  return activeKey;
}

/** The same, subscribed — the dock needs to re-render when the rail switches
 *  conversations, since on `/agent` it follows the active one. */
export function useActiveRunKey(): string | null {
  return useSyncExternalStore(subscribeAgentRun, getActiveRunKey, getActiveRunKey);
}

/** Show a different conversation — the rail's switch. */
export function selectRun(key: string | null): void {
  if (key === null) return;
  // BEFORE the early return below, deliberately. Any saved-conversation load
  // still in flight is stale the moment the reader says which conversation
  // they want — and the commonest way to say it is clicking the one already
  // active, to get back to it while a slow load spins. Placed after the guard,
  // that click invalidated nothing and the load still switched the transcript
  // away when it landed.
  openSeq += 1;
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
  // What the reader is looking at, when it is a list rather than one resource:
  // the kind alone, since that is all a list has. Said because the agent
  // cannot see the tab — "pass kind type like which tab is opened".
  //
  // `!about.name` matters: a kind WITH a name is the subject itself and is
  // already stated above as `Pod mongodb-0`. Saying "looking at the Pod list"
  // beside it would be srelens describing a screen the reader is not on.
  if (about?.kind && !about.name) {
    text += ` The reader is looking at the ${about.kind} list.`;
  }
  // The narrowing the tab carries. Said as SCOPE rather than as a fact,
  // because that is what it is: the reader set the picker, and an agent not
  // told about it sweeps every namespace in the cluster instead.
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
    /**
     * The conversation this question belongs to, named outright.
     *
     * `about` + `route` normally decide it (see {@link runKeyFor}), and for
     * every screen's own composer that is right — the surface the reader is on
     * IS the subject. The full view is not: it shows whichever conversation is
     * selected, and its key cannot always be reconstructed.
     *
     * Two cases where re-deriving reached the WRONG run. A conversation opened
     * beside a live one about the same subject is aliased to `saved|<id>` while
     * still carrying that subject, so recomputing landed on the live run and a
     * follow-up joined the wrong transcript. And a dock expanded before its
     * first question has no stored subject at all, so the fallback produced
     * `<cluster>|/agent`.
     *
     * When given, this is used verbatim. `about` still supplies the preface,
     * so a follow-up carries the resource the conversation is about.
     */
    key?: string;
  },
): Promise<boolean> {
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
  // The caller's own key wins. Derivation is a convenience for the surfaces
  // that ARE their subject; a caller holding the identity has better
  // information than anything recomputed from a route.
  const key = opts?.key ?? runKeyFor(about, route);

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
    state.refusalOnly = true;
    commitTo(key, {
      ...state.run,
      error:
        busyElsewhere.label === state.label
          ? "srelens is still answering the last question. Stop it, or wait for it to finish, before asking another."
          : `srelens is still answering a question about ${busyElsewhere.label}. Stop it, or wait for it to finish, before asking another.`,
    });
    // NOT taken. The composer clears its draft on `true` only: it clears
    // unconditionally otherwise, so a question refused here was discarded
    // along with any screenshot attached to it while the alert said it had not
    // been sent — leaving the reader to retype what they could still see a
    // moment ago.
    return false;
  }

  const state = runFor(key, runLabelFor(about, route));
  activeKey = key;
  state.at = Date.now();
  state.order = ++touchSeq;
  // What this conversation is about, so a follow-up asked from the full view
  // reaches THIS run rather than one keyed by `/agent`.
  state.subject = { about, route };

  const images = opts?.images;
  const skills = opts?.skills ?? activeSkills;
  const userTurn: Turn = { id: ++turnSeq, role: "user", text: question, calls: [], images, at: Date.now() };
  const agentTurnId = ++turnSeq;
  const agentTurn: Turn = { id: agentTurnId, role: "agent", text: "", calls: [], at: Date.now() };
  const myGeneration = state.run.generation + 1;
  // The RUN's agent, not the module's. They are the same for a fresh run
  // (`runFor` seeds it from the module value), and they differ for a
  // conversation reopened from disk: `openSavedRun` restores that file's kind
  // and its resume token, and reading the module value here handed a Codex
  // conversation's token to Claude while the picker — which shows
  // `run.agentKind` — still said Codex.
  const myAgentKind = state.run.agentKind;

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
  // Saved with the question already in it, before anything is awaited. A
  // conversation interrupted mid-answer then still holds what the reader
  // asked, which is the half they cannot reconstruct.
  persistRun(key);

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
    // `true`: the question WAS taken — its turn is in the transcript — it was
    // merely abandoned before it could be sent.
    if (abandoned(state, myGeneration)) return true;
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
        // Taken, and then failed ON its own turn — which is where the reader
        // reads about it. The draft is right to clear: the question is on
        // screen in the transcript, not lost.
        return true;
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
    if (abandoned(state, myGeneration)) return true;
    const result = await sendChat(
      started,
      `${preface}${guidance}${question}`,
      agentPath,
      onEvent,
      // Raw base64, not the data URIs the turn records: `chat_send` passes
      // these to `decode_base64_image`, which is `STANDARD.decode` and fails on
      // a `data:` prefix. Stripped here, at the send, so every caller can hold
      // the displayable form.
      images?.map(stripDataUri),
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
      // Stamped NOW, not when the question was submitted: the transcript draws
      // this clock beneath the finished answer, so an answer that streamed for
      // minutes was labelled with the moment the reader pressed Enter. The user
      // turn keeps its own submission time, which is what that one means.
      //
      // Folded into the SAME commit as `busy: false` rather than its own
      // `updateTurnIn`. One settle is one change, and a separate write here was
      // a second notification immediately before this one — two renders for one
      // event, which this store's own emit-count test exists to catch.
      const settledAt = Date.now();
      const settled = state.run.turns.map((t) =>
        t.id !== agentTurnId
          ? t
          : t.role === "agent" && t.text === "" && (t.notes?.length ?? 0) > 0
            ? // The stream is over: now a note can be judged. Nothing said and
              // a note recorded means the note was the whole story, so the turn
              // IS its error. Text alongside it means the answer arrived and
              // the note is a warning the reader should still see.
              { ...t, role: "error" as const, text: t.notes!.join("\n"), notes: undefined, at: settledAt }
            : { ...t, at: settledAt },
      );
      commitTo(key, { ...state.run, turns: settled, busy: false });
      // The condition every refusal was about — a turn in flight — is over, so
      // the sentences saying so stop being true. Cleared here rather than left
      // for the reader to dismiss: a stale message about a current problem is
      // the class of defect this branch exists to remove.
      // Whether anything was REMOVED rather than committed. `commitTo` emits;
      // a delete does not, and neither does reassigning `activeKey` — so
      // without the emit below, `useAgentRun`, `useActiveRunKey` and `useRun`
      // kept showing the refusal alert or a blank run until some unrelated
      // store update happened to wake them.
      let removed = false;
      for (const [k, st] of runs) {
        if (!st.refusalOnly) continue;
        st.refusalOnly = false;
        // A run that exists ONLY to have carried a refusal is not a
        // conversation: nothing was ever asked in it. `askAgent` has to create
        // it to have somewhere to put the message, and leaving it behind
        // listed a newest "0 questions" row in Recent runs and opened `/agent`
        // on a blank transcript. It goes with the message.
        if (st.run.turns.length === 0) {
          runs.delete(k);
          // The conversation the reader is actually watching — the one whose
          // turn just finished — rather than a key that no longer resolves.
          if (activeKey === k) activeKey = key;
          removed = true;
          continue;
        }
        commitTo(k, { ...st.run, error: undefined });
      }
      if (removed) emit();

      // Every turn, not only the last: a window closed mid-conversation must
      // not lose the answers already given, and "save on exit" has no hook to
      // hang on in a Tauri window the reader can kill.
      persistRun(key);
    }
  }
  // Taken. Every other exit above says so too; only the busy refusal does not.
  return true;
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
    /*
      Only if this is still the turn that was cancelled, AND it is still in
      flight.

      The generation half catches a rejection landing after the reader cleared
      the run or asked something else: that spread whatever `run` was current,
      so a NEW conversation was told "That question was not sent" for a Stop
      belonging to the one before it.

      The busy half catches normal completion, which the generation cannot:
      nothing bumps it when a turn simply finishes, so a `cancelChat` rejecting
      after the answer arrived put "That question was not sent" above a question
      that visibly finished. A Stop that lost a race with the answer it was
      trying to stop has nothing to report.
    */
    if (state.run.generation !== generation || !state.run.busy) return;
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
export function clearAgentRun(target?: string | null): void {
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
  // The conversation is over, so its file goes with it — AFTER whatever write
  // is still in flight. Classic's own comment on this: a save still flushing
  // when the delete lands recreates the file and its index entry, and the
  // reader's "New question" quietly un-deletes what they just cleared.
  //
  // A fresh id, so the next question in this run writes a new file rather than
  // reusing the one just removed.
  const dead = state.id;
  // Off the not-yet-loaded list BEFORE the id rotates. `getRunSummaries` hides
  // a persisted file only while its id belongs to a live run, so rotating the
  // live id while `saved` still held the old one made the conversation the
  // reader just cleared reappear immediately as a saved row — openable until
  // the delete landed, and a load error afterwards.
  saved = saved.filter((m) => m.id !== dead);
  state.saving = state.saving.then(() => deleteSession(dead)).catch(() => {});
  state.id = newRunId();
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
  // Same ordering as a clear: drain the write, then remove the file. And the
  // same removal from the not-yet-loaded list, for the same reason — a run
  // dropped here while `saved` still held its id came straight back as a saved
  // row. Fixed as a class rather than at the one site that was reported.
  const dead = state.id;
  saved = saved.filter((m) => m.id !== dead);
  void state.saving.then(() => deleteSession(dead)).catch(() => {});
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
export function chooseAgent(kind: string, shownIn?: string | null): void {
  /*
    The run whose PICKER was used, as well as the window's preference.

    A conversation reopened from disk carries its own agent and `askAgent` asks
    THAT one, so the two legitimately differ. Comparing only the module value
    made the visible act of picking Claude a no-op that left the other CLI's
    resume token in place.

    `shownIn` matters because the dock is keyed by its own route, which is not
    always the ACTIVE run: off `/agent` the picker renders `useRun(runKey)`, so
    a reader could be looking at a restored Codex conversation while `activeKey`
    points at a Claude one. Falling back to the active run keeps every caller
    that has no key to give — the `/` menu, tests — working as before.
  */
  const shown = (shownIn !== undefined && shownIn !== null ? runs.get(shownIn) : activeState())?.run
    .agentKind;
  if (kind === agentKind && (shown === undefined || shown === kind)) return;
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
    busyState.refusalOnly = true;
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
  // A gate is part of the record decision 1 traded the second set of buttons
  // for, so it has to survive a restart like the turns do.
  persistRun(key);
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
  persistRun(key);
}

/**
 * Put away a run-level failure the reader has read.
 *
 * `error` is the store's one channel for something that happened to the RUN
 * rather than to a turn — a refused submission, a `cancelChat` that did not
 * land — and it is rendered by both views. Without a way to dismiss it, the
 * sentence would sit there until the next question happened to clear it.
 */
export function dismissAgentError(target?: string | null): void {
  const key = target ?? activeKey;
  if (key === null) return;
  const state = runs.get(key);
  if (!state || state.run.error === undefined) return;
  state.refusalOnly = false;
  commitTo(key, { ...state.run, error: undefined });
}

/** `newId`, reused: it already guards `crypto.randomUUID` for a non-secure
 *  context, and a second generator would be a second thing to get wrong. */
function newRunId(): string {
  return newId();
}

/**
 * What a run looks like on disk.
 *
 * `Session.messages` is `unknown[]` and documented as opaque to the backend —
 * "the frontend owns its shape" — so the whole run travels as ONE envelope
 * element rather than being smeared across fields that mean other things.
 * `contexts` is which clusters a conversation touched; abusing it to carry a
 * run key would be the kind of thing that reads fine and breaks later.
 *
 * Versioned from the first write. A run saved by this build must still be
 * readable by the next one, and the alternative to a version is guessing.
 */
type SavedRun = {
  v: 1;
  /** The subject key, so a restored run merges with the live one for the same
   *  thing rather than sitting beside it as a duplicate. */
  key: string;
  label: string;
  turns: Turn[];
  gates: GateRecord[];
  /**
   * What the conversation is about, so a follow-up typed in the full view
   * reaches it after a restart too.
   *
   * Optional: files written before this existed have none, and a run restored
   * without it falls back to the composer's own route — the old behaviour, not
   * a crash. `v` stays `1` for the same reason: nothing about the old shape
   * became unreadable.
   */
  subject?: { about: AskContext; route: string };
};

/** One recorded tool call, checked before the transcript reads it. */
function isSavedCall(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const c = value as { id?: unknown; tool?: unknown };
  return typeof c.id === "string" && typeof c.tool === "string";
}

/**
 * One recorded turn.
 *
 * `calls` is required and must be an ARRAY, because `Transcript` reads
 * `turn.calls.length` — a truncated or hand-edited file whose turn lacks it
 * took the agent screen down rather than being reported as unreadable.
 */
function isSavedTurn(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const t = value as { id?: unknown; role?: unknown; text?: unknown; calls?: unknown; at?: unknown };
  return (
    typeof t.id === "number" &&
    (t.role === "user" || t.role === "agent" || t.role === "error") &&
    typeof t.text === "string" &&
    typeof t.at === "number" &&
    Array.isArray(t.calls) &&
    t.calls.every(isSavedCall)
  );
}

/** One recorded gate. `gates` is optional; a present one must be usable. */
function isSavedGate(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const g = value as { id?: unknown; tool?: unknown };
  return typeof g.id === "string" && typeof g.tool === "string";
}

/**
 * Whether a stored message is one of THIS build's run envelopes.
 *
 * Every turn, call and gate is checked, not just the shape around them. A file
 * can be truncated mid-write, hand-edited, or written by a future build, and
 * the transcript reads `turn.calls.length` and `gates.map` the moment it
 * renders — so a structurally-plausible envelope with malformed contents used
 * to take the agent screen down instead of being reported as unreadable. A
 * session that fails here falls through to the classic reader, and then to an
 * empty transcript, rather than to a crash.
 */
function isSavedRun(value: unknown): value is SavedRun {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Partial<SavedRun> & { gates?: unknown };
  return (
    v.v === 1 &&
    typeof v.key === "string" &&
    typeof v.label === "string" &&
    Array.isArray(v.turns) &&
    v.turns.every(isSavedTurn) &&
    (v.gates === undefined || (Array.isArray(v.gates) && v.gates.every(isSavedGate)))
  );
}

/** One of classic's stored tool calls (`StoredToolCall`), checked before it is
 *  drawn: a call with no capability name renders a blank row in the
 *  transcript, which reads as a call srelens made and cannot name. */
type ClassicCall = { id: string; tool: string; args: unknown; status: ToolStatus | null };

function isClassicCall(value: unknown): value is ClassicCall {
  if (typeof value !== "object" || value === null) return false;
  const v = value as { id?: unknown; tool?: unknown; status?: unknown };
  return (
    typeof v.id === "string" &&
    typeof v.tool === "string" &&
    v.tool !== "" &&
    (v.status === null ||
      v.status === undefined ||
      v.status === "ok" ||
      v.status === "error" ||
      v.status === "denied")
  );
}

/**
 * One of classic's stored messages, as `chatHistory.ts` documents it —
 * `StoredMessage`, structurally checked rather than trusted.
 *
 * The predicate claims ONLY what it verifies. `StoredMessage.id` is real on
 * disk but never read here (a restored turn takes a fresh `turnSeq`), so
 * asserting a `number` this function never looked at would be a claim the
 * check does not back.
 */
type ClassicMessage = {
  role: "user" | "assistant" | "error";
  text: string;
  toolCalls?: unknown[];
  images?: string[];
  thoughts?: string;
};

function isStoredMessage(value: unknown): value is ClassicMessage {
  if (typeof value !== "object" || value === null) return false;
  const v = value as { role?: unknown; text?: unknown; images?: unknown; thoughts?: unknown };
  return (
    typeof v.text === "string" &&
    (v.role === "user" || v.role === "assistant" || v.role === "error") &&
    (v.images === undefined || (Array.isArray(v.images) && v.images.every((i) => typeof i === "string"))) &&
    (v.thoughts === undefined || typeof v.thoughts === "string")
  );
}

/**
 * A conversation classic saved, read into this design's turns.
 *
 * These are the reader's own conversations — the ones the rail has been listing
 * as `saved · 14d ago` all along — and opening one showed NOTHING, because
 * `openSavedRun` looked only for this build's own envelope. The comment there
 * said classic's transcript was "not ours to interpret", which was wrong: the
 * shape is documented in `chatHistory.ts` and maps onto a `Turn` almost
 * exactly.
 *
 * What does not map is TIME. `StoredMessage` carries none, so every turn takes
 * the conversation's `createdAt` and is marked `atRecorded: false` — see
 * {@link Turn.atRecorded}. `createdAt` rather than `updatedAt` because the run
 * head reads the FIRST turn's stamp as `started <time>`, and the conversation
 * genuinely did start then; `updatedAt` would print the last touch as the
 * start. What is genuinely lost is per-call duration, which classic never
 * stored either.
 */
function turnsFromClassic(messages: readonly unknown[], at: number): Turn[] {
  return messages.filter(isStoredMessage).map((m) => ({
    id: ++turnSeq,
    role: m.role === "assistant" ? ("agent" as const) : m.role,
    text: m.text,
    calls: (Array.isArray(m.toolCalls) ? m.toolCalls : []).filter(isClassicCall).map((c) => ({
      id: c.id,
      tool: c.tool,
      args: c.args,
      status: c.status ?? null,
      // Classic stored no duration, and an absent reading renders no reading.
      ms: undefined,
    })),
    thoughts: m.thoughts,
    images: m.images,
    at,
    atRecorded: false,
  }));
}

/**
 * Whether the turn that started at `generation` has been abandoned — so a
 * continuation resolving after an await must not go on to send.
 *
 * **Two halves, and both are needed.**
 *
 * `stoppedGeneration` catches a Stop or a clear with nothing asked after it:
 * `stopAgentRun` leaves the generation where it is when the turn has no
 * session yet, so currency alone would let that turn through.
 *
 * Currency catches the case the marker cannot. `askAgent` resets
 * `stoppedGeneration` to null for the turn it is starting — a stop recorded
 * against an older turn does not belong to this one — and that reset ERASED
 * the only marker the abandoned turn had. Press Stop before `startChat`
 * resolves, ask again immediately, and the discarded question passed both
 * guards and went out alongside the replacement, on a session Stop could no
 * longer reach. A single `number | null` cannot hold two abandoned
 * generations either, so the marker was never enough on its own.
 *
 * Currency is safe here BECAUSE `askAgent` refuses while any run is busy
 * (ruling AB): two turns are never legitimately in flight, so a generation
 * that is no longer current was abandoned rather than merely overtaken.
 */
function abandoned(state: RunState, generation: number): boolean {
  return state.stoppedGeneration === generation || state.run.generation !== generation;
}

/** Sessions on disk that are not (yet) loaded into `runs` — the rail lists
 *  them so a reader's history survives a restart, and one is hydrated only
 *  when they open it. */
let saved: SessionMeta[] = [];

/**
 * Write one run to disk, behind its own chain.
 *
 * Best-effort and deliberately silent: a failed write must not turn into an
 * error over a conversation that is otherwise fine, and the reader has not
 * asked for anything here. It is not silent about being best-effort — see
 * {@link restoreRuns}, which is where a reader learns their history could not
 * be read.
 */
function persistRun(key: string): void {
  const state = runs.get(key);
  if (!state) return;
  // Nothing worth a file until something was actually asked.
  const asked = state.run.turns.find((t) => t.role === "user");
  if (!asked) return;
  const envelope: SavedRun = {
    v: 1,
    key,
    label: state.label,
    turns: state.run.turns,
    gates: state.run.gates,
    ...(state.subject ? { subject: state.subject } : {}),
  };
  const session: Session = {
    id: state.id,
    // What a reader recognises in a list: the question they asked. The subject
    // is already the rail's label, and repeating it here would make every
    // session in the picker read the same.
    // The same derivation the rail uses, so a conversation reads the same
    // whether it is live or restored from disk. `slice(0, 120)` cut mid-word.
    title: titleFromQuestion(asked.text) || asked.text.slice(0, 120),
    createdAt: state.run.turns[0]?.at ?? state.at,
    updatedAt: state.at,
    contexts: [],
    skills: state.run.activeSkills,
    cliSessionId: state.resume,
    agentKind: state.run.agentKind,
    messages: [envelope],
  };
  state.saving = state.saving.then(() => saveSession(session)).catch(() => {});
}

/**
 * Read the saved conversations back, so a restart does not lose them.
 *
 * Only the metadata: `listSessions` is documented as cheap for exactly this
 * reason, and loading every transcript to draw a list would read a megabyte to
 * show ten titles. A conversation is hydrated when it is opened.
 */
export async function restoreRuns(): Promise<void> {
  const list = await listSessions();
  saved = list;
  emit();
}

/** Open a saved conversation: load its transcript, put it in the map under its
 *  own subject key, and show it. */
export async function openSavedRun(id: string): Promise<void> {
  const mine = ++openSeq;
  const meta = saved.find((m) => m.id === id);
  const session = await loadSession(id);
  // A later click has taken over. Nothing is applied — not the run, not
  // `activeKey`, and not the removal from the not-yet-loaded list, because a
  // conversation this call is abandoning must stay listed.
  if (mine !== openSeq) return;
  const envelope = session.messages.find(isSavedRun);
  // Where this conversation WOULD live: the subject it was saved under, or its
  // own file if it was written by a shape this build has no key for.
  const subjectKey = envelope?.key ?? `saved|${id}`;
  const holder = runs.get(subjectKey);
  /*
    A different live conversation already holds that subject — the reader asked
    about this pod after a restart, then opened the older saved conversation
    about it — so this one gets an identity of its own.

    Reusing that run replaced its turns and gates while KEEPING its id, resume
    token, save chain and busy state. If it was streaming, the deltas arriving
    afterwards could no longer find the agent turn they belonged to, and the
    saved transcript was then persisted under the live conversation's file. Two
    conversations about one subject is a real thing; one conversation wearing
    another's history is not.

    Compared by FILE id, not by identity: the same file already open at that
    key IS this conversation, and hydrating it again is exactly right.
  */
  const key = holder && holder.id !== session.id ? `saved|${id}` : subjectKey;
  const existing = runs.get(key);
  const state: RunState = existing ?? {
    run: { ...EMPTY_RUN, agentKind: session.agentKind ?? agentKind, activeSkills },
    session: null,
    resume: session.cliSessionId,
    stoppedGeneration: null,
    id: session.id,
    saving: Promise.resolve(),
    refusalOnly: false,
    label: envelope?.label ?? meta?.title ?? "saved",
    at: session.updatedAt,
    order: ++touchSeq,
  };
  runs.set(key, state);
  if (existing) {
    // Reopening a conversation already loaded: its own file, so its resume
    // token and agent are the ones on disk. Kept in step with `run.agentKind`
    // below, which `askAgent` now reads.
    state.resume = session.cliSessionId;
  }
  // What it is about, so a follow-up typed in the full view continues THIS
  // conversation rather than opening one keyed by `/agent`.
  if (envelope?.subject) state.subject = envelope.subject;
  if (envelope) {
    state.run = {
      ...state.run,
      turns: envelope.turns,
      gates: envelope.gates ?? [],
      agentKind: session.agentKind ?? state.run.agentKind,
    };
  } else {
    // No envelope of ours: classic wrote this one. Its transcript is readable
    // and it is the reader's own conversation, so it opens rather than opening
    // empty.
    state.run = {
      ...state.run,
      turns: turnsFromClassic(session.messages, session.createdAt),
      gates: [],
      agentKind: session.agentKind ?? state.run.agentKind,
    };
  }
  activeKey = key;
  // Off the not-yet-loaded list: it is a live run now, and showing it twice
  // would be the duplicate the rail's own history bug already taught.
  saved = saved.filter((m) => m.id !== id);
  emit();
}

/** Reset the module-level store between tests. */
export function resetAgentRun(): void {
  runs.clear();
  activeKey = null;
  agentKind = "claude";
  activeSkills = [];
  saved = [];
  touchSeq = 0;
  summaryStamp = -1;
  emptyRun = { ...EMPTY_RUN };
  turnSeq = 0;
  listeners.clear();
}
