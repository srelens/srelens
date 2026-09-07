import { CopyAnnounce } from "./CopyAnnounce";
import { IconButton, type IconComponent } from "./IconButton";
import { useCopied } from "./useCopied";

/* Inline rather than an icon-set import: the kit takes no dependency on lucide,
   and this is the only glyph it needs. */
const CheckGlyph: IconComponent = ({ size = 14, ...rest }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" {...rest}>
    <path
      d="m5 13 4 4 10-10"
      stroke="currentColor"
      strokeWidth="3"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export interface CopyIconButtonProps {
  /** The glyph at rest. A check replaces it while the copy is confirmed. */
  icon: IconComponent;
  /** The name at rest — "Copy address for web-0", not "Copy". */
  label: string;
  /**
   * Puts the thing on the clipboard. Resolving `false`, or throwing, means it
   * did not happen and is drawn as such — the write stays the caller's, so a
   * caller that already owns a copy helper keeps using it.
   */
  onCopy: () => void | boolean | Promise<void | boolean>;
  className?: string;
}

/**
 * A copy control that answers: the glyph becomes a check and the name becomes
 * "Copied" for a moment, then both come back.
 *
 * The icon-only sibling of {@link CopyCommand}, for the places with no room to
 * print the thing being copied — a table row's trailing column, a line inside a
 * confirm dialog. Both draw their state from {@link useCopied}, so the delay and
 * the never-say-Copied-on-failure rule are written once. (#410)
 *
 * **The name does not change; the live region speaks instead.** There is no
 * visible word here to carry the outcome, so {@link CopyAnnounce} carries it —
 * and the button stays called what it does. Renaming it to "Copied" was the
 * first draft, and it is the wrong half of the choice twice over: the name of
 * the control under the reader's own click changes, and the announcement
 * arrives as well, so the news is delivered twice. This is the same split
 * `CopyButton` makes for its icon-only form. (#413 review)
 *
 * The outcome also rides along as the tooltip, which is the only place a
 * sighted user can be told that a copy FAILED on a control with no word in it —
 * a check that never appears is not a message. `title` is a description, not a
 * name, so it does not disturb the one above it.
 */
export function CopyIconButton({ icon, label, onCopy, className }: CopyIconButtonProps) {
  const { state, run } = useCopied();
  return (
    <>
      <IconButton
        icon={state === "copied" ? CheckGlyph : icon}
        label={label}
        title={state === "copied" ? "Copied" : state === "failed" ? "Copy failed" : undefined}
        className={state === "copied" ? `copy-ok ${className ?? ""}`.trim() : className}
        onClick={() => void run(onCopy)}
      />
      <CopyAnnounce state={state} />
    </>
  );
}
