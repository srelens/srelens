import { useEffect, useState } from "react";
import { ConsoleDock, EmptyState } from "@srelens/ui-kit";
import { useConsole } from "../console";
import { hint } from "../lib/shortcuts";

/**
 * The console docked along the bottom of the window: the kit's `ConsoleDock`
 * joined to the provider's state, and standing in for an agent that has not
 * been ported yet.
 *
 * It answers nothing, and says so. That is the point of mounting it now: the
 * dock, the accelerator that opens it and the `ask()` path from every other
 * screen are the parts the rest of the shell has to build against, and they
 * can be finished and tested while the agent behind them is still classic's.
 * A question asked here is acknowledged rather than dropped, so the seam
 * reads as unfinished rather than broken. (#320)
 *
 * `registerSubmit` in an effect is what makes `ask()` from elsewhere land in
 * this component; the provider holds a question asked before the dock has
 * mounted and delivers it on registration, so there is no race to lose one to.
 * The cleanup matters: unregistering on unmount is what stops a later question
 * calling into a component that is gone.
 */
export function Console({ apple }: { apple: boolean }) {
  const { open, setOpen, scope, registerSubmit } = useConsole();
  const [value, setValue] = useState("");
  const [asked, setAsked] = useState<string[]>([]);

  function onSubmit(question: string) {
    setAsked((prev) => [...prev, question]);
    // The dock is controlled, so a submitted question stays at the prompt
    // unless someone clears it. Clearing it is what makes the prompt ready
    // for the next one.
    setValue("");
    setOpen(true);
  }

  useEffect(() => registerSubmit(onSubmit), [registerSubmit]);

  const last = asked[asked.length - 1];

  return (
    <ConsoleDock
      open={open}
      onOpenChange={setOpen}
      value={value}
      onValueChange={setValue}
      onSubmit={onSubmit}
      mode="Agent"
      // Empty rather than absent is a bordered chip with nothing in it, and
      // the console is unscoped until something on screen scopes it.
      context={scope || undefined}
      shortcutHint={hint("console", apple)}
      emptyLabel="Ask about the cluster you are looking at"
      onClear={() => setAsked([])}
    >
      {last !== undefined && (
        <EmptyState
          title="The agent is not in the new design yet"
          hint={`You asked: ${last}`}
        />
      )}
    </ConsoleDock>
  );
}
