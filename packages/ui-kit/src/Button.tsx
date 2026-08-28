import type { ButtonHTMLAttributes } from "react";
import { cx } from "./cx";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "outline";

export type ButtonSize =
  | "xs"
  | "sm"
  | "default"
  | "lg"
  | "icon"
  | "icon-xs"
  | "icon-sm"
  | "icon-lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

/**
 * The design's button.
 *
 * The classic component wrapped shadcn's, which cannot come along: it lives in
 * `apps/desktop` and is written against the classic Tailwind config. The API is
 * what callers depend on, so `variant` and `size` survive unchanged and only
 * the appearance moves. (#318)
 *
 * `secondary` and `outline` both render the plain `.btn` — the new design has
 * one bordered surface button where shadcn had two. `data-variant` keeps the
 * caller's intent on the element so the stylesheet can separate them later
 * without touching a single call site.
 *
 * The size is an attribute rather than a set of utility classes because the
 * kit's stylesheet owns the appearance; see `tokens-only.test.ts`.
 */
export function Button({ variant = "primary", size = "default", className, ...rest }: ButtonProps) {
  return (
    <button
      data-variant={variant}
      data-size={size}
      className={cx(
        "btn",
        variant === "primary" && "btn-accent",
        variant === "danger" && "btn-danger",
        variant === "ghost" && "btn-ghost",
        className,
      )}
      {...rest}
    />
  );
}
