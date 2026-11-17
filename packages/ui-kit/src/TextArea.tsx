import type { TextareaHTMLAttributes } from "react";
import { cx } from "./cx";

export interface TextAreaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "value" | "onChange" | "style" | "aria-invalid"> {
  value: string;
  onValueChange: (value: string) => void;
  /** Marks the field as failing validation, in colour and to assistive tech. */
  invalid?: boolean;
}

/**
 * Multi-line text box, drawn as `TextInput` draws its single line.
 *
 * Fixed-width, because every multi-line box in the app holds machine text: a
 * manifest, a kubeconfig, namespace patterns, a skill's Markdown. Before it
 * existed four screens styled their own, three different ways. (#1001)
 */
export function TextArea({ value, onValueChange, invalid, className, ...rest }: TextAreaProps) {
  return (
    <textarea
      {...rest}
      value={value}
      onChange={(e) => onValueChange(e.target.value)}
      aria-invalid={invalid ? true : undefined}
      className={cx(
        "scroll w-full resize-y rounded-md border px-2 py-1.5 font-mono text-[0.75rem] leading-relaxed placeholder:text-faint",
        className,
      )}
      style={{
        background: "var(--surface-sunk)",
        borderColor: invalid ? "var(--sev)" : "var(--control-line)",
      }}
    />
  );
}
