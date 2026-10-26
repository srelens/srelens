import { useSyncExternalStore } from "react";
import {
  ASSISTANT_KEY,
  assistantAllowedFor,
  flushSettingsWrites,
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
 * **The file is the truth, not this window.** The MCP gate reads the settings
 * file and nothing else, so a switch that showed "off" over a file still
 * saying "on" would be the one lie this feature cannot tell. A change shows at
 * once, as a change waiting on top of what is stored; the save then reads the
 * setting again (another window may have changed it — `refreshSetting`'s own
 * rule for list writers), writes the change against that, and waits for the
 * backend to confirm. A save that fails is dropped and the setting read again,
 * so the switch shows what the file holds, with the error beside it — the
 * shape `collections.ts` keeps for its list.
 */
type Change = (setting: AssistantSetting) => AssistantSetting;

/** Changes shown but not yet saved, oldest first. */
let waiting: Change[] = [];
let saving: Promise<void> = Promise.resolve();
let saveError: unknown = null;
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
  return waiting.reduce((setting, change) => change(setting), readAssistantSetting());
}

/** Why the last save did not land, or `null`; cleared by the next one that does. */
export function getAssistantSaveError(): unknown {
  return saveError;
}

/** Resolves when every change made so far has been saved or dropped. */
export function assistantSaved(): Promise<void> {
  return saving;
}

/** Forget a save error and anything waiting, between tests. */
export function __resetAssistantForTests(): void {
  waiting = [];
  saving = Promise.resolve();
  saveError = null;
}

function save(change: Change): Promise<void> {
  waiting.push(change);
  changed();
  saving = saving.then(async () => {
    try {
      await refreshSetting(ASSISTANT_KEY);
      writeAssistantSetting(change(readAssistantSetting()));
      await flushSettingsWrites({ throwOnError: true });
      saveError = null;
    } catch (error) {
      saveError = error;
      // Show what the file holds, not what was asked for.
      await refreshSetting(ASSISTANT_KEY).catch((cause) => console.warn("could not re-read the AI Assistant setting", cause));
    }
    waiting = waiting.filter((w) => w !== change);
    changed();
  });
  return saving;
}

export function setAssistantEnabled(on: boolean): Promise<void> {
  return save((setting) => ({ ...setting, enabled: on }));
}

export function setClusterAssistant(stableId: string, allowed: boolean): Promise<void> {
  return save((setting) => {
    const others = setting.disabledClusters.filter((id) => id !== stableId);
    return { ...setting, disabledClusters: allowed ? others : [...others, stableId] };
  });
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
