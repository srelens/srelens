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
 * **The name changes with the glyph.** A button drawn as a check but still named
 * "Copy address for web-0" is the version a screen-reader user is handed, and it
 * is the wrong one. The spoken confirmation is {@link CopyAnnounce}'s, because a
 * name that changes under a button the reader has just pressed is not reliably
 * announced on its own.
 */
export function CopyIconButton({ icon, label, onCopy, className }: CopyIconButtonProps) {
  const { state, run } = useCopied();
  return (
    <>
      <IconButton
        icon={state === "copied" ? CheckGlyph : icon}
        label={state === "copied" ? "Copied" : state === "failed" ? `${label} — failed` : label}
        className={state === "copied" ? `copy-ok ${className ?? ""}`.trim() : className}
        onClick={() => void run(onCopy)}
      />
      <CopyAnnounce state={state} />
    </>
  );
}
