import { useEffect, useState } from "react";
import { isTauri, listAgents, settingsStorage } from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { useExtensions } from "../../extensions/inventoryStore";
import { useAgentInventoryVersion } from "../../lib/agentInventory";
import { useContexts, useContextsStatus } from "../../lib/clusters";
import { openSettings } from "../../lib/settingsRequest";
import { openTab } from "../../lib/tabsStore";
import { useCanLockWorkspace } from "../../shell/LockGate";

/** Where the dismissal lives: the settings file on the desktop, `localStorage` on the web. */
export const CHECKLIST_DISMISSED_KEY = "srelens.next.homeChecklistDismissed";

/** `checking` until a read answers; `unknown` when it refused — never counted as done. */
type Known = "done" | "todo" | "unknown" | "checking";

interface Step {
  name: string;
  state: Known;
  action: { label: string; run: () => void };
}

// Guarded like every settings accessor here: a refusing storage costs the
// dismissal and nothing else.
function readDismissed(): boolean {
  try {
    return settingsStorage.getItem(CHECKLIST_DISMISSED_KEY) === "true";
  } catch {
    return false;
  }
}
function saveDismissed(): void {
  try {
    settingsStorage.setItem(CHECKLIST_DISMISSED_KEY, "true");
  } catch (error) {
    console.error("could not persist the dismissed getting-started list", error);
  }
}

/**
 * "Getting started": four first steps, each ticked from real state — the
 * contexts srelens found, the vault the gate opened, the agents the inventory
 * offers, and the apps installed.
 *
 * Not drawn while any of those is still being read, so a reader who has done
 * everything never sees it flash up; drawn with "Could not check" for a read
 * that refused, because a step nobody could check is not done. Gone once every
 * step is done, or once dismissed — and the dismissal is kept.
 *
 * The vault and the assistant are desktop things; the web host lists neither.
 */
export function GettingStarted() {
  const desktop = isTauri();
  const contexts = useContexts();
  const contextsStatus = useContextsStatus();
  const vaultOpen = useCanLockWorkspace();
  const inventory = useExtensions();
  const agentsVersion = useAgentInventoryVersion();
  const [assistant, setAssistant] = useState<Known>("checking");
  const [dismissed, setDismissed] = useState(readDismissed);

  useEffect(() => {
    if (!desktop || dismissed) return;
    let alive = true;
    listAgents().then(
      (agents) => { if (alive) setAssistant(agents.some((a) => a.available && !a.gated) ? "done" : "todo"); },
      () => { if (alive) setAssistant("unknown"); },
    );
    return () => {
      alive = false;
    };
  }, [desktop, dismissed, agentsVersion]);

  if (dismissed) return null;
  const steps: Step[] = [
    {
      name: "Connect a cluster",
      // A listing that failed — even one that found some contexts — is not a fact about what is set up.
      state: contextsStatus === "loading" ? "checking" : contextsStatus === "failed" ? "unknown" : contexts.length > 0 ? "done" : "todo",
      action: { label: "Connect", run: () => openTab("/connect") },
    },
    ...(desktop
      ? [
          { name: "Protect the workspace", state: (vaultOpen ? "done" : "todo") as Known, action: { label: "Security", run: () => openSettings("security") } },
          { name: "Set up the assistant", state: assistant, action: { label: "Set up", run: () => openSettings("agent") } },
        ]
      : []),
    {
      name: "Install an app",
      state: inventory.status === "loading" ? "checking" : inventory.status === "error" ? "unknown" : (inventory.data?.plugins.length ?? 0) > 0 ? "done" : "todo",
      action: { label: "Browse apps", run: () => openSettings("extensions", "catalog") },
    },
  ];
  if (steps.some((s) => s.state === "checking") || steps.every((s) => s.state === "done")) return null;
  const done = steps.filter((s) => s.state === "done").length;

  return (
    <section className="home-side-section" aria-labelledby="home-start-title">
      <div className="home-section-heading">
        <h2 id="home-start-title">Getting started <span className="text-muted">{done}/{steps.length}</span></h2>
        <Button variant="ghost" size="sm" aria-label="Dismiss getting started" onClick={() => { saveDismissed(); setDismissed(true); }}>
          Dismiss
        </Button>
      </div>
      <ul className="home-side-group home-pick-list">
        {steps.map((step) => (
          <li key={step.name} className="home-live-row">
            <span aria-hidden className={step.state === "done" ? "home-check home-check-done" : "home-check"}>{step.state === "done" ? "✓" : ""}</span>
            <span className="min-w-0 flex-1 text-[0.8125rem]">{step.name}</span>
            {step.state === "done" ? (
              <span className="text-xs text-muted">Done</span>
            ) : (
              <>
                {step.state === "unknown" && <span className="text-xs text-muted">Could not check</span>}
                <Button variant="secondary" size="sm" onClick={step.action.run}>{step.action.label}</Button>
              </>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
