import { Button, Eyebrow, Screen, SideRail } from "@srelens/ui-kit";
import { useActiveContext } from "../lib/clusters";
import { clearAgentRun, useAgentRun, type Turn } from "../lib/agentRun";
import { Composer } from "./agent/Composer";
import { Transcript } from "./agent/Transcript";
import { AGENT_RAIL_WIDTH, RunsRail } from "./agent/RunsRail";

/** §5's own sentence, verbatim — the one line that says the dock and this
 *  screen are drawing the same conversation rather than two of them. Static
 *  copy, not a store read: it needs no field the store has, so cutting it
 *  alongside the figures #386/#387 exclude would have been over-applying that
 *  rule to a sentence it never touched. */
const CONTINUE_FROM_CONSOLE = "Continue this run from the console at the bottom of the window";

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** `started 14:04`, off the first turn's own timestamp — the one figure in
 *  §5's `started <time> · <n> calls · <duration>` head that the store
 *  actually observed (`Turn.at`, stamped by `askAgent` itself). The call
 *  count and duration beside it in the mock are #386's business, not this
 *  one's: neither is stored anywhere, so unlike the time they stay out.
 *  Absent for an empty run — there is no first turn to have started at. */
function startedLabel(turns: readonly Turn[]): string | undefined {
  if (turns.length === 0) return undefined;
  const d = new Date(turns[0].at);
  return `started ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
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
  const { turns, gates } = useAgentRun();
  const activeCtx = useActiveContext();
  const context = activeCtx?.name ?? "";
  const started = startedLabel(turns);

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
          started ? (
            <span className="min-w-0 truncate normal-case tracking-normal text-[0.75rem] text-ink">
              {started}
            </span>
          ) : undefined
        }
      >
        <div className="flex min-h-0 flex-1 min-w-0 flex-col gap-3 p-3">
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
