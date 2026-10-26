import { useSyncExternalStore } from "react";
import {
  ASSISTANT_KEY,
  assistantAllowedFor,
  readAssistantSetting,
  refreshSetting,
  writeAssistantSetting,
  type AssistantSetting,
} from "@srelens/core";
import { getRun, getRunSubject, getRunSummaries, stopAgentRun } from "./agentRun";
import { getContexts, useContexts } from "./clusters";

/**
 * The AI Assistant switches (#968), for the new design: whether the assistant
 * is on at all, and the clusters it is off for. Core parses the key; this
 * makes it reactive and keeps open windows in step.
 *
 * Read from `settingsStorage` on every call rather than cached: it is an
 * in-memory map already, and one source means a write this window could not
 * save never shows as saved.
 */
let version = 0;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const getVersion = () => version;

/** Redraw every reader, then stop a turn whose cluster the change ruled out. */
function changed(): void {
  version += 1;
  for (const listener of listeners) listener();
  // Only one run is ever busy. The backend refuses its next tool call anyway;
  // this ends the turn instead of leaving it to fail call by call.
  const busy = getRunSummaries().find((summary) => getRun(summary.key).busy);
  if (busy && !isAssistantAllowed(getRunSubject(busy.key)?.about.cluster)) stopAgentRun();
}

export function getAssistant(): AssistantSetting {
  return readAssistantSetting();
}

export function setAssistantEnabled(on: boolean): void {
  writeAssistantSetting({ ...getAssistant(), enabled: on });
  changed();
}

export function setClusterAssistant(stableId: string, allowed: boolean): void {
  const current = getAssistant();
  const others = current.disabledClusters.filter((id) => id !== stableId);
  writeAssistantSetting({ ...current, disabledClusters: allowed ? others : [...others, stableId] });
  changed();
}

/** Whether the assistant may act on `cluster`, named any way the app names one. */
export function isAssistantAllowed(cluster: string | null | undefined): boolean {
  return assistantAllowedFor(getAssistant(), getContexts(), cluster);
}

export function useAssistant(): AssistantSetting {
  useSyncExternalStore(subscribe, getVersion, getVersion);
  return getAssistant();
}

/** Reactive {@link isAssistantAllowed}: redraws on a switch and on the context list. */
export function useAssistantAllowed(cluster: string | null | undefined): boolean {
  useSyncExternalStore(subscribe, getVersion, getVersion);
  useContexts();
  return isAssistantAllowed(cluster);
}

/**
 * Read the setting again each time this window is focused. A window loads its
 * settings once, so a switch flipped in another window would otherwise not
 * reach one already open — as `followMainProtection` does for protection.
 * Returns the release.
 */
export function followAssistantSetting(): () => void {
  const onFocus = () => {
    refreshSetting(ASSISTANT_KEY).then(changed, (error) => console.warn("could not re-read the AI Assistant setting", error));
  };
  window.addEventListener("focus", onFocus);
  return () => window.removeEventListener("focus", onFocus);
}
