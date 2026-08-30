import { Button, Screen, SideRail } from "@srelens/ui-kit";
import { useActiveContext } from "../lib/clusters";
import { clearAgentRun, useAgentRun } from "../lib/agentRun";
import { Composer } from "./agent/Composer";
import { Transcript } from "./agent/Transcript";
import { AGENT_RAIL_WIDTH, RunsRail } from "./agent/RunsRail";

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
      <SideRail head="Agent" width={AGENT_RAIL_WIDTH} rail={<RunsRail />}>
        <div className="flex min-h-0 flex-1 min-w-0 flex-col gap-3 p-3">
          <div className="scroll min-h-0 min-w-0 flex-1">
            <Transcript turns={turns} gates={gates} />
          </div>
          <Composer context={context} />
        </div>
      </SideRail>
    </Screen>
  );
}
