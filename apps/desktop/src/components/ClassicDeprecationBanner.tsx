import { useId, useState } from "react";
import { X } from "lucide-react";
import { Button, IconButton } from "../ui";
import { switchDesign } from "../design";

/** Session scope: a dismissal survives a reload of the window, not a relaunch. */
const DISMISSED_KEY = "srelens.design.classicNoticeDismissed";

function wasDismissed(): boolean {
  try {
    return sessionStorage.getItem(DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * The window-level notice that classic is on its way out, at the top of the
 * main column so it shows on every tab and on the landing page alike.
 *
 * The switch goes through `switchDesign`, the same path Settings → Appearance
 * uses, so a refusal reads the same in both places. It reloads the window, and
 * session restore leaves out create and edit tabs, so with `unsavedDrafts`
 * open it asks first rather than throwing that work away.
 */
export function ClassicDeprecationBanner({ unsavedDrafts = 0 }: { unsavedDrafts?: number }) {
  const [dismissed, setDismissed] = useState(wasDismissed);

  if (dismissed) return null;

  function dismiss() {
    try {
      sessionStorage.setItem(DISMISSED_KEY, "1");
    } catch {
      // Session storage throws in some privacy modes; it then comes back on
      // the next reload, which is a nag, not a fault.
    }
    setDismissed(true);
  }

  return (
    <div className="fl-design-deprecated" role="status">
      <p>The classic design is deprecated and will be removed in a future version.</p>
      <SwitchToNewDesign unsavedDrafts={unsavedDrafts} />
      <IconButton icon={X} label="Dismiss" onClick={dismiss} />
    </div>
  );
}

/**
 * What classic draws where an app page or Settings → Apps used to be: apps
 * are built for the new design only (#1001). Unlike the banner it cannot be
 * dismissed, because there is nothing else to show in its place.
 */
export function ClassicAppsNotice({ unsavedDrafts = 0 }: { unsavedDrafts?: number }) {
  return (
    <div className="flex flex-col items-start gap-2 p-4 text-sm">
      <h2 className="text-base font-semibold">Apps open in the new design</h2>
      <p>The classic design no longer shows apps, their pages or their settings.</p>
      <SwitchToNewDesign unsavedDrafts={unsavedDrafts} />
    </div>
  );
}

function SwitchToNewDesign({ unsavedDrafts }: { unsavedDrafts: number }) {
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const warningId = useId();

  async function switchToNext() {
    setConfirming(false);
    setBusy(true);
    setError(null);
    try {
      const result = await switchDesign("next");
      // A successful switch reloads, so only a refusal gets here.
      if (!result.ok) {
        setError(result.reason);
        setBusy(false);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setBusy(false);
    }
  }

  const drafts = `${unsavedDrafts} unsaved editor draft${unsavedDrafts === 1 ? "" : "s"}`;

  return (
    <>
      {confirming && <p id={warningId}>Switching reloads the window and discards {drafts}.</p>}
      {error && <p role="alert">Could not switch design. {error}</p>}
      {confirming ? (
        <>
          <Button type="button" size="sm" onClick={() => void switchToNext()}>
            Switch anyway
          </Button>
          {/* The pressed button is gone; land on the safe choice, which reads the warning. */}
          <Button
            type="button"
            size="sm"
            variant="ghost"
            autoFocus
            aria-describedby={warningId}
            onClick={() => setConfirming(false)}
          >
            Cancel
          </Button>
        </>
      ) : (
        <Button
          type="button"
          size="sm"
          onClick={() => (unsavedDrafts > 0 ? setConfirming(true) : void switchToNext())}
          disabled={busy}
        >
          Switch to the new design
        </Button>
      )}
    </>
  );
}
