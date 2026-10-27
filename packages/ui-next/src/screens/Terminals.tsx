import { useEffect, useRef, useState, useSyncExternalStore } from "react";
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
import { useAsk } from "../console";
import { useActiveContext } from "../lib/clusters";
import { ClusterMovedAlert } from "../lib/clusterMoved";
import { useTabs } from "../lib/tabsStore";
import { useDismissOnPause } from "../lib/pausedContext";
import {
  endSession,
  getSessions,
  renameSession,
  subscribeSessions,
  terminalFor,
  type TerminalSessionRow,
} from "../lib/sessions";
import { clearTerminalsFocusRequest, useTerminalsFocusRequest } from "../lib/terminalPanel";
import { NewSessionMenu } from "./terminals/NewSessionMenu";
import {
  NOT_GATED,
  SESSION_RAIL_WIDTH,
  SESSION_VERDICT,
  SessionRail,
  sessionRailHead,
} from "./terminals/SessionRail";
import { TerminalView } from "./terminals/TerminalView";

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
  const { workspace } = useTabs();
  const paused = cluster !== undefined && workspace.pausedClusters?.includes(cluster.stableId) === true;
  const [picked, setPicked] = useState<number | null>(null);
  /**
   * The `New session` menu, and the cluster it was opened against.
   *
   * **Captured at the gesture, not read live.** Since #357 a dialog covers only
   * its own tab, so the rail stays live behind this menu and `setActiveCluster`
   * switches the active cluster in place with nothing remounting. The menu's
   * namespace, pod, container and node are all identities out of one cluster's
   * listings; with a live context they stayed in the fields while the shell
   * opened somewhere else — a pod name read off production, exec'd on staging.
   * `ResourceMenu`'s `Open shell` pinned its own pick for the same reason.
   */
  const [newSession, setNewSession] = useState<{ context: string; namespace: string } | null>(null);
  const targetPaused = useDismissOnPause(newSession?.context, () => setNewSession(null));

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

  // A page's terminal panel moved its session here (#942's Open in tab): that
  // session is the one on screen, whatever was selected before. Taken once.
  const focusRequest = useTerminalsFocusRequest();
  useEffect(() => {
    if (focusRequest === null) return;
    setPicked(focusRequest);
    clearTerminalsFocusRequest();
  }, [focusRequest]);

  // The selection, resolved against the rows that actually exist. A detached
  // session leaves an id behind that names nothing; the newest row is what a
  // reader is most likely to want next, and it is also what the effect above
  // would have chosen.
  const active =
    sessions.find((s) => s.id === picked) ??
    (sessions.length > 0 ? sessions[sessions.length - 1] : undefined);

  const context = active?.context ?? cluster?.name ?? "";
  // About the session on screen, which can be on another cluster than the rail.
  const ask = useAsk(context);
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
          {ask && (
            <Button
              type="button"
              size="sm"
              aria-label={`Draft a command: ${askQuestion(active, context)}`}
              onClick={() => ask(askQuestion(active, context))}
            >
              Draft a command
            </Button>
          )}
          <Button
            variant="primary"
            size="sm"
            disabled={paused}
            onClick={() =>
              setNewSession({ context: cluster?.name ?? "", namespace: cluster?.namespace ?? "" })
            }
          >
            New session
          </Button>
        </>
      }
    >
      {newSession && !targetPaused && (
        <NewSessionMenu
          // The cluster that was in focus when `New session` was pressed, not
          // the active session's and not the one in focus now: a new session is
          // started where the reader ASKED for it, and the menu says so itself
          // when there was no cluster rather than opening on an empty context.
          context={newSession.context}
          namespace={newSession.namespace}
          // Stated, and nothing more — a session writes nothing to the cluster
          // and the terminal it opens is captioned with its own context. Same
          // rule as the row menu's container picker.
          //
          // Both names have to be REAL for there to be a divergence to state.
          // An empty one is not a cluster: pinned `""` against the cluster the
          // reader selected next read "This still runs against , not prod-eu",
          // and a rail that lost its selection read "…against prod-eu, not ".
          // The menu says the no-cluster case itself instead.
          moved={
            newSession.context !== "" && cluster && newSession.context !== cluster.name ? (
              <ClusterMovedAlert pinned={newSession.context} live={cluster.name}>
                {` Cancel and open New session again to start one on ${cluster.name}.`}
              </ClusterMovedAlert>
            ) : undefined
          }
          onStarted={setPicked}
          onClose={() => setNewSession(null)}
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
            onRename={renameSession}
            onDetach={endSession}
            // The rail's own door into the menu, pinning the cluster exactly as
            // the toolbar's does above: two doors to one dialog is how the two
            // start disagreeing about which cluster they opened it on.
            onNewSession={() =>
              setNewSession({ context: cluster?.name ?? "", namespace: cluster?.namespace ?? "" })
            }
            newSessionDisabled={paused}
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
        {/* The pane carries `min-w-0` as well as `min-h-0`. A flex item's
            implicit `min-width: auto` refuses to shrink below its content, and
            xterm sizes its content in explicit pixels — so without it the pane
            grows to the terminal's width and pushes the 230px rail off the
            right edge of the window. Fifth time this property has bitten on
            this migration, and jsdom can see none of them. */}
        {active ? (
          <div data-slot="terminal-body" className="flex min-h-0 min-w-0 flex-1 flex-col">
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
            <div className="flex shrink-0 items-center gap-2 border-t border-rule bg-[var(--ground-sunk)] px-2.5 py-1.5">
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
