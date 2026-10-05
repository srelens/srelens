import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: vi.fn(() => true),
  listContexts: vi.fn(),
  openExtensionView: vi.fn(),
}));
vi.mock("./inventoryStore", async (original) => ({
  ...(await original<typeof import("./inventoryStore")>()),
  useExtensions: vi.fn(),
}));

import { isTauri, listContexts, openExtensionView, type ExtensionView, type InstalledExtension } from "@srelens/core";
import { useExtensions } from "./inventoryStore";
import { refreshContextIds } from "./contextIds";
import { useLogProviders, useLogProviderSource, type LogProviderChoice } from "./logProviders";

function app(id: string, providers: unknown[], extra: Partial<InstalledExtension> = {}): InstalledExtension {
  return {
    manifest: {
      id, name: id === "org.example.loki" ? "Loki logs" : "Other", version: "1.0.0", srelensApiVersion: "^0.7",
      kind: "declarative", permissions: [], capabilities: [],
      contributions: { pages: [], detailTabs: [], detailLinks: [], logProviders: providers },
    },
    enabled: true, revision: 4, grants: [], settings: {}, source: "local", installedAt: 0, history: [],
    ...extra,
  } as unknown as InstalledExtension;
}
const loki = (id: string, forKinds = ["/Pod"]) =>
  ({ id, title: id === "loki" ? "Loki" : id, capability: "q", language: "logql", forKinds, query: "{}" });

function inventory(plugins: InstalledExtension[]) {
  vi.mocked(useExtensions).mockReturnValue({ status: "ready", data: { schemaVersion: 1, nextRevision: 9, plugins }, reload: vi.fn() } as never);
}

/** A fake view whose opens are recorded, and whose streams the test drives. */
function fakeView() {
  const opened: Array<{ request: Parameters<ExtensionView["open"]>[0]; handlers: Parameters<ExtensionView["open"]>[1] }> = [];
  const close = vi.fn(async () => {});
  const view: ExtensionView = {
    view: "v-1",
    close,
    open: (async (request, handlers) => {
      opened.push({ request, handlers: handlers as never });
      return { stream: `s-${opened.length}`, cancel: vi.fn(async () => {}) };
    }) as ExtensionView["open"],
  };
  return { view, opened, close };
}

beforeEach(async () => {
  vi.mocked(listContexts).mockResolvedValue({ contexts: [{ name: "prod-eu", key: "prod" }] } as never);
  vi.mocked(openExtensionView).mockReset();
});

/** The hook rendered once the cluster listing it subscribed to has answered. */
async function listed<T, P>(hook: (props: P) => T) {
  const rendered = renderHook(hook);
  await act(async () => {
    await refreshContextIds();
  });
  return rendered;
}

describe("the log providers the log view offers (#569)", () => {
  it("lists each enabled app's log providers for the view's kind, by title and app", async () => {
    inventory([
      app("org.example.loki", [loki("loki"), loki("audit"), loki("deployments", ["apps/Deployment"])]),
      app("org.example.off", [loki("off")], { enabled: false }),
      app("org.example.quarantined", [loki("q")], { quarantined: "bad" }),
      app("org.example.blocked", [loki("b")], { policyBlocked: "no" }),
      app("org.example.elsewhere", [loki("e")], { contexts: ["staging"] }),
    ]);
    const { result } = await listed(() => useLogProviders("prod-eu", "/Pod"));
    expect(result.current.status).toBe("ready");
    expect(result.current.choices.map((choice) => [choice.key, choice.label])).toEqual([
      ["org.example.loki/loki", "Loki · Loki logs"],
      ["org.example.loki/audit", "audit · Loki logs"],
    ]);
    expect(result.current.choices[0]).toMatchObject({ appId: "org.example.loki", revision: 4, provider: "loki" });
  });

  it("offers none on the web, which runs no app streams yet", async () => {
    inventory([app("org.example.loki", [loki("loki")])]);
    vi.mocked(isTauri).mockReturnValue(false);
    expect(renderHook(() => useLogProviders("prod-eu", "/Pod")).result.current).toEqual({ status: "ready", choices: [] });
    vi.mocked(isTauri).mockReturnValue(true);
    expect((await listed(() => useLogProviders("prod-eu", "/Pod"))).result.current.choices).toHaveLength(1);
  });

  it("offers none for a kind no provider is for, or before the inventory has loaded", async () => {
    inventory([app("org.example.loki", [loki("loki")])]);
    expect((await listed(() => useLogProviders("prod-eu", "batch/Job"))).result.current).toEqual({ status: "ready", choices: [] });
    expect(renderHook(() => useLogProviders("prod-eu", undefined)).result.current).toEqual({ status: "ready", choices: [] });
    vi.mocked(useExtensions).mockReturnValue({ status: "loading", reload: vi.fn() } as never);
    expect(renderHook(() => useLogProviders("prod-eu", "/Pod")).result.current).toEqual({ status: "loading", choices: [] });
  });

  it("keeps the last list through a refresh that fails or is still loading, and says which", async () => {
    inventory([app("org.example.loki", [loki("loki")])]);
    const { result, rerender } = await listed(() => useLogProviders("prod-eu", "/Pod"));
    expect(result.current.choices.map((choice) => choice.key)).toEqual(["org.example.loki/loki"]);
    // A failed refresh is not every app removed: the list stays, marked as not current.
    vi.mocked(useExtensions).mockReturnValue({ status: "error", error: "The inventory could not be read", reload: vi.fn() } as never);
    rerender();
    expect(result.current).toMatchObject({ status: "error", error: "The inventory could not be read" });
    expect(result.current.choices.map((choice) => choice.key)).toEqual(["org.example.loki/loki"]);
    vi.mocked(useExtensions).mockReturnValue({ status: "loading", reload: vi.fn() } as never);
    rerender();
    expect(result.current.status).toBe("loading");
    expect(result.current.choices).toHaveLength(1);
    // An answer is current again, even an empty one.
    inventory([]);
    rerender();
    expect(result.current).toEqual({ status: "ready", choices: [] });
  });
});

