import { useEffect, useState } from "react";
import { ageFromTimestamp } from "@srelens/core";
import { Button, EmptyState, Section, StatusPill, toneColor, toneWash, type StatusKind } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";
import type { SessionState, TerminalSessionRow } from "../../lib/sessions";

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

function SessionRow({
  session,
  active,
  now,
  onSelect,
}: {
  session: TerminalSessionRow;
  active: boolean;
  now: number;
  onSelect: () => void;
}) {
  const verdict = SESSION_VERDICT[session.state];
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={active || undefined}
      className="flex w-full items-center gap-1.5 rounded px-1 py-1.5 text-left"
      style={{ background: active ? toneWash("accent") : undefined }}
    >
      <Icons.terminal
        size={14}
        aria-hidden="true"
        className="shrink-0"
        style={{ color: active ? toneColor("accent") : toneColor("muted") }}
      />
      <span className="min-w-0 flex-1 truncate text-[0.8125rem] font-medium">{session.title}</span>
      <span className="shrink-0 text-[0.75rem] text-muted">
        {session.kind} · {idleFor(session.lastOutputAt, now)}
      </span>
      <StatusPill status={verdict.word} kind={verdict.kind} tinted />
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
  /** "New session" was picked, from the empty state. */
  onNewSession: () => void;
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
export function SessionRail({ sessions, activeId, onSelect, onNewSession }: SessionRailProps) {
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
          <Button size="xs" onClick={onNewSession}>
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
        />
      ))}
    </Section>
  );
}
