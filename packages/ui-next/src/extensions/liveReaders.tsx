// Live app views (#566): pages, table columns and dashboard cards follow the
// kinds their readers list, through `watch` app streams, instead of waiting for
// a Refresh. A watch never carries data: it says the kind changed, and the view
// reads again through the path it already uses. See docs/extensions/streams.md.
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  describeStreamEnd,
  isExtensionWatchEvent,
  isTauri,
  openExtensionView,
  type ExtensionStream,
  type ExtensionView,
  type InstalledExtension,
} from "@srelens/core";

/**
 * What a live view may claim about what it shows. Only `live` is current;
 * `reconnecting` means what is shown may be out of date; `stopped` and `off`
 * mean it is as fresh as the last read, and Refresh is the only way on.
 */
export type LiveState =
  | { state: "connecting" }
  | { state: "live" }
  | { state: "reconnecting"; message: string }
  | { state: "stopped"; message: string }
  | { state: "off"; reason: string };

const WEB = "The web app has no live app updates yet; it reads on Refresh.";

/**
 * Why nothing is followed, from the actual cause, or `null` when something
 * is. The web reason is given only on the web.
 */
function offReason(context: string, readers: number, off?: string): string | null {
  if (!isTauri()) return WEB;
  if (off) return off;
  if (!context) return "Choose a cluster first.";
  if (readers === 0) return "Nothing in this view follows a reader.";
  return null;
}

const initial = (reason: string | null): LiveState =>
  reason === null ? { state: "connecting" } : { state: "off", reason };

type Listener = { onChange(): void; onState(state: LiveState): void };
type Entry = { state: LiveState; listeners: Set<Listener>; stream: Promise<ExtensionStream | null> };
type Ask = { revision: number; context: string; namespace: string; capability: string };

/** One view's watches, one per reader it shows, however many parts of the view show it. */
export class LiveHub {
  private view?: ExtensionView;
  private entries = new Map<string, Entry>();
  constructor(readonly appId: string, private readonly label: string) {}

  watch(ask: Ask, listener: Listener): () => void {
    const key = JSON.stringify([ask.revision, ask.context, ask.namespace, ask.capability]);
    let entry = this.entries.get(key);
    if (!entry) {
      entry = this.start(ask);
      this.entries.set(key, entry);
    }
    const mine = entry;
    mine.listeners.add(listener);
    listener.onState(mine.state);
    return () => {
      mine.listeners.delete(listener);
      if (mine.listeners.size || this.entries.get(key) !== mine) return;
      this.entries.delete(key);
      void mine.stream.then((stream) => stream?.cancel()).catch(() => {});
    };
  }

  /** End every watch this view opened, on the host, in one call. */
  close() {
    const view = this.view;
    this.view = undefined;
    this.entries.clear();
    void view?.close().catch(() => {});
  }

  private start(ask: Ask): Entry {
    this.view ??= openExtensionView(this.appId, this.label);
    const entry: Entry = { state: { state: "connecting" }, listeners: new Set(), stream: Promise.resolve(null) };
    const set = (state: LiveState) => {
      entry.state = state;
      for (const listener of [...entry.listeners]) listener.onState(state);
    };
    entry.stream = this.view
      .open(
        { id: this.appId, revision: ask.revision, context: ask.context, namespace: ask.namespace, source: { kind: "watch", capability: ask.capability } },
        {
          onData: (data) => {
            if (!isExtensionWatchEvent(data)) return;
            if (data.event === "reconnecting") {
              set({ state: "reconnecting", message: data.message });
              return;
            }
            set({ state: "live" });
            for (const listener of [...entry.listeners]) listener.onChange();
          },
          onEnd: (end) => set({ state: "stopped", message: describeStreamEnd(end) }),
        },
      )
      .catch((error: unknown) => {
        set({ state: "stopped", message: error instanceof Error ? error.message : String(error) });
        return null;
      });
    return entry;
  }
}

const HubContext = createContext<LiveHub | null>(null);

/** One view: the watches of everything inside it end when it unmounts. */
export function LiveReaders({ plugin, label, children }: { plugin: InstalledExtension; label: string; children: ReactNode }) {
  // A view is one app's: handed another app, it is another view.
  const id = plugin.manifest.id;
  const hub = useMemo(() => new LiveHub(id, label), [id, label]);
  useEffect(() => () => hub.close(), [hub]);
  return <HubContext.Provider value={hub}>{children}</HubContext.Provider>;
}

const RANK: Record<LiveState["state"], number> = { live: 0, connecting: 1, reconnecting: 2, stopped: 3, off: 4 };

/**
 * Follow `capabilities` — declared readers of `plugin` — in `context` and
 * `namespace`, calling `onChange` whenever what they would answer may have
 * changed. Answers the least current of their states. Inside `LiveReaders` the
 * watches belong to that view; outside, to this component.
 */
