import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  extensionEnabledFor,
  isTauri,
  openExtensionView,
  providersFor,
  startExtensionLogStream,
  type ExtensionStreamEnd,
  type ExtensionView,
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

/** The log providers the log view may offer, and whether that list is current. */
export interface LogProviders {
  /**
   * `ready` once the inventory and the cluster listing have answered. While either is
   * loading, or after either failed, `choices` are the last ready ones, so a refresh
   * that fails does not read as every app removed.
   */
  status: "loading" | "ready" | "error";
  choices: LogProviderChoice[];
  error?: string;
}

/**
 * The log providers of every enabled app, enabled for `context`, declared for
 * `resourceKind` (a qualified kind: `/Pod`, `apps/Deployment`), in inventory and
 * manifest order. None on the web, which runs no app streams yet: a follow is one.
 * The host checks the same again when a stream opens.
 */
export function useLogProviders(context: string, resourceKind: string | undefined): LogProviders {
  const inventory = useExtensions();
  const lookup = useContextLookup(context);
  const offered = !!resourceKind && isTauri();
  const status: LogProviders["status"] = !offered ? "ready"
    : inventory.status === "error" || lookup.status === "failed" ? "error"
    : inventory.status === "ready" && lookup.status !== "loading" ? "ready"
    : "loading";
  const error = !offered ? undefined
    : inventory.status === "error" ? inventory.error
    : lookup.status === "failed" ? lookup.error
    : undefined;
  const contextKey = lookup.status === "found" ? lookup.id : undefined;
  const plugins = inventory.status === "ready" ? inventory.data?.plugins : undefined;
  const fresh = useMemo(() => {
    if (status !== "ready") return undefined;
    if (!offered || !plugins) return [];
    return plugins
      .filter((plugin) => plugin.enabled && !plugin.quarantined && !plugin.policyBlocked && extensionEnabledFor(plugin, contextKey))
      .flatMap((plugin) =>
        providersFor(plugin.manifest, "logs", resourceKind!).map((provider) => ({
          key: `${plugin.manifest.id}/${provider.id}`,
          appId: plugin.manifest.id,
          revision: plugin.revision,
          provider: provider.id,
          label: `${provider.title} · ${extensionLabel(plugin)}`,
        })),
      );
  }, [status, offered, plugins, contextKey, resourceKind]);
  const [kept, setKept] = useState<LogProviderChoice[]>([]);
  if (fresh && fresh !== kept) setKept(fresh);
  return { status, choices: fresh ?? kept, error };
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
   * status, so the readout counts one source; none until its view is open, and
   * `undefined` for Kubernetes.
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
  const [opened, setOpened] = useState<{ identity: string; view: ExtensionView } | null>(null);
  const [end, setEnd] = useState<ExtensionStreamEnd | null>(null);
  const [attempt, setAttempt] = useState(0);
  // Which open an ending may speak for: only the latest of the current view. The host
  // ends a stream it was told to stop (`cancelled`) or whose view closed (`viewClosed`)
  // after the next one is open, and that ending is not the next stream's.
  const opens = useRef(0);
  useEffect(() => {
    if (!choice) return;
    const view = openExtensionView(choice.appId, `logs:${choice.provider}`);
    setOpened({ identity, view });
    return () => {
      opens.current += 1;
      setEnd(null);
      void view.close();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity]);
  const view = opened?.identity === identity ? opened.view : null;
  // No targets until the view is open, so nothing is followed for that render: not the
  // provider, which has no view yet, and not the cluster under the provider's name.
  const targets = useMemo<LogTarget[] | undefined>(
    () => (choice ? (view ? [{ pod: subject.name, label: choice.provider }] : []) : undefined),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [identity, view, subject.name],
  );
  const retry = useCallback(() => {
    setEnd(null);
    setAttempt((count) => count + 1);
  }, []);
  const { context, namespace, resourceKind, name } = subject;
  const source = useMemo<UseLogStreamOptions["source"]>(() => {
    if (!choice || !view) return undefined;
    const open: LogSourceOpener = (_targets, onLine, onStatus, options) => {
      const id = ++opens.current;
      setEnd(null);
      return startExtensionLogStream(
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
        { onEnd: (ending) => id === opens.current && setEnd(ending) },
        options,
      );
    };
    return { key: `provider:${identity}/${resourceKind}/${name}/${attempt}`, open };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, identity, context, namespace, resourceKind, name, attempt]);
  return { source, targets, end: choice ? end : null, retry };
}
