import React, { useEffect, useState } from "react";
import { Bot, Moon, Settings, Sun, Wrench } from "lucide-react";
import { listContexts, type ClusterContext } from "@srelens/core";
import {
  type Theme,
} from "../ui";
import { ContextAvatar } from "./ContextAvatar";
import { contextDisplayName, orderContexts, type ContextProfiles } from "@srelens/core";
import { TitleTooltip } from "@/components/ui/tooltip";

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
  onOpenToolbox,
  onOpenAssistant,
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
  onOpenToolbox?: () => void;
  onOpenAssistant?: () => void;
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

  const renderItem = (c: ClusterContext) => {
    const profile = contextProfiles[c.name];
    const displayName = contextDisplayName(c.name, profile);
    return (
      <TitleTooltip key={c.name} title={c.isLocal ? `${displayName} (local)` : displayName}>
        <button
          className={`fl-hotbar__item${openContext === c.name ? " fl-hotbar__item--active" : ""}`}
          aria-label={c.isLocal ? `${displayName} (local)` : displayName}
          onClick={() => onOpenContext(c.name)}
        >
          <ContextAvatar context={c.name} profile={profile} />
        </button>
      </TitleTooltip>
    );
  };

  // Keep local dev clusters visually separated from the rest so they don't get
  // mixed up with remote/production contexts.
  const remoteContexts = contexts.filter((c) => !c.isLocal);
  const localContexts = contexts.filter((c) => c.isLocal);

  return (
    // A navigation landmark, named: it is the app's other nav region, and
    // "Clusters" is how a screen-reader user tells the two apart.
    <nav className="fl-hotbar" aria-label="Clusters">
      {/* Only the cluster list scrolls — with many contexts it must never
          push the fixed controls below (toolbox, assistant, settings) out
          of the window. */}
      <div className="fl-hotbar__clusters">
        {remoteContexts.map(renderItem)}
        {remoteContexts.length > 0 && localContexts.length > 0 && (
          <div className="fl-hotbar__divider" role="separator" aria-label="Local clusters" />
        )}
        {localContexts.map(renderItem)}
      </div>
      <TitleTooltip title="Toggle light/dark mode">
        <button
          className="fl-hotbar__theme"
          aria-label={theme.mode === "dark" ? "Switch to light mode" : "Switch to dark mode"}
          onClick={onToggleTheme}
        >
          {theme.mode === "dark" ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
        </button>
      </TitleTooltip>
      {onOpenToolbox && (
        <TitleTooltip title="Open toolbox">
          <button className="fl-hotbar__theme" aria-label="Open toolbox" onClick={onOpenToolbox}>
            <Wrench aria-hidden="true" />
          </button>
        </TitleTooltip>
      )}
      {onOpenAssistant && (
        <TitleTooltip title="Open assistant">
          <button className="fl-hotbar__theme" aria-label="Open assistant" onClick={onOpenAssistant}>
            <Bot aria-hidden="true" />
          </button>
        </TitleTooltip>
      )}
      <TitleTooltip title="Open settings">
        <button className="fl-hotbar__theme" aria-label="Open settings" onClick={onOpenSettings}>
          <Settings aria-hidden="true" />
        </button>
      </TitleTooltip>
    </nav>
  );
}
