import { cx } from "./cx";
import { filled } from "./slot";
import { IconButton, type IconComponent } from "./IconButton";
import { Popover } from "./Popover";

export interface WorkspaceSummary {
  id: string;
  name: string;
  /** How many clusters it holds, and how many tabs are open in it. */
  clusters: number;
  tabs: number;
}

export interface WorkspaceSwitcherProps {
  workspaces: WorkspaceSummary[];
  activeId: string;
  onSelect: (id: string) => void;
  /** Offered per workspace when given, and never on the last one. */
  onRemove?: (id: string) => void;
  /** Offered under the list when given. */
  onCreate?: () => void;
  createLabel?: string;
  /** Marks the chip — a grid, a mark, whatever the app uses for a workspace. */
  icon?: IconComponent;
  emptyLabel?: string;
  /** Names the panel. */
  label?: string;
  className?: string;
}

/* Inline rather than an icon-set import: the kit takes no dependency on
   lucide, and these are the only two glyphs it needs. */
const ChevronGlyph = () => (
  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" aria-hidden="true" className="shrink-0">
    <path d="m6 9 6 6 6-6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const CloseGlyph: IconComponent = ({ size = 11, className, "aria-hidden": hidden }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} aria-hidden={hidden}>
    <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

/** "2 clusters · 11 tabs", and "1 cluster · 1 tab". */
function summarize({ clusters, tabs }: WorkspaceSummary): string {
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  return `${plural(clusters, "cluster")} · ${plural(tabs, "tab")}`;
}

/**
 * The chip at the left of the title bar, and the panel of workspaces it opens.
 *
 * A workspace is the largest thing a switch can change — which clusters are in
 * reach and which tabs are open — so the control that changes it has to be
 * reachable. The mock's was a `<span class="ws-chip">` handed to a popover as
 * its trigger: not focusable, not operable by keyboard, and not announced as a
 * control at all. It is a button here, which is the whole of the fix and most
 * of the reason this component exists. (#332)
 *
 * Everything it knew, it now takes. The mock read `useTabs()` and called
 * `switchWorkspace`, `createWorkspace` and `removeWorkspace` straight out of a
 * click; the kit may not hold app state, and a summary of each workspace plus
 * three callbacks is all the panel ever needed.
 *
 * Removing a workspace is one click, has no undo, and takes every tab in it.
 * Whether to ask first is the caller's — a confirmation this component put up
 * would be one the app could not skip — but the weight of it belongs in the
 * control's own name, so "Remove Production, 2 clusters and 11 tabs" is what a
 * screen reader says before the press rather than after. The last workspace
 * offers no remove at all: there would be nothing left to be in.
 */
export function WorkspaceSwitcher({
  workspaces,
  activeId,
  onSelect,
  onRemove,
  onCreate,
  createLabel = "New workspace",
  icon: Icon,
  emptyLabel = "No workspaces",
  label = "Workspaces",
  className,
}: WorkspaceSwitcherProps) {
  const active = workspaces.find((workspace) => workspace.id === activeId);
  // Never on the last one, whatever the caller passes: removing it leaves the
  // app with no workspace to be in and no way back to one.
  const removable = onRemove !== undefined && workspaces.length > 1;

  return (
    <Popover
      label={label}
      className={cx("w-[262px] py-1", className)}
      trigger={
        <span className="ws-chip">
          {Icon && <Icon size={12} aria-hidden="true" />}
          <span className="max-w-[110px] truncate">{active?.name ?? emptyLabel}</span>
          <ChevronGlyph />
        </span>
      }
    >
      {(close) => (
        <>
          <div className="px-2 pb-1">
            <span className="eyebrow">{label}</span>
          </div>
          {workspaces.length === 0 ? (
            <p className="px-2 py-3 text-center text-[0.75rem] text-muted">{emptyLabel}</p>
          ) : (
            workspaces.map((workspace) => {
              const current = workspace.id === activeId;
              return (
                <div key={workspace.id} className="flex items-center">
                  <button
                    type="button"
                    className="ns-row"
                    data-on={current}
                    // Which one you are in, said rather than only shown: the
                    // mock marked it with a tick at opacity 1 and nothing else.
                    aria-current={current || undefined}
                    onClick={() => {
                      onSelect(workspace.id);
                      close();
                    }}
                  >
                    <svg
                      width="12"
                      height="12"
                      viewBox="0 0 24 24"
                      fill="none"
                      aria-hidden="true"
                      // Always rendered: a tick appearing would shove the name
                      // it belongs to sideways.
                      className={cx("shrink-0", current ? "opacity-100" : "opacity-0")}
                      style={{ color: "var(--accent)" }}
                    >
                      <path d="M20 6 9 17l-5-5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                    <span className="min-w-0 flex-1 text-left">
                      <span className="block truncate">{workspace.name}</span>
                      <span className="path block truncate text-faint">{summarize(workspace)}</span>
                    </span>
                  </button>
                  {removable && (
                    <IconButton
                      icon={CloseGlyph}
                      // The whole cost, in the name. This is destructive, it
                      // has no undo, and the count is the part worth knowing.
                      label={`Remove ${workspace.name}, ${summarize(workspace).replace(" · ", " and ")}`}
                      title="Remove workspace"
                      className="mr-1 shrink-0"
                      onClick={() => onRemove?.(workspace.id)}
                    />
                  )}
                </div>
              );
            })
          )}
          {filled(onCreate ? createLabel : null) && (
            <div data-slot="workspace-new" className="rule-t mt-1 p-1.5">
              <button
                type="button"
                className="btn w-full justify-center"
                onClick={() => {
                  onCreate?.();
                  close();
                }}
              >
                {createLabel}
              </button>
            </div>
          )}
        </>
      )}
    </Popover>
  );
}
