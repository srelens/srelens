import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { LogStreamOptions, LogTarget, SpoolPage } from "@srelens/core";

/**
 * The seam `Logs.test.tsx` cannot see (#914). That suite mocks the stream HOOK, so what the screen tells the hook
 * (`hold`) and what the hook then does with it (freezes the view, skips the ring's pre-raise) are never in one run.
 * This file keeps the real `useLogStream` and fakes only the transport under it, `startLogStream`, the way
 * `lib/logStream.test.ts` does, then drives the screen with a mouse.
 */
const h = vi.hoisted(() => ({
  startLogStream: vi.fn(),
  resolve: vi.fn(),
}));

vi.mock("../lib/logSubject", async (orig) => ({
  ...(await orig<typeof import("../lib/logSubject")>()),
  resolveLogSubject: (...a: unknown[]) => h.resolve(...a),
}));

vi.mock("@srelens/core", async (orig) => {
  const real = await orig<typeof import("@srelens/core")>();
  return {
    ...real,
    isTauri: () => true,
    startLogStream: (...a: unknown[]) => h.startLogStream(...a),
    listContexts: async () => ({ contexts: [{ name: "prod-eu", key: "prod" }] }),
    listResource: async () => ({ items: [{ name: "checkout-api", namespace: "checkout", age: "1d" }] }),
  };
});

// No app installed. One value for every render: the screen derives its sources from it, and a fresh one each time
// would never settle.
vi.mock("../extensions/inventoryStore", async (orig) => {
  const inventory = { status: "ready", data: { schemaVersion: 1, nextRevision: 9, plugins: [] }, reload: () => {} };
  return { ...(await orig<typeof import("../extensions/inventoryStore")>()), useExtensions: () => inventory };
});

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
HTMLElement.prototype.scrollIntoView ??= () => {};

import type { ClusterContext } from "@srelens/core";
import { Logs, logsRoute } from "./Logs";
import { ConsoleProvider } from "../console";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { loadRecentLogSubjects } from "../lib/logRecents";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  key: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};
const TARGETS: LogTarget[] = [{ pod: "api-7", container: "api", label: "api-7/api" }];

beforeEach(() => {
  vi.clearAllMocks();
  h.resolve.mockResolvedValue({
    status: "resolved",
    targets: TARGETS,
    pods: [{ name: "api-7", health: "success" }],
    previous: [],
  });
  resetContexts();
  setContexts([CTX]);
  setKubeconfigFiles(["/home/u/.kube/config"]);
  store.setState(defaultState([CTX]));
  const m = new Map<string, string>();
  loadRecentLogSubjects({
    getItem: (k) => m.get(k) ?? null,
    setItem: (k, v) => void m.set(k, v),
    removeItem: (k) => void m.delete(k),
  });
});

/** Give `.logline` rows a height, so the screen windows its rows instead of drawing every one of 10,000. */
function measurableRows() {
  const proto = window.HTMLElement.prototype;
  const original = proto.getBoundingClientRect;
  proto.getBoundingClientRect = function (this: HTMLElement) {
    if (this.classList.contains("logline")) return { height: 20 } as DOMRect;
    return original.call(this);
  };
  return () => {
    proto.getBoundingClientRect = original;
  };
}

/** The `n` lines of a stream numbered from `from`, as the backend frames them: `l<seq>` is line `seq`. */
const many = (from: number, n: number) =>
  Array.from({ length: n }, (_, i) => ({ source: "api-7/api", text: `l${from + i}` }));

/** A stream that keeps history on disk: the test fires frames and decides when a read returns. */
function spooledStream() {
  let options!: LogStreamOptions;
  const readEarlier = vi.fn<(before: number, limit: number) => Promise<SpoolPage>>();
  h.startLogStream.mockImplementation(
    (_c: string, _n: string, _t: LogTarget[], _line: unknown, _status: unknown, opts: LogStreamOptions) => {
      options = opts;
      return Promise.resolve({ stop: vi.fn(), readEarlier, exportSpool: vi.fn(async () => null) });
    },
  );
  return {
    readEarlier,
    frame: (seq: number, count: number, first = 1) =>
      act(() => options.onLines!(many(seq, count), { seq, first })),
    /** The next read waits until the returned function gives it a page. */
    holdRead: () => {
      let arrive!: (page: SpoolPage) => void;
      readEarlier.mockReturnValue(new Promise<SpoolPage>((resolve) => (arrive = resolve)));
      return arrive;
    },
  };
}

