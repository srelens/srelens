import React, { useEffect, useState } from "react";
import { Moon, Settings, Sun } from "lucide-react";
import { listContexts, type ClusterContext } from "../lib/clusters";
import {
  type Theme,
} from "../ui";
import { ContextAvatar } from "./ContextAvatar";
import { contextDisplayName, orderContexts, type ContextProfiles } from "../lib/settings";

const EMPTY_LIST: string[] = [];

/**
 * Far-left vertical strip of srelens cluster avatars. Click an
 * avatar to enter that cluster.
 */
export function ClusterHotbar({
  openContext,
  onOpenContext,
  theme,
  onToggleTheme,
  onOpenSettings,
  contextProfiles = {},
  kubeconfigFiles = EMPTY_LIST,
  contextOrder = EMPTY_LIST,
  contexts: passedContexts,
}: {
  openContext: string | null;
  onOpenContext: (context: string) => void;
  theme: Theme;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  contextProfiles?: ContextProfiles;
  kubeconfigFiles?: string[];
  contextOrder?: string[];
  contexts?: ClusterContext[];
}) {
  const [internalContexts, setInternalContexts] = useState<ClusterContext[]>([]);

  useEffect(() => {
    if (passedContexts) return;
    let active = true;
    void listContexts(kubeconfigFiles).then((o) => {
      if (active && o.contexts) setInternalContexts(orderContexts(o.contexts, contextOrder));
    });
    return () => {
      active = false;
    };
  }, [contextOrder, kubeconfigFiles, passedContexts]);

  const contexts = passedContexts
    ? orderContexts(passedContexts, contextOrder)
    : internalContexts;

  return (
    <div className="fl-hotbar">
      {contexts.map((c) => (
        (() => {
          const profile = contextProfiles[c.name];
          const displayName = contextDisplayName(c.name, profile);
          return (
        <button
          key={c.name}
          className={`fl-hotbar__item${openContext === c.name ? " fl-hotbar__item--active" : ""}`}
          title={displayName}
          aria-label={displayName}
          onClick={() => onOpenContext(c.name)}
        >
          <ContextAvatar context={c.name} profile={profile} />
        </button>
          );
        })()
      ))}
      <div className="fl-hotbar__spacer" aria-hidden="true" />
      <button
        className="fl-hotbar__theme"
        aria-label={theme.mode === "dark" ? "Switch to light mode" : "Switch to dark mode"}
        title="Toggle light/dark mode"
        onClick={onToggleTheme}
      >
        {theme.mode === "dark" ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
      </button>
      <button
        className="fl-hotbar__theme"
        aria-label="Open settings"
        title="Open settings"
        onClick={onOpenSettings}
      >
        <Settings aria-hidden="true" />
      </button>
    </div>
  );
}
