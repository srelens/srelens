import { useEffect, useRef, useState, type ReactNode, type Ref } from "react";
import { ageFromTimestamp } from "@srelens/core";
import { Button, EmptyState, IconButton, Section, StatusPill, toneColor, toneWash, type StatusKind } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";
import type { SessionKind, SessionState, TerminalSessionRow } from "../../lib/sessions";

/**
 * §14's rail width, and this screen's alone — see `SideRail`'s note on why the
 * width is a number per screen rather than a scale. Exported so the screen and
 * this file cannot hold two different answers to the same question.
 */
export const SESSION_RAIL_WIDTH = 230;

/**
 * §14's rail head. A pure function of the rows rather than something this
 * component prints itself: the head sits in `SideRail`'s own `head` slot,
 * which the screen that mounts this rail owns, and a head computed in two
 * places is two places that can disagree about what "attached" counts. This
 * is the one place it is counted.
 *
 * Counts `attached` only — an `idle` session is still running but has not
 * said anything lately, and §14's own worked example counts it out (four
 * sessions, one idle, one closed, and the head still reads "2 attached").
 */
export function sessionRailHead(sessions: readonly TerminalSessionRow[]): string {
  const attached = sessions.filter((s) => s.state === "attached").length;
  return `Sessions · ${attached} attached`;
}

/**
 * The word and tone this rail draws for each session state.
 *
 * THE ONE HAND-PAIRED WORD/TONE TABLE IN THIS FILE, AND IT IS MARKED BECAUSE
 * IT SHOULD NOT HAVE TO EXIST. `../../lib/sessions.ts` says so on
 * `SessionState` itself: core has a verdict for a Kubernetes resource
 * (`k8sStatus`/`k8sHealth`) and for a log stream's connection
 * (`logConnectionStatus`), and neither speaks about a shell session. Until
 * core grows a `sessionStatus`, the pairing lives here — shaped the way
 * `Toolbox.tsx`'s `TOOL_VERDICT` and `Forwards.tsx`'s `FORWARD_VERDICT`
 * already are. **If a `sessionStatus` is ever added to `packages/core`,
 * delete this and call it** — and do not add a second copy of it, here or
 * anywhere else in this file.
 *
 * The severities follow §14's own dot colours: `attached` is the healthy
 * state and reads ok; `idle` is still a running shell that has gone quiet,
 * which is worth a glance rather than an alarm, so it reads warn rather than
 * danger; `closed` is neither good nor bad, just over, so it reads neutral —
 * the "faint" the design asks for is `StatusPill`'s own untinted plain text
 * for a state `BAD` does not cover, not a fourth tone invented for it.
 */
export const SESSION_VERDICT: Record<SessionState, { word: string; kind: StatusKind }> = {
  attached: { word: "Attached", kind: "success" },
  idle: { word: "Idle", kind: "warning" },
  closed: { word: "Closed", kind: "neutral" },
};

/**
 * §14's own prose for what a session IS — `pod exec`, `node shell`, `local`.
 *
 * **THIS IS NOT THE ELEVENTH HAND-PAIRED TABLE, and the difference is the
 * tone.** The rule this migration keeps — the one that removed ten tables —
 * is that every status WORD AND ITS SEVERITY comes from core: each of those
 * ten paired a state with a meaning (healthy, degraded, failing), and a second
 * copy of such a pairing is how a red dot ends up beside an amber word. A kind
 * pairs with nothing. It says which of three things a session is, exactly the
 * way a column header says what a column holds; there is no verdict in it, no
 * tone to disagree with, and nothing core could ever be asked to decide —
 * `SessionKind` is this store's own union, declared next to the emulator it
 * belongs to. {@link SESSION_VERDICT} above is the table that carries tone,
 * and it is the only one in this file.
 *
 * It exists at all because rendering the raw union member put `pod` on a row
 * §14 writes as `pod exec` — and "pod" beside a pod's own name says nothing,
 * where "pod exec" says what kind of shell the reader is looking at.
 */
export const SESSION_KIND_LABEL: Record<SessionKind, string> = {
  pod: "pod exec",
  node: "node shell",
  local: "local",
};

/** How often the idle time recomputes. Same resolution `Forwards`' Age column
 *  ticks at, and for the same reason: a screen of live sessions is where a
 *  frozen idle time is a lie the reader would act on. */
const IDLE_TICK_MS = 1_000;

