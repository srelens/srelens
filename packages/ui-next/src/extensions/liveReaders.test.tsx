import { act, cleanup, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ExtensionStreamHandlers, ExtensionStreamRequest } from "@srelens/core";

const core = vi.hoisted(() => ({
  tauri: true,
  openExtensionView: vi.fn(),
}));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => core.tauri,
  openExtensionView: core.openExtensionView,
}));

import { LiveReaders, LiveStatus, useLiveApps, useLiveReaders, type LiveState } from "./liveReaders";

type Opened = { request: ExtensionStreamRequest; handlers: ExtensionStreamHandlers; cancel: ReturnType<typeof vi.fn> };
let opened: Opened[] = [];
let views: Array<{ view: string; close: ReturnType<typeof vi.fn> }> = [];
let refuse: string | null = null;

beforeEach(() => {
  core.tauri = true;
  opened = [];
  views = [];
  refuse = null;
  core.openExtensionView.mockReset();
  core.openExtensionView.mockImplementation((app: string, label: string) => {
    const view = { view: `${app}/${label}#${views.length}`, close: vi.fn(async () => {}) };
    views.push(view);
    return {
      ...view,
      open: async (request: ExtensionStreamRequest, handlers: ExtensionStreamHandlers) => {
        if (refuse) throw new Error(refuse);
        const cancel = vi.fn(async () => {});
        opened.push({ request, handlers, cancel });
        return { stream: `s-${opened.length}`, cancel };
      },
    };
  });
});
afterEach(() => cleanup());

const plugin = { manifest: { id: "org.example.flux", name: "Flux" }, revision: 3 } as never;

function hook(capabilities: string[], onChange = vi.fn(), namespace = "team") {
  const rendered = renderHook(() => useLiveReaders({ plugin, capabilities, context: "kind-demo", namespace, label: "page:test", onChange }));
  return { ...rendered, onChange };
}

