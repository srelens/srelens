import { useState, useSyncExternalStore, type ReactNode } from "react";
import {
  browsable,
  forwardAddress,
  getForwards,
  isForwardEnded,
  kindToForwardTarget,
  openExternal,
  stopPortForward,
  subscribeForwards,
} from "@srelens/core";
import { Button, Eyebrow } from "@srelens/ui-kit";
import { useContexts } from "../../lib/clusters";
import { FailureAlert } from "../../lib/errorCopy";
import { openOnCluster } from "../../lib/openCluster";
import { endSession, getSessions, subscribeSessions } from "../../lib/sessions";
import { useLiveLogStreams } from "../../lib/liveLogStreams";
import { activateTab, closeTab, openTab, togglePin, useTabs } from "../../lib/tabsStore";

/**
 * "Live now": what this window is holding open — running port-forwards, open
 * shells, and logs tabs following a stream — each with a way to it and a way
 * to stop it, from the stores that already own them. Not drawn when nothing is
 * live, because an empty "Live now" is a heading over nothing.
 *
 * A log stream lives in its tab: tabs stay mounted behind the one on screen,
 * so the stream runs while the tab is open and stops when it closes, which is
 * what Stop does. Only a tab whose stream is actually running is listed — the
 * stream marks its own tab (`lib/liveLogStreams`), because a logs route alone
 * may be a subject that never resolved. A shell is jumped to through
 * `/terminals`, which shows the newest session; the store keeps no way to ask
 * it for a particular one.
 */
export function LiveNow() {
  const forwards = useSyncExternalStore(subscribeForwards, getForwards, getForwards);
  const sessions = useSyncExternalStore(subscribeSessions, getSessions, getSessions);
  const { tabs } = useTabs();
  const streaming = useLiveLogStreams();
  const contexts = useContexts();
  const [failure, setFailure] = useState<{ title: string; error: unknown } | null>(null);
  const running = forwards.filter((f) => !isForwardEnded(f));
  const shells = sessions.filter((s) => s.state !== "closed");
  const logs = tabs.filter((t) => streaming.has(t.id));
  if (running.length === 0 && shells.length === 0 && logs.length === 0 && !failure) return null;

  // Forwards and shells name their cluster by context name.
  const goTo = (contextName: string, route: string) => {
    const context = contexts.find((c) => c.name === contextName);
    if (context) openOnCluster(context, route);
    else openTab(route);
  };
  const attempt = (title: string, act: () => Promise<void>) => {
    setFailure(null);
    act().catch((error: unknown) => setFailure({ title, error }));
  };

  return (
    <section className="home-side-section" aria-labelledby="home-live-title">
      <h2 id="home-live-title" className="home-section-heading">Live now</h2>
      {failure && <FailureAlert title={failure.title} error={failure.error} className="home-section-alert" />}
      {running.length > 0 && (
        <LiveGroup title="Port forwards">
          {running.map((f) => {
            const target = `${kindToForwardTarget(f.kind)}/${f.name}`;
            const address = forwardAddress({ id: f.id, localPort: f.localPort });
            return (
              <LiveRow
                key={f.id}
                label={target}
                detail={f.status === "reconnecting" ? `Reconnecting · ${f.context}` : `${address} · ${f.context}`}
                go={{ label: `Go to forward ${target} on ${f.context}`, run: () => goTo(f.context, "/forwards") }}
              >
                <Button variant="secondary" size="sm" aria-label={`Open ${target} in the browser`}
                  onClick={() => attempt(`Could not open ${target}`, () => openExternal(browsable(address)))}>Open</Button>
                <Button variant="secondary" size="sm" aria-label={`Stop forward ${target}`}
                  onClick={() => attempt(`Could not stop ${target}`, () => stopPortForward(f.id))}>Stop</Button>
              </LiveRow>
            );
          })}
        </LiveGroup>
      )}
      {shells.length > 0 && (
        <LiveGroup title="Shells">
          {shells.map((s) => (
            <LiveRow key={s.id} label={s.title} detail={s.context}
              go={{ label: `Go to shell ${s.title} on ${s.context}`, run: () => goTo(s.context, "/terminals") }}>
              <Button variant="secondary" size="sm" aria-label={`End shell ${s.title}`} onClick={() => endSession(s.id)}>End</Button>
            </LiveRow>
          ))}
        </LiveGroup>
      )}
      {logs.length > 0 && (
        <LiveGroup title="Log streams">
          {logs.map((t) => (
            <LiveRow key={t.id} label={t.title} detail={t.sub} go={{ label: `Go to ${t.title}`, run: () => activateTab(t.id) }}>
              <Button
                variant="secondary"
                size="sm"
                aria-label={`Stop ${t.title}`}
                title={t.pinned ? "Unpins its tab and closes it, which stops the stream" : "Stops the stream and closes its tab"}
                onClick={() => {
                  // `closeTab` refuses a pinned tab — silently — and Stop must stop.
                  if (t.pinned) togglePin(t.id);
                  closeTab(t.id);
                }}
              >
                Stop
              </Button>
            </LiveRow>
          ))}
        </LiveGroup>
      )}
    </section>
  );
}

function LiveGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="home-side-group">
      <Eyebrow>{title}</Eyebrow>
      <ul className="home-pick-list">{children}</ul>
    </div>
  );
}

function LiveRow({ label, detail, go, children }: {
  label: string;
  detail?: string;
  go: { label: string; run: () => void };
  children: ReactNode;
}) {
  return (
    <li className="home-live-row">
      <button type="button" className="home-live-go" aria-label={go.label} onClick={go.run}>
        <span className="block truncate font-medium">{label}</span>
        {detail && <span className="block truncate text-xs text-muted">{detail}</span>}
      </button>
      {children}
    </li>
  );
}
