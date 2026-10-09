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
 * uses, so a refusal reads the same in both places.
 */
export function ClassicDeprecationBanner() {
  const [dismissed, setDismissed] = useState(wasDismissed);
  const [busy, setBusy] = useState(false);
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
    setBusy(true);
    setError(null);
    const result = await switchDesign("next");
    // A successful switch reloads, so only a refusal gets here.
    if (!result.ok) {
      setError(result.reason);
      setBusy(false);
    }
  }

  return (
    <div className="fl-design-deprecated" role="status">
      <p>
        The classic design is deprecated and will be removed in a future version.
        {error && <span role="alert"> Could not switch design. {error}</span>}
      </p>
      <Button type="button" size="sm" onClick={() => void switchToNext()} disabled={busy}>
        Switch to the new design
      </Button>
      <IconButton icon={X} label="Dismiss" onClick={dismiss} />
    </div>
  );
}
