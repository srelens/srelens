import { Alert, Button, CopyButton, Screen, SideRail, usePortalShowing } from "@srelens/ui-kit";
import { isTauri } from "@srelens/core";
import { Console } from "../shell/Console";
import { useActiveContext } from "../lib/clusters";
import {
  clearAgentRun,
  dismissAgentError,
  useAgentRun,
  useActiveRunKey,
  useRunSubject,
  type Turn,
} from "../lib/agentRun";
import { pad2 } from "../lib/numbers";
import { titleFromQuestion } from "../lib/runTitle";
import { Transcript, transcriptText } from "./agent/Transcript";
import { AGENT_RAIL_WIDTH, RunsRail } from "./agent/RunsRail";

/** §5's own sentence, verbatim — the one line that says the dock and this
 *  screen are drawing the same conversation rather than two of them. Static
 *  copy, not a store read: it needs no field the store has, so cutting it
 *  alongside the figures #386/#387 exclude would have been over-applying that
 *  rule to a sentence it never touched. */
/*
 * §5's footer sentence — "Continue this run from the console at the bottom of
 * the window" — is GONE, deliberately.
 *
 * It earned its place when this screen had a composer of its own and the dock
 * was a separate surface: the sentence said those two were one conversation,
 * which a reader could not otherwise know. There is one prompt in the app now,
 * directly beneath this transcript, so the line instructed the reader to do
 * the only thing they could do.
 *
 * A sentence justified by a design that no longer exists is not neutral — it
 * is one more thing to read that says nothing.
 */

/** `started 14:04`, off the first turn's own timestamp — one of the two
 *  figures in §5's `started <time> · <n> calls · <duration>` head that the
 *  store actually observed (`Turn.at`, stamped by `askAgent` itself). Absent
 *  for an empty run — there is no first turn to have started at. */
