import { useState } from "react";
import { Button } from "../ui";
import { PORTED_SCREENS, loadDesign, switchDesign, type Design } from "../design";

/**
 * Settings → Appearance: the choice between the current design and the new one.
 *
 * Sits beside the theme controls rather than in a section of its own, because
 * from the user's side it is the same kind of decision — how the app looks.
 *
 * The new design is the default and classic is deprecated, so the copy says
 * so plainly: someone staying on classic should know it is going away.
 */
export function AppearanceSettingsSection() {
  // Read once: switching reloads the window, so this cannot go stale while
  // the component is mounted.
  const [current] = useState<Design>(loadDesign);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function choose(design: Design) {
    // Re-picking the design already in use would reload for nothing.
    if (design === current || busy) return;
    setBusy(true);
    setError(null);
    const result = await switchDesign(design);
    // A successful switch reloads, so only a refusal ever gets here. Clearing
    // busy matters: without it both buttons stayed disabled for good, and the
    // setting looked broken rather than unavailable. (#314 review)
    if (!result.ok) {
      setError(result.reason);
      setBusy(false);
    }
  }

  return (
    <div className="fl-settings-field">
      <h3>Design</h3>
      <p className="fl-settings-hint">
        The new design is the default. The classic design is deprecated and will be removed in
        a future version. Switching reloads the window.
      </p>
      <p className="fl-settings-hint">In the new design so far:</p>
      <ul className="fl-settings-hint">
        {PORTED_SCREENS.map((s) => (
          <li key={s.route}>{s.name}</li>
        ))}
      </ul>
      {error && (
        <p className="fl-settings-hint" role="alert">
          Could not switch design. {error}
        </p>
      )}
      <div role="group" aria-label="Design">
        <Button
          type="button"
          onClick={() => void choose("classic")}
          aria-pressed={current === "classic"}
          disabled={busy}
        >
          Classic (deprecated)
        </Button>
        <Button
          type="button"
          onClick={() => void choose("next")}
          aria-pressed={current === "next"}
          disabled={busy}
        >
          New design (default)
        </Button>
      </div>
    </div>
  );
}
