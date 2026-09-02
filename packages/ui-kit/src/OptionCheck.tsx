import { cx } from "./cx";

/**
 * The design's mark for "this is the one" in a list where a single option
 * wins — a popover of workspaces, of agents, of values.
 *
 * **Always rendered, never removed.** The labels beside it line up in a column,
 * and a check that appears on selection would shove its own row sideways. Off
 * is transparent, not absent.
 *
 * Written out three times before this — `PickerRow`, `WorkspaceSwitcher`, and
 * a fourth call site that hand-rolled a `hover:bg-sunk` list with no mark at
 * all, which is what got reported ("use same ones used in the project"). The
 * glyph, its size, its colour and the alignment rule live here once.
 */
export function OptionCheck({ checked, className }: { checked: boolean; className?: string }) {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className={cx("shrink-0", checked ? "opacity-100" : "opacity-0", className)}
      style={{ color: "var(--accent)" }}
    >
      <path
        d="M20 6 9 17l-5-5"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
