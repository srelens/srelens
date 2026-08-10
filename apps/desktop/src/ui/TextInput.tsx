import React from "react";
import { Input } from "@/components/ui/input";

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
  "aria-label"?: string;
}

/**
 * Themed single-line text input with a value-first onChange contract. Local
 * wrapper over shadcn's Input — the value-first API and onEnter are preserved
 * so callers don't depend on the underlying library.
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
  "aria-label": ariaLabel,
}: TextInputProps) {
  return (
    <Input
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
      className={className}
      aria-label={ariaLabel}
    />
  );
}
