import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { ITheme } from "@xterm/xterm";
import {
  Alert,
  AskChip,
  Badge,
  Button,
  EmptyState,
  Screen,
  SideRail,
  statusTone,
} from "@srelens/ui-kit";
import { useConsole } from "../console";
import { useActiveContext } from "../lib/clusters";
import {
  endSession,
  getSessions,
  subscribeSessions,
  terminalFor,
  type TerminalSessionRow,
} from "../lib/sessions";
import { NewSessionMenu } from "./terminals/NewSessionMenu";
import {
  SESSION_RAIL_WIDTH,
  SESSION_VERDICT,
  SessionRail,
  sessionRailHead,
} from "./terminals/SessionRail";
import { TerminalView } from "./terminals/TerminalView";

/**
 * §14's footer eyebrow, verbatim.
 *
 * **Do not soften this and do not move it into documentation.** Every other
 * write path in srelens is gated — a confirm dialog with the kubectl
 * equivalent printed in it, for a delete, a drain, a scale. A shell is
 * deliberately not: srelens cannot tell `ls` from `rm -rf /` inside a PTY, and
 * pretending to would be a gate that gates nothing. Saying so where the shell
 * IS, rather than in a document nobody has open at the moment it matters, is
 * the honest place for it.
 */
const NOT_GATED =
  "Destructive commands inside a shell are not gated — the shell is your own session";

/**
 * The design tokens the emulator is dressed from.
 *
 * xterm paints its own canvas and knows nothing about this app's stylesheet,
 * so unlike every other surface here it cannot simply inherit. Left alone it
 * draws its own dark palette in a white pane, in a font that matches no other
 * monospace surface in the app — which is what a terminal that reads as
 * foreign looks like.
 *
 * **The token NAMES are here; the values are not.** They are read off the
 * document at {@link terminalDress}, so a theme change, a palette change or a
 * token edit carries into the transcript without this file knowing any colour.
 * That is the same rule `toneColor` follows for everything drawn in CSS; this
 * is the one place that has to resolve a token to a value rather than hand the
 * browser a `var()`, because a canvas cannot take one.
 *
 * The ANSI half maps only the four colours the token set has an honest answer
 * for — a program's `red` means "bad" and so does `--sev`. `cyan` is left to
 * xterm's own palette: there is no token that means cyan, and pointing it at
 * `--info` would be inventing a meaning for someone else's output. The black
 * and white ends come from the ink ramp rather than literal black and white,
 * which is what keeps a program's `white` legible on a light theme — the one
 * place a terminal's defaults are actively wrong here.
 */
const TERMINAL_TOKENS: Readonly<Partial<Record<keyof ITheme, string>>> = {
  // §14 draws the transcript as a SUNK surface, not the pane's own.
  background: "--surface-sunk",
  foreground: "--ink-soft",
  // §14: "a pulsing accent block cursor sits on the last line".
  cursor: "--accent",
  cursorAccent: "--surface-sunk",
  selectionBackground: "--accent-wash",
  black: "--ink",
  red: "--sev",
  green: "--ok",
  yellow: "--warn",
  blue: "--info",
  magenta: "--accent",
  white: "--ink-muted",
  brightBlack: "--ink-faint",
  brightRed: "--sev",
  brightGreen: "--ok",
  brightYellow: "--warn",
  brightBlue: "--info",
  brightMagenta: "--accent",
  brightWhite: "--ink",
};

/** The token the app's monospace surfaces already share. */
const FONT_TOKEN = "--font-mono";

/**
 * Resolve {@link TERMINAL_TOKENS} against the document root.
 *
 * The root rather than the pane, deliberately: `:root` is where the token
 * block is declared and `data-theme` is where the mode is written
 * (`applyNextDesignTheme` in `apps/desktop/src/design.ts`), so this reads the
 * same element both of them speak about.
 *
 * A token that resolves to nothing is left out rather than written as an empty
 * string — xterm treats `""` as a colour and throws parsing it, and a document
 * with no stylesheet attached (a unit test, a first paint) is a real state.
 * Left out, xterm keeps its own default for that one key.
 */
function terminalDress(root: Element): { theme: ITheme; fontFamily?: string } {
  const style = getComputedStyle(root);
  const read = (token: string) => style.getPropertyValue(token).trim();
  const theme: Record<string, string> = {};
  for (const [key, token] of Object.entries(TERMINAL_TOKENS)) {
    const value = read(token);
    if (value) theme[key] = value;
  }
  const fontFamily = read(FONT_TOKEN);
  return { theme, fontFamily: fontFamily || undefined };
}

