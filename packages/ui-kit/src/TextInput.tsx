import { cx } from "./cx";

export interface TextInputProps {
  value: string;
  onValueChange: (value: string) => void;
  /** Called when the user presses Enter. */
  onEnter?: () => void;
  /** Called when the user presses Escape. */
  onEscape?: () => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  type?: "text" | "search" | "number" | "password";
  autoFocus?: boolean;
  /** Marks the field as failing validation, in colour and to assistive tech. */
  invalid?: boolean;
  "aria-label"?: string;
}

/**
 * Single-line text input with a value-first change contract.
 *
 * The classic component wrapped shadcn's Input; the value-first API, `onEnter`
 * and `onEscape` are what callers depend on and survive unchanged. (#318)
 *
 * `aria-invalid` is set only when the field has actually been judged invalid.
 * The mock passes the flag straight through, which puts `aria-invalid="false"`
 * on every untouched field — noise that says a field has been checked and
 * passed when nothing has checked it.
 */
export function TextInput({
  value,
  onValueChange,
  onEnter,
  onEscape,
  placeholder,
  disabled,
  className,
  type = "text",
  autoFocus,
  invalid,
  "aria-label": ariaLabel,
}: TextInputProps) {
  return (
    <input
      value={value}
      onChange={(e) => onValueChange(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && onEnter) onEnter();
        if (e.key === "Escape" && onEscape) onEscape();
      }}
      placeholder={placeholder}
      disabled={disabled}
      type={type}
      autoFocus={autoFocus}
      aria-label={ariaLabel}
      aria-invalid={invalid ? true : undefined}
      className={cx("w-full rounded-md border px-2 py-1 text-[0.8125rem] outline-none", className)}
      style={{
        background: "var(--surface-sunk)",
        borderColor: invalid ? "var(--sev)" : "var(--rule)",
      }}
    />
  );
}
