import { Alert, Button, Eyebrow, Screen, SideRail } from "@srelens/ui-kit";
import { isTauri } from "@srelens/core";
import { useActiveContext } from "../lib/clusters";
import { clearAgentRun, dismissAgentError, useAgentRun, type Turn } from "../lib/agentRun";
import { pad2 } from "../lib/numbers";
import { Composer } from "./agent/Composer";
import { Transcript } from "./agent/Transcript";
import { AGENT_RAIL_WIDTH, RunsRail } from "./agent/RunsRail";

/** §5's own sentence, verbatim — the one line that says the dock and this
 *  screen are drawing the same conversation rather than two of them. Static
 *  copy, not a store read: it needs no field the store has, so cutting it
 *  alongside the figures #386/#387 exclude would have been over-applying that
 *  rule to a sentence it never touched. */
const CONTINUE_FROM_CONSOLE = "Continue this run from the console at the bottom of the window";

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
  const started = startedLabel(turns);
  const calls = callCount(turns);
  const head = started && calls > 0 ? `${started} · ${calls} call${calls === 1 ? "" : "s"}` : started;

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
      eyebrow={context || undefined}
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
          head ? (
            <span className="min-w-0 truncate normal-case tracking-normal text-[0.75rem] text-ink">
              {head}
            </span>
          ) : undefined
        }
      >
        <div className="flex min-h-0 flex-1 min-w-0 flex-col gap-3 p-3">
          {/* Run-level, not turn-level: a submission refused because a turn is
              already in flight, or a Stop that did not land. The store held
              this in `error` and NOTHING drew it, so a chip the reader pressed
              simply did nothing — the exact silence the refusal was meant to
              break. */}
          {error !== undefined && (
            <Alert tone="sev" title="That question was not sent" onDismiss={() => dismissAgentError()}>
              <p className="m-0">{error}</p>
            </Alert>
          )}
          <div className="scroll min-h-0 min-w-0 flex-1">
            <Transcript turns={turns} gates={gates} />
          </div>
          <Eyebrow>{CONTINUE_FROM_CONSOLE}</Eyebrow>
          <Composer context={context} />
        </div>
      </SideRail>
    </Screen>
  );
}