/**
 * Keep the session on screen wearing the app's tokens.
 *
 * Re-run when the theme attribute changes, because that is a theme switch:
 * the tokens under it now resolve to different values and the emulator has
 * already read the old ones. Nothing else observes `data-theme` — the rest of
 * the app is CSS and follows on its own — which is exactly why the one canvas
 * in the app has to.
 */
function useTokenDress(sessionId: number | null): void {
  useEffect(() => {
    if (sessionId === null) return undefined;
    const root = document.documentElement;
    const dress = () => {
      const term = terminalFor(sessionId);
      if (!term) return;
      const { theme, fontFamily } = terminalDress(root);
      term.options.theme = theme;
      if (fontFamily) term.options.fontFamily = fontFamily;
    };
    dress();
    const observer = new MutationObserver(dress);
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, [sessionId]);
}

/**
 * §14's Ask chip, asked about the shell that is actually open.
 *
 * §14's own question — "Write the kubectl command to check the pool size in
 * this pod" — is mock content: "this pod" names nothing an agent could act on,
 * and "pool size" is the mock's example workload. The chip's WORD is §14's;
 * the question carries the subject on screen instead, which is the same
 * treatment every other ask chip in this app gets.
 */
function askQuestion(session: TerminalSessionRow | undefined, context: string): string {
  if (!session) {
    return `Draft a kubectl command to run against ${context || "the current cluster"}`;
  }
  if (session.kind === "local") {
    return `Draft a kubectl command to run against ${session.context}`;
  }
  const where = session.namespace ? ` in namespace ${session.namespace}` : "";
  return `Draft a shell command to run inside ${session.title}${where}`;
}

/**
 * `/terminals` — §14's session rail and the live shell one of them is.
 *
 * **The sessions are not this screen's.** They live in `lib/sessions.ts`,
 * module-level, with their emulators; this screen subscribes, draws whichever
 * one is selected, and is otherwise free to unmount without taking a shell
 * with it. That is why `Detach` calls `endSession` rather than doing anything
 * itself: ending a session closes the far end, disposes the emulator and —
 * for a node shell — deletes the privileged debug pod srelens put on the
 * cluster. A row removal that merely looked like a detach would leave that pod
 * running.
 *
 * **A `closed` session is still selectable and still shows its transcript.**
 * That is why it stays in the rail at all (#349's vanishing port-forward): the
 * scrollback is what the reader came back for, and the row's `error` is why it
 * ended. `Clear` and `Detach` both still make sense on one — clearing a
 * transcript is not ending anything, and detaching is how the row leaves.
 *
 * **The active session is an id, never a position.** The store's array
 * shortens whenever a row is detached and grows whenever one is started, from
 * this screen or from the resource row menu — and a pane keyed on `[0]` or on
 * the last index would follow that shuffle instead of following the reader.
 */
export function Terminals(_props: { route: string }) {
  const sessions = useSyncExternalStore(subscribeSessions, getSessions, getSessions);
  const cluster = useActiveContext();
  const { ask } = useConsole();
  const [picked, setPicked] = useState<number | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);

  /**
   * A session started anywhere — this screen's menu, or the resource row
   * menu's `Open shell`, which starts one and then opens this tab — becomes
   * the one on screen. Ids are the store's own monotonic counter, so "the last
   * row" is "the newest session", and following it is what makes `Open shell`
   * land on the shell it just opened rather than on whatever was already here.
   *
   * Held in a ref rather than compared against `picked`, so it fires once per
   * new session: a reader who then selects an older row keeps it.
   */
  const newest = sessions.length > 0 ? sessions[sessions.length - 1].id : null;
  const followed = useRef<number | null>(null);
  useEffect(() => {
    if (newest === null || newest === followed.current) return;
    followed.current = newest;
    setPicked(newest);
  }, [newest]);

  // The selection, resolved against the rows that actually exist. A detached
  // session leaves an id behind that names nothing; the newest row is what a
  // reader is most likely to want next, and it is also what the effect above
  // would have chosen.
  const active =
    sessions.find((s) => s.id === picked) ??
    (sessions.length > 0 ? sessions[sessions.length - 1] : undefined);
  useTokenDress(active?.id ?? null);

  const context = active?.context ?? cluster?.name ?? "";
  const namespace = active?.namespace ?? cluster?.namespace ?? "";
  const verdict = active ? SESSION_VERDICT[active.state] : undefined;

  return (
    <Screen
      title="Terminals"
      // §14's sub, `prod-eu / checkout`, read off the session on screen rather
      // than off the cluster rail: a shell keeps talking to the cluster it was
      // opened in, and the rail's selection can move away from it.
      eyebrow={[context, namespace].filter(Boolean).join(" / ")}
      fill
      actions={
        <>
          {/* A `Button`, not the row's `AskChip`, for the reason `Events.tsx`
              and `Overview.tsx` both give: `.row-ask` is `opacity: 0` until a
              `.tbl tbody tr` is hovered, which is right for one of forty rows
              and invisible in a header, where there is no row to hover. The
              visible word is the design's; the question it will actually send
              goes in the accessible name — the same split the chip makes. */}
          <Button
            type="button"
            size="sm"
            aria-label={`Draft a command: ${askQuestion(active, context)}`}
            onClick={() => ask(askQuestion(active, context))}
          >
            Draft a command
          </Button>
          <Button variant="primary" size="sm" onClick={() => setMenuOpen(true)}>
            New session
          </Button>
        </>
      }
    >
      {menuOpen && (
        <NewSessionMenu
          // The cluster in focus, not the active session's: a new session is
          // started where the reader is, and the menu says so itself when
          // there is no cluster rather than opening on an empty context.
          context={cluster?.name ?? ""}
          namespace={cluster?.namespace}
          onStarted={setPicked}
          onClose={() => setMenuOpen(false)}
        />
      )}
      <SideRail
        head={sessionRailHead(sessions)}
        width={SESSION_RAIL_WIDTH}
        rail={
          <SessionRail
            sessions={sessions}
            activeId={active?.id ?? null}
            onSelect={setPicked}
            onNewSession={() => setMenuOpen(true)}
          />
        }
        mainHead={
          active && verdict ? (
            <>
              {/* §14 asks for the name in normal case, against the uppercase
                  `.pane-head` wears everywhere else — a pod name is an
                  identifier the reader will compare against `kubectl get pods`,
                  and uppercasing it makes it a different string on sight. */}
              <span
                data-slot="session-name"
                className="min-w-0 truncate normal-case tracking-normal text-[0.75rem] text-ink"
              >
                {active.title}
              </span>
              <span className="flex-1" />
              {/* The rail's own verdict, not a second reading of the same
                  state: `Attached` in the head and `Idle` in the rail, on one
                  session, would be two answers to one question. */}
              <Badge tone={statusTone(verdict.kind)}>{verdict.word}</Badge>
            </>
          ) : undefined
        }
      >
        {active ? (
          <div className="flex min-h-0 flex-1 flex-col">
            {active.error && (
              <Alert tone="sev" title="This session ended" className="m-3 mb-0">
                {/* Already through `describeError` — the store describes a
                    session's reason once, where it arrives, so a sentence is
                    never classified twice on its own wording. */}
                {active.error}
              </Alert>
            )}
            {/* Keyed on the session, so switching pulls the whole pane down and
                puts the next emulator's element in a fresh container. Without
                it the old element stays where it was appended and two
                transcripts stack in one pane. */}
            <TerminalView key={active.id} sessionId={active.id} />
            <div className="flex shrink-0 items-center gap-2 border-t border-rule bg-sunk px-2.5 py-1.5">
              {/* Not an `Eyebrow`: that voice is 10px tracked uppercase mono,
                  which is right over a figure and unreadable across a
                  seventy-character sentence. The words are §14's, verbatim;
                  the voice is the one that can actually be read at the moment
                  it matters. */}
              <span className="min-w-0 text-[0.75rem] text-muted">{NOT_GATED}</span>
              <span className="flex-1" />
              {/* Clears the TRANSCRIPT. The session keeps running — there is
                  no `Clear` that also detaches, and a reader tidying the
                  screen must not lose their shell to it. */}
              <Button variant="secondary" size="xs" onClick={() => terminalFor(active.id)?.clear()}>
                Clear
              </Button>
              <Button variant="danger" size="xs" onClick={() => endSession(active.id)}>
                Detach
              </Button>
            </div>
          </div>
        ) : (
          <EmptyState
            title="No shell open"
            hint="Start a session to open a shell inside a pod, on a node, or on this machine. Sessions keep running while you are on another tab."
            // `fill` hands the body the whole area and leaves the centring to
            // whatever is in it; without this the state sits at the top edge.
            className="flex-1"
          />
        )}
      </SideRail>
    </Screen>
  );
}