describe("following a log provider (#569)", () => {
  const choice: LogProviderChoice = { key: "org.example.loki/loki", appId: "org.example.loki", revision: 4, provider: "loki", label: "Loki · Loki logs" };
  const subject = { context: "prod-eu", namespace: "team", resourceKind: "/Pod", name: "web-1" };

  it("opens a logProvider stream on its own view and speaks the log view's callbacks", async () => {
    const fake = fakeView();
    vi.mocked(openExtensionView).mockReturnValue(fake.view);
    const { result } = renderHook(() => useLogProviderSource(choice, subject));
    expect(openExtensionView).toHaveBeenCalledWith("org.example.loki", "logs:loki");
    // One target, tagged as the host tags the provider's status, so the readout counts one source.
    expect(result.current.targets).toEqual([{ pod: "web-1", label: "loki" }]);
    const onLine = vi.fn();
    const onStatus = vi.fn();
    await result.current.source!.open([], onLine, onStatus, { tailLines: 1000, sinceSeconds: 600, timestamps: true });
    expect(fake.opened[0].request).toEqual({
      id: "org.example.loki", revision: 4, context: "prod-eu", namespace: "team",
      source: { kind: "logProvider", provider: "loki", resourceKind: "/Pod", name: "web-1", tailLines: 1000, sinceSeconds: 600, timestamps: true },
    });
    act(() => fake.opened[0].handlers.onData({ event: "lines", lines: [{ source: "web-1/app", line: "hello" }] }, 1));
    act(() => fake.opened[0].handlers.onData({ event: "status", source: "loki", status: "live" }, 2));
    expect(onLine).toHaveBeenCalledWith("web-1/app", "hello", false);
    expect(onStatus).toHaveBeenCalledWith("live", "loki");
  });

  it("says how the stream ended, and a retry opens it again under a new key", async () => {
    const fake = fakeView();
    vi.mocked(openExtensionView).mockReturnValue(fake.view);
    const { result } = renderHook(() => useLogProviderSource(choice, subject));
    const key = result.current.source!.key;
    await result.current.source!.open([], vi.fn(), vi.fn(), {});
    act(() => fake.opened[0].handlers.onEnd?.({ type: "error", code: "source", message: "The server answered HTTP 403 Forbidden" }));
    expect(result.current.end).toEqual({ type: "error", code: "source", message: "The server answered HTTP 403 Forbidden" });
    act(() => result.current.retry());
    expect(result.current.end).toBeNull();
    expect(result.current.source!.key).not.toBe(key);
  });

  it("does not take an earlier open's ending for the current stream's", async () => {
    const fake = fakeView();
    vi.mocked(openExtensionView).mockReturnValue(fake.view);
    const { result } = renderHook(() => useLogProviderSource(choice, subject));
    // A change of window: the log view stops the first stream and opens a second.
    await result.current.source!.open([], vi.fn(), vi.fn(), { sinceSeconds: 600 });
    await result.current.source!.open([], vi.fn(), vi.fn(), { sinceSeconds: 300 });
    // The host ends the first only after the second is open.
    act(() => fake.opened[0].handlers.onEnd?.({ type: "close", reason: "cancelled" }));
    expect(result.current.end).toBeNull();
    act(() => fake.opened[1].handlers.onEnd?.({ type: "close", reason: "completed" }));
    expect(result.current.end).toEqual({ type: "close", reason: "completed" });
  });

  it("does not carry a closed view's ending to the provider chosen again", async () => {
    const first = fakeView();
    const second = fakeView();
    vi.mocked(openExtensionView).mockReturnValueOnce(first.view).mockReturnValueOnce(second.view);
    const { result, rerender } = renderHook(({ picked }) => useLogProviderSource(picked, subject), {
      initialProps: { picked: choice as LogProviderChoice | undefined },
    });
    await result.current.source!.open([], vi.fn(), vi.fn(), {});
    // Kubernetes, then Loki again: the first view closes, and the host says so late.
    rerender({ picked: undefined });
    act(() => first.opened[0].handlers.onEnd?.({ type: "close", reason: "viewClosed" }));
    rerender({ picked: choice });
    expect(result.current.end).toBeNull();
    expect(result.current.source).toBeDefined();
  });

  it("closes its view when the source changes or the screen goes", () => {
    const first = fakeView();
    const second = fakeView();
    vi.mocked(openExtensionView).mockReturnValueOnce(first.view).mockReturnValueOnce(second.view);
    const { result, rerender, unmount } = renderHook(({ picked }) => useLogProviderSource(picked, subject), {
      initialProps: { picked: choice as LogProviderChoice | undefined },
    });
    rerender({ picked: { ...choice, key: "org.example.loki/audit", provider: "audit" } });
    expect(first.close).toHaveBeenCalledTimes(1);
    unmount();
    expect(second.close).toHaveBeenCalledTimes(1);
    // Kubernetes again: no view, no source, and the log view's own targets.
    const { result: none } = renderHook(() => useLogProviderSource(undefined, subject));
    expect(none.current.source).toBeUndefined();
    expect(none.current.targets).toBeUndefined();
    void result;
  });
});
