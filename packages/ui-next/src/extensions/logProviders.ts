import { useCallback, useEffect, useMemo, useState } from "react";
import {
  extensionEnabledFor,
  isTauri,
  openExtensionView,
  providersFor,
  startExtensionLogStream,
  type ExtensionStreamEnd,
  type LogTarget,
} from "@srelens/core";
import type { LogSourceOpener, UseLogStreamOptions } from "../lib/logStream";
import { useContextLookup } from "./contextIds";
import { extensionLabel, useExtensions } from "./inventoryStore";

/**
 * Log providers (#569) as sources of the log view: which installed apps' providers
 * it may offer, and the `source` option that follows one through `useLogStream`,
 * so a provider's lines land in the same buffer, pause and readout as the
 * cluster's own.
 */

/** One log provider the log view can follow. */
export interface LogProviderChoice {
  /** `<app id>/<provider id>`, unique among the choices. */
  key: string;
  appId: string;
  /** The installed revision the choice was listed from; the host refuses any other. */
  revision: number;
  /** The provider's `id`. */
  provider: string;
  /** "Loki · Observability": the provider's title, then the app's. */
  label: string;
}

/**
 * The log providers of every enabled app, enabled for `context`, declared for
 * `resourceKind` (a qualified kind: `/Pod`, `apps/Deployment`), in inventory and
 * manifest order. None while the inventory has not loaded, and none on the web, which
 * runs no app streams yet: a follow is one. The host checks the same again when a
 * stream opens.
 */
export function useLogProviders(context: string, resourceKind: string | undefined): LogProviderChoice[] {
  const inventory = useExtensions();
  const lookup = useContextLookup(context);
  const contextKey = lookup.status === "found" ? lookup.id : undefined;
  const plugins = inventory.status === "ready" ? inventory.data?.plugins : undefined;
  return useMemo(() => {
    if (!resourceKind || !plugins || !isTauri()) return [];
    return plugins
      .filter((plugin) => plugin.enabled && !plugin.quarantined && !plugin.policyBlocked && extensionEnabledFor(plugin, contextKey))
      .flatMap((plugin) =>
        providersFor(plugin.manifest, "logs", resourceKind).map((provider) => ({
          key: `${plugin.manifest.id}/${provider.id}`,
          appId: plugin.manifest.id,
          revision: plugin.revision,
          provider: provider.id,
          label: `${provider.title} · ${extensionLabel(plugin)}`,
        })),
      );
  }, [plugins, contextKey, resourceKind]);
}

/** The resource the log view follows. */
export interface LogProviderSubject {
  context: string;
  namespace: string;
  /** Its qualified kind: `/Pod`, `apps/Deployment`. */
  resourceKind: string;
  name: string;
}

export interface LogProviderSource {
  /** For `useLogStream`; `undefined` while the view follows Kubernetes. */
  source?: UseLogStreamOptions["source"];
  /**
   * The one target a provider's stream reports on, tagged as the host tags its
   * status, so the readout counts one source; `undefined` for Kubernetes.
   */
  targets?: LogTarget[];
  /** How the provider's stream ended, when it has: a close is an ending, an error a failure. */
  end: ExtensionStreamEnd | null;
  /** Follow again after an ending. */
  retry: () => void;
}

/**
 * Follow `choice` for `subject`: one app view per chosen provider, closed when the
 * choice changes and when the log view goes, which ends every stream it opened.
 */
export function useLogProviderSource(choice: LogProviderChoice | undefined, subject: LogProviderSubject): LogProviderSource {
  const identity = choice ? `${choice.key}#${choice.revision}` : "";
  const [ended, setEnded] = useState<{ identity: string; end: ExtensionStreamEnd } | null>(null);
  const [attempt, setAttempt] = useState(0);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const view = useMemo(() => (choice ? openExtensionView(choice.appId, `logs:${choice.provider}`) : null), [identity]);
  useEffect(() => () => void view?.close(), [view]);
  const targets = useMemo<LogTarget[] | undefined>(
    () => (choice ? [{ pod: subject.name, label: choice.provider }] : undefined),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [identity, subject.name],
  );
  const retry = useCallback(() => {
    setEnded(null);
    setAttempt((count) => count + 1);
  }, []);
  const { context, namespace, resourceKind, name } = subject;
  const source = useMemo<UseLogStreamOptions["source"]>(() => {
    if (!choice || !view) return undefined;
    const open: LogSourceOpener = (_targets, onLine, onStatus, options) =>
      startExtensionLogStream(
        view,
        {
          id: choice.appId,
          revision: choice.revision,
          context,
          namespace,
          source: { kind: "logProvider", provider: choice.provider, resourceKind, name },
        },
        onLine,
        onStatus,
        { onEnd: (end) => setEnded({ identity, end }) },
        options,
      );
    return { key: `provider:${identity}/${resourceKind}/${name}/${attempt}`, open };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, identity, context, namespace, resourceKind, name, attempt]);
  return {
    source,
    targets,
    end: ended && ended.identity === identity ? ended.end : null,
    retry,
  };
}
