import React from "react";
import { Button as ShadButton } from "@/components/ui/button";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "outline";

// Our intents map onto shadcn's button variants.
const VARIANT: Record<ButtonVariant, "default" | "secondary" | "ghost" | "destructive" | "outline"> = {
  primary: "default",
  secondary: "secondary",
  ghost: "ghost",
  danger: "destructive",
  outline: "outline",
};

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: "xs" | "sm" | "default" | "lg" | "icon" | "icon-xs" | "icon-sm" | "icon-lg";
}

/**
 * Themed button. Local wrapper over shadcn's Button so the rest of the app
 * depends on this stable API (`variant`, `onClick`, `disabled`) rather than on
 * the underlying library — swapping it later touches only this file.
 */
export function Button({ variant = "primary", ...rest }: ButtonProps) {
  return <ShadButton variant={VARIANT[variant]} {...rest} />;
}
