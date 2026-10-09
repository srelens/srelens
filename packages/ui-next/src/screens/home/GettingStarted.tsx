import { useEffect, useState } from "react";
import { flushSettingsWrites, isTauri, listAgents, settingsStorage } from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { useExtensions } from "../../extensions/inventoryStore";
import { useAgentInventoryVersion } from "../../lib/agentInventory";
import { useContexts, useContextsError, useContextsStatus } from "../../lib/clusters";
import { FailureAlert, FailureLine } from "../../lib/errorCopy";
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
  /** Why the check refused, for an `unknown` step. */
  cause?: unknown;
  /** Asks again, for an `unknown` step. */
  retry?: () => void;
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
/**
 * Keep the dismissal; the failure when it could not be kept, `null` when it was.
 * After startup `setItem` only queues the write, so this waits for the backend
 * to take it: a refusal there would otherwise come back as the list returning
 * on the next launch.
 */
async function saveDismissed(): Promise<unknown> {
  try {
    settingsStorage.setItem(CHECKLIST_DISMISSED_KEY, "true");
    await flushSettingsWrites({ throwOnError: true });
    return null;
  } catch (error) {
    // Not kept, so not remembered either: left in memory, the next mount would
    // read it back as dismissed.
    try {
      settingsStorage.removeItem(CHECKLIST_DISMISSED_KEY);
    } catch {
      // Nothing was held to undo.
    }
    console.error("could not persist the dismissed getting-started list", error);
    return error;
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
 * A step that could not be checked says why and offers to check again;
 * `retryContexts` is Home's own retry of the cluster listing.
 *
 * The vault and the assistant are desktop things; the web host lists neither.
 */
export function GettingStarted({ retryContexts }: { retryContexts?: () => void } = {}) {
  const desktop = isTauri();
  const contexts = useContexts();
  const contextsStatus = useContextsStatus();
  const contextsError = useContextsError();
  const vaultOpen = useCanLockWorkspace();
  const inventory = useExtensions();
  const agentsVersion = useAgentInventoryVersion();
  const [assistant, setAssistant] = useState<{ state: Known; error?: unknown }>({ state: "checking" });
  // Bumped by Retry. The step keeps saying what it last knew until the new answer lands.
  const [agentsAttempt, setAgentsAttempt] = useState(0);
  const [dismissed, setDismissed] = useState(readDismissed);
  // A dismissal that could not be kept would come back on the next launch, so the list stays and says so.
  const [dismissFailure, setDismissFailure] = useState<unknown>(null);
  // One save at a time: a failed save's rollback would otherwise remove the
  // value a later save, queued meanwhile, had just kept.
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!desktop || dismissed) return;
    let alive = true;
    listAgents().then(
      (agents) => { if (alive) setAssistant({ state: agents.some((a) => a.available && !a.gated) ? "done" : "todo" }); },
      (error: unknown) => { if (alive) setAssistant({ state: "unknown", error }); },
    );
    return () => {
      alive = false;
    };
  }, [desktop, dismissed, agentsVersion, agentsAttempt]);

  if (dismissed) return null;
  const steps: Step[] = [
    {
      name: "Connect a cluster",
      // A listing that failed — even one that found some contexts — is not a fact about what is set up.
      state: contextsStatus === "loading" ? "checking" : contextsStatus === "failed" ? "unknown" : contexts.length > 0 ? "done" : "todo",
      action: { label: "Connect", run: () => openTab("/connect") },
      cause: contextsError,
      retry: retryContexts,
    },
    ...(desktop
      ? [
          { name: "Protect the workspace", state: (vaultOpen ? "done" : "todo") as Known, action: { label: "Security", run: () => openSettings("security") } },
          {
            name: "Set up the assistant",
            state: assistant.state,
            action: { label: "Set up", run: () => openSettings("agent") },
            cause: assistant.error,
            retry: () => setAgentsAttempt((n) => n + 1),
          },
        ]
      : []),
    {
      name: "Install an app",
      state: inventory.status === "loading" ? "checking" : inventory.status === "error" ? "unknown" : (inventory.data?.plugins.length ?? 0) > 0 ? "done" : "todo",
      action: { label: "Browse apps", run: () => openSettings("extensions", "catalog") },
      cause: inventory.error,
      retry: inventory.reload,
    },
  ];
  if (steps.some((s) => s.state === "checking") || steps.every((s) => s.state === "done")) return null;
  const done = steps.filter((s) => s.state === "done").length;

  return (
    <section className="home-side-section" aria-labelledby="home-start-title">
      <div className="home-section-heading">
        <h2 id="home-start-title">Getting started <span className="text-muted">{done}/{steps.length}</span></h2>
        <Button
          variant="ghost"
          size="sm"
          aria-label="Dismiss getting started"
          disabled={saving}
          onClick={async () => {
            if (saving) return;
            setSaving(true);
            const failure = await saveDismissed();
            setSaving(false);
            if (failure === null) setDismissed(true);
            else setDismissFailure(failure);
          }}
        >
          Dismiss
        </Button>
      </div>
      {dismissFailure !== null && (
        <FailureAlert title="Could not keep this dismissed" error={dismissFailure} className="home-section-alert" />
      )}
      <ul className="home-side-group home-pick-list">
        {steps.map((step) => (
          <li key={step.name} className="home-live-row">
            <span aria-hidden className={step.state === "done" ? "home-check home-check-done" : "home-check"}>{step.state === "done" ? "✓" : ""}</span>
            <span className="min-w-0 flex-1 text-[0.8125rem]">
              {step.name}
              {step.state === "unknown" && step.cause !== undefined && step.cause !== "" && (
                <FailureLine error={step.cause} className="text-xs text-muted" />
              )}
            </span>
            {step.state === "done" ? (
              <span className="text-xs text-muted">Done</span>
            ) : step.state === "unknown" ? (
              <>
                <span className="text-xs text-muted">Could not check</span>
                {step.retry ? (
                  <Button variant="secondary" size="sm" aria-label={`Retry checking ${step.name}`} onClick={step.retry}>Retry</Button>
                ) : (
                  <Button variant="secondary" size="sm" onClick={step.action.run}>{step.action.label}</Button>
                )}
              </>
            ) : (
              <Button variant="secondary" size="sm" onClick={step.action.run}>{step.action.label}</Button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
