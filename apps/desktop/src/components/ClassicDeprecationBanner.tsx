import { useState } from "react";
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
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    <div className="fl-design-deprecated" role="status">
      <p>
        {confirming
          ? `Switching reloads the window and discards ${drafts}.`
          : "The classic design is deprecated and will be removed in a future version."}
        {error && <span role="alert"> Could not switch design. {error}</span>}
      </p>
      {confirming ? (
        <>
          <Button type="button" size="sm" onClick={() => void switchToNext()}>
            Switch anyway
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={() => setConfirming(false)}>
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
      <IconButton icon={X} label="Dismiss" onClick={dismiss} />
    </div>
  );
}