function startedLabel(turns: readonly Turn[]): string | undefined {
  if (turns.length === 0) return undefined;
  const d = new Date(turns[0].at);
  return `started ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/**
 * `<n> calls`, off `Turn.calls[]` across every turn in the run — the OTHER
 * figure the store actually observes. #386's exclusion is scoped to
 * `RunsRail`'s `Recent runs` list (`SessionMeta` genuinely carries no
 * counts); THIS pane describes the live run pane 1 is heading, and the store
 * counts every tool call an agent has made in it. Dropping this alongside
 * duration was over-applying #386 to a figure it does not cover — the one
 * genuinely unknowable figure here is `duration` (a conversation has no
 * single well-defined one), which stays out.
 *
 * `0` renders nothing: a run with no tool calls yet is not the same fact as
 * "zero calls" worth reading out, so an absent reading renders no reading.
 */
function callCount(turns: readonly Turn[]): number {
  return turns.flatMap((t) => t.calls).length;
}

/**
 * `/agent` — §5's full view of the one agent run the window is holding: the
 * whole transcript, the composer that drives it, and the 312px rail beside
 * it.
 *
 * **This is a second renderer over the SAME store the console dock reads
 * (`lib/agentRun.ts`), never a copy of it.** `useAgentRun()` here is the
 * identical hook `shell/Console.tsx` calls; a question asked from the dock
 * shows up here and a question asked here shows up in the dock, because both
 * are drawing `run.turns`/`run.gates` and neither owns them.
 *
 * **`Composer` is mounted here full, `compact` deleted.** The dock never
 * mounted it — `ConsoleDock` supplies its own single-line prompt — so this
 * screen is `Composer`'s only caller, and it renders with nothing shrunk.
 */
export function Agent(_props: { route: string }) {
  const { turns, gates, error } = useAgentRun();
  const activeCtx = useActiveContext();
  const context = activeCtx?.name ?? "";
  // Whether this tab is the one on screen — see the dock's own note below.
  const showing = usePortalShowing();
  const activeKey = useActiveRunKey();
  const subject = useRunSubject(activeKey);
  const runCluster = subject?.about.cluster ?? context;
  const started = startedLabel(turns);
  const calls = callCount(turns);
  const head = started && calls > 0 ? `${started} · ${calls} call${calls === 1 ? "" : "s"}` : started;
  // The whole conversation, for the clipboard. Per-exchange copy is beside
  // each answer; this is the one for taking the lot into a ticket. Empty until
  // something has been said, so the control is not offered over nothing.
  const chatText = transcriptText(turns);
  // The conversation's own name, from the question that opened it — the same
  // derivation the rail row and the saved session use, so one conversation is
  // called one thing everywhere.
  const title = titleFromQuestion(turns.find((t) => t.role === "user")?.text ?? "");

  // The one place a web reader is TOLD, rather than left to discover it from a
  // failed send. `askAgent` starts with `chat_start`, and the web command
  // dispatcher (`crates/server/src/api_command.rs`) has no `chat_*` or
  // `agent_list` arm — every question 404s. So this screen says so instead of
  // drawing a composer that cannot work, and the dock hides itself for the
  // same reason.
  //
  // A paragraph, not an `Alert`: nothing has failed, and the whole point is
  // that nothing is asked to. Same treatment, and same reasoning, as the MCP
  // server's own section in Settings.
  if (!isTauri()) {
    return (
      <Screen title="Agent" eyebrow={context || undefined}>
        <div className="max-w-prose p-3">
          <p data-testid="agent-desktop-only" className="text-[0.8125rem] leading-relaxed text-muted">
            The agent runs in the srelens desktop app. It drives Claude, Codex, Cursor or srelens's
            own agent as local processes and answers through srelens's MCP tools — all of which are
            desktop commands, so there is nothing here for a question to reach.
          </p>
          <p className="mt-2 text-[0.75rem] leading-relaxed text-faint">
            Everything else on this cluster works in the browser: the resource screens, logs,
            events, Helm and port-forwards are all served by <code className="code">srelens
            server</code>.
          </p>
        </div>
      </Screen>
    );
  }

  return (
    <Screen
      title="Agent"
      // The SELECTED conversation's cluster, not the active workspace's. A run
      // started on cluster A survives a switch to B and can still be opened
      // here; naming B above a transcript that will be continued on A is the
      // #380 class of defect. Falls back to the active cluster when nothing is
      // selected, which is the ordinary case.
      eyebrow={runCluster || undefined}
      fill
      actions={
        <Button type="button" size="sm" onClick={() => clearAgentRun()}>
          New question
        </Button>
      }
    >
      <SideRail
        head="Agent"
        width={AGENT_RAIL_WIDTH}
        rail={<RunsRail />}
        mainHead={
          // The head reads left to right as: WHAT this conversation is, then
          // the figures about it, then the control that acts on it. The title
          // was derived and wired into the rail row and the saved file, and
          // NOTHING drew it here — so the screen said `Agent` twice and named
          // the open conversation nowhere.
          title || head || chatText !== "" ? (
            <span className="flex w-full min-w-0 items-center gap-3">
              {title !== "" && (
                <span
                  className="min-w-0 flex-1 truncate normal-case tracking-normal text-[0.8125rem] font-medium text-ink"
                  title={title}
                >
                  {title}
                </span>
              )}
              {/* Figures sit right, beside the control acting on the same run.
                  `ml-auto` so they stay right even with no title to push
                  them there. */}
              {head && (
                <span className="ml-auto min-w-0 shrink-0 truncate normal-case tracking-normal text-[0.75rem] text-faint">
                  {head}
                </span>
              )}
              {chatText !== "" && (
                <span className={head ? "shrink-0" : "ml-auto shrink-0"}>
                  <CopyButton text={chatText} label="Copy the whole conversation" iconOnly />
                </span>
              )}
            </span>
          ) : undefined
        }
      >
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {/* Run-level, not turn-level: a submission refused because a turn is
              already in flight, or a Stop that did not land. The store held
              this in `error` and NOTHING drew it, so a chip the reader pressed
              simply did nothing — the exact silence the refusal was meant to
              break. */}
          {error !== undefined && (
            <div className="px-3 pt-3">
              <Alert tone="sev" title="That question was not sent" onDismiss={() => dismissAgentError()}>
                <p className="m-0">{error}</p>
              </Alert>
            </div>
          )}
          {/* The chat sits on the CANVAS, not the surface. Exchanges are cards
              now — `bg-surface` inside a thin border — and a card on the same
              colour as its own ground is not a card. This is the pairing the
              mock uses: light ground, white exchanges. */}
          <div className="scroll min-h-0 min-w-0 flex-1 bg-canvas px-3 py-3">
            {turns.length === 0 ? (
              /* An empty screen said nothing at all — reported as "page looks
                 empty". What it says is what this agent can actually do,
                 because none of it is obvious from a blank pane: it drives a
                 real CLI, it reads through srelens's own tools, and a mutation
                 stops for the reader's approval. */
              <div className="mx-auto flex max-w-prose flex-col gap-3 pt-10">
                <h2 className="text-[0.9375rem] font-medium text-ink">Ask about this cluster</h2>
                <p className="m-0 text-[0.8125rem] leading-relaxed text-muted">
                  Questions go to a real agent CLI running on this machine — Claude, Codex, Cursor,
                  or srelens&rsquo;s own — and it answers by calling srelens&rsquo;s Kubernetes tools,
                  so what it tells you comes from the cluster rather than from memory.
                </p>
                <p className="m-0 text-[0.8125rem] leading-relaxed text-muted">
                  Every tool call it makes is listed with the arguments it used and how long it took.
                  Anything that would CHANGE the cluster stops and asks you first.
                </p>
                <p className="m-0 text-[0.8125rem] leading-relaxed text-muted">
                  Ask from any screen and the question is about what you are looking at — a pod&rsquo;s
                  logs, a list narrowed to one namespace. Each subject keeps its own conversation, and
                  they are listed in <span className="text-ink">Recent runs</span> beside this.
                </p>
                <p className="m-0 text-[0.75rem] leading-relaxed text-faint">
                  Type below, or press{" "}
                  <span className="kbd">⌘K</span> from anywhere. Paste a screenshot to send it along.
                </p>
              </div>
            ) : (
              <Transcript turns={turns} gates={gates} />
            )}
          </div>
          {/*
            The dock, mounted HERE rather than at the window's bottom edge.
            That is what lets this screen's rail be a full-height sibling and
            use the bottom of the window: the rail lives inside the screen, so
            a dock below the screen bounds it, and everything under it was dead
            space three attempts running.

            Still one dock component and one instance — `Window` skips its own
            on this route.
          */}
          {/*
            `fullView`: the dock is this screen's own composer, sitting directly
            under the transcript. It is the mount site that knows that — so it
            says so, rather than leaving the dock to infer it from a route
            string.

            And only while this tab is the one on SCREEN. `TabSurface` keeps
            hidden tabs mounted, so an `/agent` tab sitting behind another one
            kept this dock alive while `Window` mounted its own — two live
            consoles, each with its own draft and attachments, both registering
            against the single provider, with whichever effect ran last owning
            `ask()`. `Window`'s comment claiming "one instance — never both at
            once" was simply untrue.
          */}
          {showing && <Console fullView />}
        </div>
      </SideRail>
    </Screen>
  );
}