export function useLiveReaders(args: {
  plugin: InstalledExtension;
  capabilities: string[];
  context: string;
  namespace: string;
  label: string;
  onChange(): void;
}): LiveState {
  const { plugin, capabilities, context, namespace, label } = args;
  const id = plugin.manifest.id;
  // The enclosing view's hub follows its own app only; another app's reader gets a hub of its own.
  const enclosing = useContext(HubContext);
  const provided = enclosing?.appId === id ? enclosing : null;
  const own = useMemo(() => (provided ? null : new LiveHub(id, label)), [provided, id, label]);
  const hub = provided ?? own!;
  useEffect(() => () => own?.close(), [own]);
  const onChange = useRef(args.onChange);
  onChange.current = args.onChange;
  const reason = offReason(context, capabilities.length);
  const [live, setLive] = useState<LiveState>(() => initial(reason));
  const readers = [...new Set(capabilities)].sort().join("\u0000");
  useEffect(() => {
    if (reason !== null) {
      setLive({ state: "off", reason });
      return;
    }
    const states = new Map<string, LiveState>();
    const publish = () => {
      let worst: LiveState = { state: "live" };
      for (const state of states.values()) if (RANK[state.state] > RANK[worst.state]) worst = state;
      setLive(worst);
    };
    const stops = readers.split("\u0000").map((capability) => {
      states.set(capability, { state: "connecting" });
      return hub.watch(
        { revision: plugin.revision, context, namespace, capability },
        {
          onChange: () => onChange.current(),
          onState: (state) => { states.set(capability, state); publish(); },
        },
      );
    });
    publish();
    return () => { for (const stop of stops) stop(); };
  }, [hub, reason, id, plugin.revision, context, namespace, readers]);
  return live;
}

/**
 * {@link useLiveReaders} over several apps at once — the apps contributing
 * columns to one table — each app's watches in a view of its own, ended when
 * the component unmounts or the app leaves the list.
 */
export function useLiveApps(args: {
  apps: Array<{ plugin: InstalledExtension; capabilities: string[] }>;
  context: string;
  namespace: string;
  label: string;
  /** Called with the app whose reader changed. */
  onChange(app: string): void;
  /** Why the caller follows nothing yet, on the desktop: shown as the reason. */
  off?: string;
}): LiveState {
  const { apps, context, namespace, label, off } = args;
  const onChange = useRef(args.onChange);
  onChange.current = args.onChange;
  const hubs = useRef(new Map<string, LiveHub>());
  useEffect(() => () => {
    for (const hub of hubs.current.values()) hub.close();
    hubs.current.clear();
  }, []);
  const wanted = apps
    .filter((app) => app.capabilities.length > 0)
    .map((app) => [app.plugin.manifest.id, app.plugin.revision, [...new Set(app.capabilities)].sort()] as const);
  const signature = JSON.stringify(wanted);
  const reason = offReason(context, wanted.length, off);
  const [live, setLive] = useState<LiveState>(() => initial(reason));
  useEffect(() => {
    if (reason !== null) {
      setLive({ state: "off", reason });
      return;
    }
    const current = new Set(wanted.map(([id]) => id));
    for (const [id, hub] of hubs.current) {
      if (!current.has(id)) { hub.close(); hubs.current.delete(id); }
    }
    const states = new Map<string, LiveState>();
    const publish = () => {
      let worst: LiveState = { state: "live" };
      for (const state of states.values()) if (RANK[state.state] > RANK[worst.state]) worst = state;
      setLive(worst);
    };
    const stops = wanted.flatMap(([id, revision, capabilities]) => {
      let hub = hubs.current.get(id);
      if (!hub) { hub = new LiveHub(id, label); hubs.current.set(id, hub); }
      const own = hub;
      return capabilities.map((capability) => {
        const key = `${id}\u0000${capability}`;
        states.set(key, { state: "connecting" });
        return own.watch(
          { revision, context, namespace, capability },
          { onChange: () => onChange.current(id), onState: (state) => { states.set(key, state); publish(); } },
        );
      });
    });
    publish();
    return () => { for (const stop of stops) stop(); };
    // `wanted` is `signature`, parsed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reason, signature, context, namespace, label]);
  return live;
}

/** The state, in words: colour is never the only signal. */
export function LiveStatus({ live }: { live: LiveState }) {
  const [word, detail] =
    live.state === "live" ? ["Live", "Updates as the cluster changes"]
      : live.state === "connecting" ? ["Connecting…", "Starting live updates"]
        : live.state === "reconnecting" ? ["Reconnecting…", `Lost the watch: ${live.message}`]
          : ["Not live", live.state === "off" ? live.reason : `Live updates stopped: ${live.message}`];
  return (
    <span className="extension-live" data-live={live.state} title={detail} role="status">
      <span className="extension-live-dot" aria-hidden="true" />
      {word}
    </span>
  );
}

/**
 * What a view says when what it shows is not current: while reconnecting, that
 * it may be out of date; once stopped, why, and that Refresh reads again.
 */
export function LiveNotice({ live, what }: { live: LiveState; what: string }) {
  if (live.state === "reconnecting")
    return (
      <p className="extension-message extension-live-notice extension-stale-notice" role="status">
        Reconnecting to the cluster ({live.message}). The {what} below may be out of date until it reconnects.
      </p>
    );
  if (live.state === "stopped")
    return (
      <p className="extension-message extension-live-notice" role="status">
        Live updates stopped: {live.message} Showing the {what} as of the last read; use Refresh to read again.
      </p>
    );
  return null;
}
