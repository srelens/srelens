import { useState } from "react";
import { ConfirmDialog, IconButton, Titlebar, WorkspaceSwitcher } from "@srelens/ui-kit";
import { applyUiScale, getUiScale, isApplePlatform, isTauri, setUiScale, stepUiScale } from "@srelens/core";
import { rememberTheme } from "../lib/appearance";
import { Icons } from "../lib/icons";
import { hint } from "../lib/shortcuts";
import { openTab, removeWorkspace, switchWorkspace, useTabs } from "../lib/tabsStore";
import { useCanLockWorkspace, useWorkspaceSealed } from "./LockGate";

export interface ChromeProps {
  controls: "macos" | "none";
  /** The active cluster's display name — what this window is looking at. */
  clusterName?: string;
  onToggleTheme: () => void;
  /** The window owns the create dialog; the chip only asks for it. */
  onNewWorkspace: () => void;
  /**
   * Seal the vault and cover the window.
   *
   * This is `Window`'s own `lockNow` — the same function `⌘⇧L` fires, not a
   * second path to the same place. It orders `vaultLock()` before
   * `lockWorkspace()`, returns early when the cover is already up, and skips
   * web mode entirely; a copy of that here would be a second door, and two
   * doors are how they start disagreeing.
   *
   * Optional because the component gallery renders this bar with no window
   * behind it. Where it is absent the control is not drawn — the same rule as
   * every other absent affordance in this bar.
   */
  onLock?: () => void;
}

/**
 * Read, stepped, stored and applied in one place.
 *
 * Exported because the titlebar is not the only thing that zooms: the window
 * binds Cmd/Ctrl +/-/0 to the same three actions, and two copies of this would
 * be two chances for the keyboard and the buttons to disagree about what a step
 * is or whether it was persisted. `setUiScale` clamps and returns what it
 * stored, so what gets applied is what was saved rather than what was asked
 * for — at the ends of the range those differ.
 */
export function zoom(action: "in" | "out" | "reset") {
  const next = setUiScale(stepUiScale(getUiScale(), action));
  applyUiScale(next);
}

/**
 * The bar across the top of the window: which workspace you are in, which
 * cluster this window is looking at, and the actions that act on the window.
 *
 * Everything visual is the kit's; what lives here is the wiring the kit is not
 * allowed to know — the tab store, the scale in core's settings, and the
 * decision about whether removing a workspace deserves a question.
 *
 * That decision is the only judgement in the file. Removing a workspace has no
 * undo and takes every tab in it, so it asks — except when there is nothing to
 * ask about. A workspace is seeded with exactly one tab, the pinned home tab it
 * is born with; confirming the loss of a tab that was never opened is a dialog
 * about nothing, and a dialog about nothing is what teaches people to dismiss
 * the ones that matter. So one tab removes outright and anything else confirms.
 *
 * The test is the count, deliberately, and not "every tab is pinned" — which is
 * what this first read as. Pinning is `togglePin`, a thing the user does to say
 * a tab matters, so a workspace whose tabs had all been pinned was the case
 * most worth asking about and the one that skipped the question: the tabs
 * someone had marked as worth keeping went silently, with no undo, on the exact
 * path the confirmation exists to guard.
 *
 * The zoom controls are desktop-only. `applyUiScale` asks the webview to zoom;
 * in web mode there is no webview to ask, the browser's own zoom already does
 * this, and three buttons that quietly do nothing are worse than three buttons
 * that are not there.
 *
 * **This bar stands down while the vault is sealed** (spec decision 5). §25
 * leaves the titlebar outside what a lock replaces, and that is defensible
 * VISUALLY — the window keeps its shape. Leaving it interactive is not: behind
 * a raised cover the gear opened `/settings` and the workspace switcher's
 * `onRemove` deleted a workspace outright, with no dialog and no credential,
 * whenever it held one tab — which is how every workspace starts. A cover that
 * leaves those live is worse than no lock, because the window LOOKS sealed.
 *
 * So the switcher becomes a readout, the gear is disabled with a reason, and
 * `Lock workspace` is not drawn at all — it would be absurd for the lock to be
 * the one live control on a locked window. Theme and zoom stay: both change only
 * how this screen LOOKS, and a reader who cannot read the passphrase field
 * cannot unlock. The names stay too — they come from a kubeconfig and a tab
 * store the vault never sealed, and blanking them would imply it had.
 */
