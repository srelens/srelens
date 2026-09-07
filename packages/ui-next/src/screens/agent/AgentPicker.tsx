import type { AgentInfo } from "@srelens/core";
import { OptionCheck, Popover } from "@srelens/ui-kit";

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
      // `.popover` floors itself at 240px, which is written for a panel holding
      // a namespace filter or a search box. Four short agent names in it left
      // most of the panel empty — reported as "still width is too much". A
      // utility wins over the component layer (kit.css declares `@layer
      // utilities` after `@layer components`), and the small floor keeps a
      // one-name list still reading as a menu.
      className="min-w-[7.5rem]"
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
        // `.ns-row` and `OptionCheck` — the design's own row inside a popover
        // and its own mark for "this is the one", the pair `WorkspaceSwitcher`
        // and the kit's `PickerRow` already wear. This list hand-rolled
        // `hover:bg-sunk` with no mark at all, so it read as a plain white
        // list that did not belong to the app and never said which agent was
        // answering. Reported as "use same ones used in the project".
        //
        // `padding: 2px` on the panel, as `.ctx-menu` has: rows that reach the
        // panel's own border have no rounding to show.
        <div role="listbox" className="flex min-w-0 flex-col p-[2px]">
          {agents.map((a) => {
            const selected = a.kind === selectedKind;
            return (
              <button
                key={a.kind}
                type="button"
                role="option"
                // `aria-selected` is the ARIA state for a chosen option, and
                // `data-on` is what `.ns-row` styles off. Both, because they
                // answer to different readers.
                aria-selected={selected}
                data-on={selected}
                onClick={() => {
                  onSelect(a.kind);
                  close();
                }}
                className="ns-row rounded-[5px]"
              >
                <OptionCheck checked={selected} />
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
              </button>
            );
          })}
        </div>
      )}
    </Popover>
  );
}