/** "12s", "4m", "1h" — core's own compact age words, read off how long ago
 *  this session last said anything. Not a duration this file invents: the
 *  same `ageFromTimestamp` the Forwards screen's Age column already ticks. */
function idleFor(lastOutputAt: number, now: number): string {
  return ageFromTimestamp(new Date(lastOutputAt).toISOString(), now);
}

/**
 * The name, being typed over. Enter or leaving the field keeps what was typed;
 * Escape keeps what was there. A blank name keeps what was there too — the
 * store would refuse it, and the field closing on a name that then did not
 * change is the same answer said once.
 */
function RenameField({
  title,
  onDone,
}: {
  title: string;
  /** `next` is the name to keep, or `null` to keep the old one. `byKey` says
   *  the field was closed from the keyboard, with focus still in it. */
  onDone: (next: string | null, byKey: boolean) => void;
}) {
  const [draft, setDraft] = useState(title);
  // Escape blurs the field as it unmounts; without this the blur would keep
  // the very text the reader had just abandoned.
  const settled = useRef(false);
  const finish = (next: string | null, byKey: boolean) => {
    if (settled.current) return;
    settled.current = true;
    onDone(next, byKey);
  };
  return (
    <input
      aria-label={`Rename ${title}`}
      value={draft}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => finish(draft, false)}
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== "Escape") return;
        // An input method is still composing: this Enter picks a candidate
        // and this Escape drops one. Neither is about the field.
        if (e.nativeEvent.isComposing) return;
        // Finishing hands focus back to the row's button within this same
        // keystroke; left to its default, the rest of the keystroke would
        // then press that button and select a row the reader only renamed.
        e.preventDefault();
        finish(e.key === "Enter" ? draft : null, true);
      }}
      className="min-w-0 flex-1 rounded border border-rule bg-transparent px-1 py-0.5 text-[0.8125rem] font-medium outline-none focus:border-[var(--accent)]"
    />
  );
}

function SessionRow({
  session,
  active,
  now,
  onSelect,
  onRename,
  onDetach,
}: {
  session: TerminalSessionRow;
  active: boolean;
  now: number;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDetach: () => void;
}) {
  const verdict = SESSION_VERDICT[session.state];
  const [renaming, setRenaming] = useState(false);
  const selector = useRef<HTMLButtonElement>(null);
  // Back to the row once the field has gone, so the keyboard is where it was
  // rather than on the document body. After the render that brings the row's
  // button back — while renaming there is no such button to focus. Only when
  // the field was closed from the keyboard: a reader who left it by Tab or by
  // clicking something has already said where focus goes, and it stays there.
  const refocus = useRef(false);
  useEffect(() => {
    if (!refocus.current) return;
    refocus.current = false;
    selector.current?.focus();
  });

  const body = (
    <span className="flex items-center gap-1.5">
      <span className="truncate text-[0.75rem] text-muted">
        {SESSION_KIND_LABEL[session.kind]} · {idleFor(session.lastOutputAt, now)}
      </span>
      <StatusPill status={verdict.word} kind={verdict.kind} tinted />
    </span>
  );
  const icon = (
    <Icons.terminal
      size={14}
      aria-hidden="true"
      className="mt-0.5 shrink-0"
      style={{ color: active ? toneColor("accent") : toneColor("muted") }}
    />
  );

  // The row is a group of three controls, not one button holding two more: a
  // button inside a button is invalid, and a reader on a keyboard could reach
  // neither of the inner ones.
  return (
    <div
      data-slot="session-row"
      className="flex w-full items-start rounded"
      style={{ background: active ? toneWash("accent") : undefined }}
    >
      {renaming ? (
        <span className="flex min-w-0 flex-1 items-start gap-1.5 px-1 py-1.5">
          {icon}
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <RenameField
              title={session.title}
              onDone={(next, byKey) => {
                refocus.current = byKey;
                setRenaming(false);
                if (next !== null) onRename(next);
              }}
            />
            {body}
          </span>
        </span>
      ) : (
        <SessionSelect
          ref={selector}
          session={session}
          active={active}
          icon={icon}
          body={body}
          onSelect={onSelect}
          onRename={() => setRenaming(true)}
        />
      )}
      {!renaming && (
        <span className="flex shrink-0 items-center py-1 pr-0.5">
          <IconButton
            icon={Icons.edit}
            label={`Rename ${session.title}`}
            onClick={() => setRenaming(true)}
          />
          {/* The pane's own Detach, reachable for a session that is not the
              one on screen: closing the third of five no longer means
              selecting it first. Same act, same store call, and like that one
              it asks nothing — a row is the reader's to dismiss. */}
          <IconButton icon={Icons.close} label={`Detach ${session.title}`} danger onClick={onDetach} />
        </span>
      )}
    </div>
  );
}