export function Chrome({ controls, clusterName, onToggleTheme, onNewWorkspace, onLock }: ChromeProps) {
  const { workspace, workspaces } = useTabs();
  const sealed = useWorkspaceSealed();
  /**
   * Whether to draw `Lock workspace` at all.
   *
   * `⌘⇧L` has been bound since Task 9 and NOTHING on screen offered it: the
   * only ways to lock were a chord and a button inside Settings → Security.
   * This is the discoverable door.
   *
   * Three absences rather than a disabled state, and each is this project's
   * existing rule rather than a new one. No handler, no control. Web mode —
   * every vault command is a raw `invoke` and rejects there, which is why the
   * `Security` nav entry and the toolbox's install column are not drawn
   * either. And no open vault: {@link useCanLockWorkspace} is false before the
   * launch read answers, when it refused, when the vault has never been set up
   * (locking one is refused by design) and when it is already sealed — which is
   * also what makes this control inert behind the cover, alongside the switcher
   * and the gear.
   */
  const lockable = useCanLockWorkspace();
  const chord = hint("lock", isApplePlatform());
  const [removing, setRemoving] = useState<string | null>(null);
  // Found rather than held, so the dialog reads the live workspace: the count
  // in its message would otherwise be whatever it was when the dialog opened.
  const target = workspaces.find((w) => w.id === removing) ?? null;
  const desktop = isTauri();
  // The overlay titlebar keeps macOS's real traffic lights whenever the
  // window is Tauri+Apple, regardless of `controls`. The kit draws its own
  // picture of them only when `controls === "macos"`; that flag and this gap
  // must never both be true, or the OS's lights and the kit's painted ones
  // stack (the doubling `ec024b5` removed). Deriving `nativeLights` from
  // `controls` here — instead of a caller having to keep the two hand-synced
  // — is what makes that invariant hold no matter what `controls` a caller
  // passes: this bar reserves the gap exactly when it is NOT asking the kit
  // for the picture but the OS is drawing the real thing anyway.
  const nativeLights = controls !== "macos" && desktop && isApplePlatform();

  function askRemove(id: string) {
    const w = workspaces.find((x) => x.id === id);
    if (!w) return;
    if (w.tabs.length <= 1) removeWorkspace(id);
    else setRemoving(id);
  }

  const openTabs = target ? target.tabs.length : 0;

  return (
    <>
      <Titlebar
        controls={controls}
        label="Window"
        leading={
          <>
            {nativeLights && (
              // Clears macOS's own traffic lights, which the overlay keeps.
              // Carries the drag attribute so the dead space they leave still
              // moves the window.
              <span aria-hidden data-native-lights data-tauri-drag-region className="w-16 shrink-0 self-stretch" />
            )}
            {sealed ? (
              /* The chip's own class, with nothing to press. Unmounted rather
                 than disabled because the switcher is a Popover trigger with
                 no disabled state, and a panel that opened over a lock screen
                 to offer Remove is the last thing this should grow. */
              <span className="ws-chip" data-sealed="true">
                {Icons.workspace && <Icons.workspace size={12} aria-hidden="true" />}
                <span className="max-w-[110px] truncate">{workspace.name}</span>
              </span>
            ) : (
              <WorkspaceSwitcher
                icon={Icons.workspace}
                workspaces={workspaces.map((w) => ({
                  id: w.id,
                  name: w.name,
                  clusters: w.clusters.length,
                  tabs: w.tabs.length,
                }))}
                activeId={workspace.id}
                onSelect={switchWorkspace}
                onRemove={askRemove}
                onCreate={onNewWorkspace}
              />
            )}
          </>
        }
        title={clusterName ?? "srelens"}
        actions={
          <>
            {desktop && <IconButton icon={Icons.zoomOut} label="Zoom out" onClick={() => zoom("out")} />}
            {desktop && <IconButton icon={Icons.zoomReset} label="Reset zoom" onClick={() => zoom("reset")} />}
            {desktop && <IconButton icon={Icons.zoomIn} label="Zoom in" onClick={() => zoom("in")} />}
            {onLock !== undefined && desktop && lockable && (
              <IconButton
                icon={Icons.lock}
                // A real name, not a glyph: this is the one control in the bar
                // whose mistaken press replaces the window. The chord rides in
                // the tooltip rather than the name, so what is announced is the
                // action and not a run of modifier glyphs.
                label="Lock workspace"
                // From the table that binds it (`lib/shortcuts.ts`), never
                // typed here — a glyph written by hand is wrong the moment the
                // binding moves and stays wrong until somebody notices. `hint`
                // returns "" for an unbound action, so an unbound chord leaves
                // the tooltip as the label rather than trailing a bare
                // separator.
                title={chord === "" ? "Lock workspace" : `Lock workspace · ${chord}`}
                onClick={onLock}
              />
            )}
            <IconButton
              icon={Icons.sun}
              label="Theme"
              // Recorded, not just applied. The handler is the host's
              // (`toggleNextDesignTheme` writes classic's preference and
              // re-asserts ui-next's `data-theme`), and the Appearance pane's
              // store used to be the only thing that wrote the appearance
              // record — so this button's choice was remembered nowhere and
              // boot put the pane's older theme back over it at the next
              // launch. `rememberTheme` reads whatever the host just put on
              // the root, so it stores the value that actually landed rather
              // than a mode this package guessed at. After the toggle, not
              // before.
              onClick={() => {
                onToggleTheme();
                rememberTheme();
              }}
            />
            <IconButton
              icon={Icons.settings}
              label="Appearance settings"
              // Disabled rather than removed: the reader can see the way in is
              // still there and be told why it will not open, which a control
              // that vanished could not do. `title` is what `IconButton`
              // documents for exactly this.
              disabled={sealed}
              title={sealed ? "Unlock the workspace to open Settings" : undefined}
              onClick={() => openTab("/settings")}
            />
          </>
        }
      />
      {target && (
        <ConfirmDialog
          title={`Remove ${target.name}?`}
          message={`${openTabs} open ${openTabs === 1 ? "tab" : "tabs"} in this workspace will close. The clusters stay in your kubeconfig.`}
          confirmLabel="Remove"
          danger
          onConfirm={() => {
            removeWorkspace(target.id);
            setRemoving(null);
          }}
          onCancel={() => setRemoving(null)}
        />
      )}
    </>
  );
}