/** The task after a release, which is when the screen lets go of a press. */
const released = () => act(() => new Promise((r) => setTimeout(r, 0)));
/** Long enough for the next animation frame to commit what has arrived. */
const frame = () => act(() => new Promise((r) => setTimeout(r, 40)));

describe("loading earlier lines by mouse on a busy, followed stream (#914)", () => {
  // A followed view moves every frame. Loading into one raced the stream: at tens of thousands of lines a second the
  // page was pushed back out of the ring within half a second, and near the limit on loaded lines, where the ring has
  // no room to widen for it, it was refused about half the time. So the click pauses the view, and the page joins that.
  it("pauses the view and joins the page to it, though the stream ran on while it was read", async () => {
    const restore = measurableRows();
    try {
      const s = spooledStream();
      render(
        <ConsoleProvider>
          <Logs route={logsRoute("Deployment", "checkout", "checkout-api")} />
        </ConsoleProvider>,
      );
      await waitFor(() => expect(h.startLogStream).toHaveBeenCalled());
      // A full ring, 5,000 lines of a stream that has written 10,000: the other 5,000 are on disk.
      s.frame(5001, 5000);
      const load = await screen.findByRole("button", { name: /Load 5\s?000 earlier/ });
      const arrive = s.holdRead();

      // A mouse click on the button, as a browser sends it: the press, the release, and the click after both.
      fireEvent.pointerDown(load);
      fireEvent.pointerUp(load);
      fireEvent.click(load);
      expect(s.readEarlier).toHaveBeenCalledWith(5001, 5000);
      // The click paused the view: the control now offers to follow again.
      expect(screen.getByRole("button", { name: "Follow" })).toBeTruthy();
      // The release lets go of the view a task later; the stream has not stopped, and writes on while the page is read.
      await released();
      s.frame(10001, 300);
      await frame();

      await act(async () => {
        arrive({ first: 1, seq: 1, lines: many(1, 5000) });
        await Promise.resolve();
      });
      await frame();

      // Never refused: "The stream moved past those lines while they were read".
      expect(screen.queryByText(/moved past those lines/)).toBeNull();
      expect(screen.queryByText("Could not load earlier lines")).toBeNull();
      // All 5,000 lines before the ring are in the view now, so nothing is left on disk to offer.
      expect(screen.queryByText(/earlier lines on disk/)).toBeNull();
      // And the lines the stream wrote meanwhile wait, as they do in any paused view, for the reader to follow again.
      expect(screen.getByText("300 new lines")).toBeTruthy();
    } finally {
      restore();
    }
  });

  it("gives the ring back on Follow, and offers the lines it let go again", async () => {
    const restore = measurableRows();
    try {
      const s = spooledStream();
      render(
        <ConsoleProvider>
          <Logs route={logsRoute("Deployment", "checkout", "checkout-api")} />
        </ConsoleProvider>,
      );
      await waitFor(() => expect(h.startLogStream).toHaveBeenCalled());
      s.frame(5001, 5000);
      const load = await screen.findByRole("button", { name: /Load 5\s?000 earlier/ });
      s.readEarlier.mockResolvedValue({ first: 1, seq: 1, lines: many(1, 5000) });
      await act(async () => {
        fireEvent.click(load);
        await Promise.resolve();
      });
      await frame();
      // Every line before the ring is in the view, so there is nothing on disk to offer.
      expect(screen.queryByText(/earlier lines on disk/)).toBeNull();

      await act(async () => void fireEvent.click(screen.getByRole("button", { name: "Follow" })));
      await frame();
      // Following again is the live tail in the ring's own size: the 5,000 lines that no longer fit are on disk again.
      expect(screen.getByText(/5\s?000 earlier lines on disk/)).toBeTruthy();
      expect(screen.getByRole("button", { name: /Load 5\s?000 earlier/ })).toBeTruthy();
    } finally {
      restore();
    }
  });
});