async function settle() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe("live readers (#566)", () => {
  it("opens one watch per reader, in the view's cluster and namespace", async () => {
    const { result } = hook(["kustomizations", "helmreleases"]);
    await settle();
    expect(opened.map((o) => o.request)).toEqual([
      { id: "org.example.flux", revision: 3, context: "kind-demo", namespace: "team", source: { kind: "watch", capability: "helmreleases" } },
      { id: "org.example.flux", revision: 3, context: "kind-demo", namespace: "team", source: { kind: "watch", capability: "kustomizations" } },
    ]);
    expect(result.current).toEqual({ state: "connecting" });
  });

  it("is live after the first list and asks to read again on every change", async () => {
    const { result, onChange } = hook(["kustomizations"]);
    await settle();
    act(() => opened[0].handlers.onData({ event: "synced" }, 1));
    expect(result.current).toEqual({ state: "live" });
    act(() => opened[0].handlers.onData({ event: "changed" }, 2));
    expect(onChange).toHaveBeenCalledTimes(2);
    act(() => opened[0].handlers.onData({ nonsense: true }, 3));
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("says reconnecting when the watch is lost, and live again only after a fresh list", async () => {
    const { result, onChange } = hook(["kustomizations", "helmreleases"]);
    await settle();
    act(() => { opened[0].handlers.onData({ event: "synced" }, 1); opened[1].handlers.onData({ event: "synced" }, 1); });
    act(() => opened[1].handlers.onData({ event: "reconnecting", message: "connection reset" }, 2));
    expect(result.current).toEqual({ state: "reconnecting", message: "connection reset" });
    act(() => opened[0].handlers.onData({ event: "changed" }, 2));
    expect(result.current.state).toBe("reconnecting");
    act(() => opened[1].handlers.onData({ event: "synced" }, 3));
    expect(result.current).toEqual({ state: "live" });
    expect(onChange).toHaveBeenCalledTimes(4);
  });

  it("stops with why when the host refuses the open or ends the stream", async () => {
    refuse = "App org.example.flux already has 8 open streams, the most one app may have; close a view or cancel a stream first";
    const refused = hook(["kustomizations"]);
    await settle();
    expect(refused.result.current).toEqual({ state: "stopped", message: refuse });
    refused.unmount();
    refuse = null;
    const ended = hook(["kustomizations"]);
    await settle();
    act(() => opened[0].handlers.onEnd?.({ type: "close", reason: "appUpdated" }));
    expect(ended.result.current).toEqual({ state: "stopped", message: "The app was updated; reopen the view to follow it again." });
  });

  it("is off on the web, which has no app streams, and opens nothing", async () => {
    core.tauri = false;
    const { result } = hook(["kustomizations"]);
    await settle();
    expect(result.current.state).toBe("off");
    expect(core.openExtensionView).not.toHaveBeenCalled();
  });

  it("shares one watch per reader within a view and closes the view with it", async () => {
    const onChange = vi.fn();
    function Consumer({ capability }: { capability: string }) {
      useLiveReaders({ plugin, capabilities: [capability], context: "kind-demo", namespace: "", label: "x", onChange });
      return null;
    }
    const { unmount } = render(
      <LiveReaders plugin={plugin} label="page:overview">
        <Consumer capability="kustomizations" />
        <Consumer capability="kustomizations" />
        <Consumer capability="helmreleases" />
      </LiveReaders>,
    );
    await settle();
    expect(opened.map((o) => o.request.source.capability)).toEqual(["kustomizations", "helmreleases"]);
    expect(views.length).toBe(1);
    act(() => opened[0].handlers.onData({ event: "changed" }, 1));
    expect(onChange).toHaveBeenCalledTimes(2);
    unmount();
    await settle();
    expect(views[0].close).toHaveBeenCalled();
  });

  it("gives the web reason only on the web, and is connecting from the first render on the desktop", () => {
    const first: LiveState[] = [];
    renderHook(() => {
      const live = useLiveReaders({ plugin, capabilities: ["kustomizations"], context: "kind-demo", namespace: "", label: "x", onChange: () => {} });
      first.push(live);
      return live;
    });
    expect(first[0]).toEqual({ state: "connecting" });
    const nothing = hook([]);
    expect(nothing.result.current.state).toBe("off");
    expect((nothing.result.current as { reason: string }).reason).not.toMatch(/web app/);
    const apps = renderHook(() => useLiveApps({ apps: [], context: "kind-demo", namespace: "", label: "x", onChange: () => {}, off: "Held back for a reason." }));
    expect(apps.result.current).toEqual({ state: "off", reason: "Held back for a reason." });
    const firstApps: LiveState[] = [];
    renderHook(() => {
      const live = useLiveApps({ apps: [{ plugin, capabilities: ["kustomizations"] }], context: "kind-demo", namespace: "", label: "x", onChange: () => {} });
      firstApps.push(live);
      return live;
    });
    expect(firstApps[0]).toEqual({ state: "connecting" });
    core.tauri = false;
    const web = renderHook(() => useLiveApps({ apps: [{ plugin, capabilities: ["k"] }], context: "kind-demo", namespace: "", label: "x", onChange: () => {} }));
    expect((web.result.current as { reason: string }).reason).toMatch(/web app/);
  });

  it("follows the new app when a mounted view is handed another one, and ends the old app's view", async () => {
    const other = { manifest: { id: "org.example.argocd", name: "Argo CD" }, revision: 3 } as never;
    const own = renderHook(({ app }) => useLiveReaders({ plugin: app, capabilities: ["apps"], context: "kind-demo", namespace: "", label: "x", onChange: () => {} }), { initialProps: { app: plugin } });
    await settle();
    own.rerender({ app: other });
    await settle();
    expect(opened.map((o) => o.request.id)).toEqual(["org.example.flux", "org.example.argocd"]);
    expect(views[0].close).toHaveBeenCalled();
    function Consumer() {
      useLiveReaders({ plugin: other, capabilities: ["apps"], context: "kind-demo", namespace: "", label: "x", onChange: () => {} });
      return null;
    }
    const provided = render(<LiveReaders plugin={plugin} label="page"><Consumer /></LiveReaders>);
    await settle();
    provided.rerender(<LiveReaders plugin={other} label="page"><Consumer /></LiveReaders>);
    await settle();
    expect(opened.at(-1)?.request.id).toBe("org.example.argocd");
    // Inside a view of another app, a reader is still watched for its own app.
    opened = [];
    render(<LiveReaders plugin={plugin} label="page"><Consumer /></LiveReaders>);
    await settle();
    expect(opened.map((o) => o.request.id)).toEqual(["org.example.argocd"]);
  });

  it("names each state in words", () => {
    const states: LiveState[] = [
      { state: "live" }, { state: "connecting" },
      { state: "reconnecting", message: "reset" },
      { state: "stopped", message: "refused" }, { state: "off", reason: "web" },
    ];
    for (const state of states) render(<LiveStatus live={state} />);
    expect(screen.getByText("Live")).toBeTruthy();
    expect(screen.getByText("Connecting…")).toBeTruthy();
    expect(screen.getByText("Reconnecting…")).toBeTruthy();
    expect(screen.getAllByText("Not live")).toHaveLength(2);
  });
});
