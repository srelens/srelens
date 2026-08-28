import { useState } from "react";
import { Button } from "../ui";
import { PORTED_SCREENS, loadDesign, switchDesign, type Design } from "../design";

/**
 * Settings → Appearance: the choice between the current design and the new one.
 *
 * Sits beside the theme controls rather than in a section of its own, because
 * from the user's side it is the same kind of decision — how the app looks.
 *
 * The copy is deliberately blunt about the new design being unfinished.
 * Shipping a half-built UI behind a toggle is only defensible if the toggle
 * says so; someone who opts in and finds empty screens should have been told,
 * not surprised.
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
        srelens is being rebuilt in a new design. It is <strong>in progress</strong>: most
        screens are not there yet, and the ones that are may still change. Switching reloads
        the window, and you can switch back here at any time.
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
          Classic design
        </Button>
        <Button
          type="button"
          onClick={() => void choose("next")}
          aria-pressed={current === "next"}
          disabled={busy}
        >
          New design (in progress)
        </Button>
      </div>
    </div>
  );
}
