import { cx } from "./cx";
import { filled } from "./slot";
import { IconButton, type IconComponent } from "./IconButton";

export interface KubectlPreviewProps {
  /** The kubectl-equivalent command. Omit (and pass `note` instead) when there's no faithful one-liner. */
  command?: string;
  /** Shown instead of a command when no clean kubectl equivalent exists (e.g. evict). */
  note?: string;
  /** Copy-to-clipboard handler; renders a copy affordance next to `command` when set. */
  onCopy?: () => void;
  className?: string;
}

/* Inline rather than an icon-set import: the kit takes no dependency on
   lucide, and this is the only glyph it needs. */
const CopyGlyph: IconComponent = ({ size = 14, ...rest }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" {...rest}>
    <rect x="9" y="9" width="12" height="12" rx="2" stroke="currentColor" strokeWidth="2" />
    <path
      d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    />
  </svg>
);

/**
 * The kubectl command an action stands for, shown inside the confirm dialog
 * that action opens.
 *
 * It is here so that a destructive click is never a black box: the user reads
 * the one-liner they would otherwise have typed, and can take it away with
 * them. Not every action has a faithful equivalent — evict is an API call with
 * no kubectl verb of its own — so `note` says as much in the same place, rather
 * than leaving the dialog quietly silent about what it is about to do.
 *
 * The classic version coloured itself with `text-muted-foreground` and laid
 * itself out with inline styles; the new design's equivalents are `text-muted`
 * and Tailwind utilities, and the command itself wears the design's own `.code`
 * instead of rebuilding a monospace look. Given neither a command nor a note it
 * renders nothing, where the classic left an empty paragraph still holding its
 * top margin. Commands are long and dialogs are narrow, so the line wraps
 * rather than truncating — a preview you cannot finish reading is not one. (#318)
 */
export function KubectlPreview({ command, note, onCopy, className }: KubectlPreviewProps) {
  if (!filled(command)) {
    if (!filled(note)) return null;
    return <p className={cx("mt-2 text-xs text-muted", className)}>{note}</p>;
  }
  return (
    <p className={cx("mt-2 flex flex-wrap items-center gap-1 text-xs text-muted", className)}>
      <span>Equivalent kubectl:</span>
      <code className="code min-w-0 break-words">{command}</code>
      {onCopy && <IconButton icon={CopyGlyph} label="Copy kubectl command" onClick={onCopy} />}
    </p>
  );
}