function SessionSelect({
  ref,
  session,
  active,
  icon,
  body,
  onSelect,
  onRename,
}: {
  ref: Ref<HTMLButtonElement>;
  session: TerminalSessionRow;
  active: boolean;
  icon: ReactNode;
  body: ReactNode;
  onSelect: () => void;
  onRename: () => void;
}) {
  return (
    <button
      ref={ref}
      type="button"
      onClick={onSelect}
      // The two ways a name is edited everywhere else a reader has met one.
      onDoubleClick={onRename}
      onKeyDown={(e) => {
        if (e.key === "F2") onRename();
      }}
      aria-current={active || undefined}
      className="flex min-w-0 flex-1 items-start gap-1.5 rounded px-1 py-1.5 text-left"
    >
      {icon}
      {/* Stacked, because the rail is 230px and the row carries four things.
          Laid out on one line the name got 44px of 229 — two pods from the
          same namespace were told apart by four characters, which is no
          telling apart at all. §14 draws these stacked and the widths are
          why. `min-w-0` as well as `truncate`: a flex item's implicit
          `min-width: auto` refuses to shrink below its content, so a long pod
          name would widen the column rather than ellipsing. */}
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-[0.8125rem] font-medium">{session.title}</span>
        {body}
      </span>
    </button>
  );
}

export interface SessionRailProps {
  /** Every session this window knows about, running and closed, in the
   *  order the store carries them — the store appends, so this is start
   *  order and the rail draws it as given rather than re-sorting it. */
  sessions: readonly TerminalSessionRow[];
  /** The session the terminal pane is showing, or none. */
  activeId: number | null;
  /** A row was picked — make it the active session. */
  onSelect: (id: number) => void;
  /** A row's name was typed over — already trimmed of nothing; the store decides what a name is. */
  onRename: (id: number, title: string) => void;
  /** A row's Detach was pressed — end that session, whichever one is on screen. */
  onDetach: (id: number) => void;
  /** "New session" was picked, from the empty state. */
  onNewSession: () => void;
  /** The screen is still usable, but the active cluster cannot start a shell. */
  newSessionDisabled?: boolean;
}

/**
 * §14's left rail: every terminal session this window is holding open, and
 * which one the pane is showing.
 *
 * **This is rail CONTENT, not the rail's frame.** Like `StreamRail` before it,
 * it renders no `aside` and claims no width of its own beyond the
 * {@link SESSION_RAIL_WIDTH} it exports — the screen wraps it in `SideRail`,
 * heads it with {@link sessionRailHead}, and hands this component nothing but
 * props. That is what "standalone" buys here: this file has no opinion about
 * the terminal pane beside it, and nothing it does can go wrong from a screen
 * that has not been written yet.
 *
 * **A closed session stays listed.** §349's lesson, arriving again: a vanished
 * row is how a reader ends up assuming a dead session is fine. It draws with
 * the same shape as any other row — its own {@link SESSION_VERDICT}, `Closed`,
 * read plainly rather than dropped from the list.
 *
 * **The idle time ticks.** `lastOutputAt` is rounded to the second in the
 * store precisely so a ticking display here does not churn its snapshot —
 * this component owns the clock the store deliberately does not.
 */
export function SessionRail({ sessions, activeId, onSelect, onRename, onDetach, onNewSession, newSessionDisabled = false }: SessionRailProps) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), IDLE_TICK_MS);
    return () => clearInterval(tick);
  }, []);

  if (sessions.length === 0) {
    return (
      <EmptyState
        compact
        title="No sessions"
        hint="Open a shell into a pod, a node, or this machine."
        action={
          <Button size="xs" onClick={onNewSession} disabled={newSessionDisabled}>
            New session
          </Button>
        }
      />
    );
  }

  return (
    <Section padded={false}>
      {sessions.map((session) => (
        <SessionRow
          key={session.id}
          session={session}
          active={session.id === activeId}
          now={now}
          onSelect={() => onSelect(session.id)}
          onRename={(title) => onRename(session.id, title)}
          onDetach={() => onDetach(session.id)}
        />
      ))}
    </Section>
  );
}
