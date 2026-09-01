import type { AgentInfo } from "@srelens/core";
import { Popover } from "@srelens/ui-kit";

/**
 * The agent picker: a popover over `agents`, ALREADY FILTERED to
 * `available && !gated` by the caller. Never rendered with a `disabled`
 * entry for a gated agent (Ruling: an agent that is installed but gated must
 * not be offered) — the whole point of filtering before this component ever
 * sees the list, rather than filtering here and risking a call site that
 * forgets to.
 */
export function AgentPicker({
  agents,
  selectedKind,
  onSelect,
  disabled,
}: {
  agents: AgentInfo[];
  selectedKind: string;
  onSelect: (kind: string) => void;
  /** While a turn is in flight — switching would strand the running CLI (see
   *  `chooseAgent`). Disabled rather than silently refused, so the reader can
   *  see why the control is not available. */
  disabled?: boolean;
}) {
  const current = agents.find((a) => a.kind === selectedKind);
  if (disabled) {
    return (
      // The same `.chip` shape, minus the caret: the reader should see which
      // agent is answering, and see that it is not theirs to change yet.
      <span
        className="chip opacity-60"
        title="Stop the question in flight before switching agent"
      >
        <span>{current?.label ?? "Agent"}</span>
      </span>
    );
  }
  return (
    <Popover
      label="Choose agent"
      trigger={
        // `.chip` — the kit's existing small-control shape, bordered and sunk.
        // A bare `<span>` here rendered the agent's name as loose text under
        // the input, which read as a stray label rather than as something to
        // press; reported as the agent selection needing to look like it
        // belongs below the box. The caret is what says it opens.
        <span className="chip">
          <span>{current?.label ?? "Agent"}</span>
          <span aria-hidden className="text-faint">
            ⌄
          </span>
        </span>
      }
    >
      {(close) => (
        <div role="listbox" className="flex min-w-0 flex-col">
          {agents.map((a) => {
            const selected = a.kind === selectedKind;
            return (
              <button
                key={a.kind}
                type="button"
                role="option"
                aria-selected={selected}
                onClick={() => {
                  onSelect(a.kind);
                  close();
                }}
                className="flex min-w-0 items-center gap-2 rounded-tile px-2 py-1.5 text-left text-sm hover:bg-sunk"
              >
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
              </button>
            );
          })}
        </div>
      )}
    </Popover>
  );
}
