import type { InputHTMLAttributes } from "react";
import { cx } from "./cx";

export interface TextInputProps
  extends Omit<
    InputHTMLAttributes<HTMLInputElement>,
    "value" | "onChange" | "onKeyDown" | "type" | "style" | "aria-invalid"
  > {
  value: string;
  onValueChange: (value: string) => void;
  /** Called when the user presses Enter, which then goes no further: not to a form around the field. */
  onEnter?: () => void;
  /** Called when the user presses Escape, which then goes no further. */
  onEscape?: () => void;
  type?: "text" | "search" | "number" | "password" | "url";
  /** Marks the field as failing validation, in colour and to assistive tech. */
  invalid?: boolean;
}

/**
 * Single-line text input with a value-first change contract.
 *
 * The classic component wrapped shadcn's Input; the value-first API, `onEnter`
 * and `onEscape` are what callers depend on and survive unchanged. (#318)
 *
 * The input's own attributes pass through — a bound, a name from a legend, a
 * focus handler — so a typed field needs no bare `<input>` beside it. (#1001)
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
  className,
  type = "text",
  invalid,
  ...rest
}: TextInputProps) {
  return (
    <input
      {...rest}
      value={value}
      onChange={(e) => onValueChange(e.target.value)}
      onKeyDown={(e) => {
        // Mid-composition, Enter picks an input method's candidate and Escape drops one.
        if (e.nativeEvent.isComposing) return;
        const handler = e.key === "Enter" ? onEnter : e.key === "Escape" ? onEscape : undefined;
        if (!handler) return;
        e.preventDefault();
        handler();
      }}
      type={type}
      aria-invalid={invalid ? true : undefined}
      className={cx("w-full rounded-md border px-2 py-1 text-[0.8125rem] placeholder:text-faint", className)}
      style={{
        background: "var(--surface-sunk)",
        borderColor: invalid ? "var(--sev)" : "var(--control-line)",
      }}
    />
  );
}
