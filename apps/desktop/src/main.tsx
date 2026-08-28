import "./styles/globals.css"; // Tailwind + shadcn tokens — must load first.
import React from "react";
import { createRoot } from "react-dom/client";
import AppGate from "./AppGate";
import { TooltipProvider } from "./components/ui/tooltip";
import { applyPersistedTimeout } from "./lib/requestTimeout";
import { isTauri } from "./transport/platform";
import { initializeSettingsStorage } from "./lib/settingsStorage";

// Re-apply the persisted request timeout to the backend, which resets to its
// default each launch. Fire-and-forget: it resolves well before the user picks
// a context and triggers the first capability call. Tauri-only: on web this
// would race an unauthenticated 401 before the login gate resolves.
const container = document.getElementById("root");
if (!container) throw new Error("Root element #root not found");

// The container is passed in rather than captured: `bootstrap` is a hoisted
// function declaration, so TypeScript cannot rely on the null check above
// having run before a call and keeps the type as HTMLElement | null inside.
async function bootstrap(root: HTMLElement): Promise<void> {
  await initializeSettingsStorage();
  if (isTauri()) void applyPersistedTimeout();
  // One tooltip provider for the whole window: every tooltip shares its open
  // delay and, once one has shown, its neighbours open at once for a moment
  // (the "sweep across the toolbar" the native `title` never allowed, #376).
  // The delay values are the provider's defaults — see tooltip.tsx for why.
  createRoot(root).render(
    <TooltipProvider>
      <AppGate />
    </TooltipProvider>,
  );
}

void bootstrap(container);
