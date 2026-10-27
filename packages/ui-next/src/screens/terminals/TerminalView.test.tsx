import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";

/**
 * The emulator this pane attaches is `terminalFor(id)` off the real session
 * store — module-level, backed by core's `startPodExec`/`startLocalTerminal`.
 * Only the backend boundary is faked (the same shape `lib/sessions.test.ts`
 * uses); the store and the `Terminal` instance it hands out stay real, because
 * this file's whole job is proving what happens to THAT instance across
 * mount, resize and unmount.
 */
const startPodExec = vi.fn();
const startLocalTerminal = vi.fn();
vi.mock("@srelens/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core")>();
  return {
    ...actual,
    startPodExec: (...args: unknown[]) => startPodExec(...args),
    startLocalTerminal: (...args: unknown[]) => startLocalTerminal(...args),
  };
});

/**
 * `@xterm/addon-fit` computes cell dimensions off a real canvas measurement,
 * which jsdom cannot do — `proposeDimensions` reads `0` cell width there and
 * `fit()` quietly no-ops (that's the "benign" half of jsdom's xterm warning).
 * A real FitAddon would make "does the pane fit on mount / on resize" untestable
 * here, so the addon itself is doubled with a spy; everything else about the
 * emulator (attach, buffer, dispose) stays the real xterm instance.
 */
const { fitSpy } = vi.hoisted(() => ({ fitSpy: vi.fn() }));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    activate() {}
    dispose() {}
    fit() {
      fitSpy();
    }
  },
}));

const {
  __resetSessionsForTests,
  startLocalSession,
  terminalFor,
} = await import("../../lib/sessions");
const { TerminalView } = await import("./TerminalView");

// jsdom has no `matchMedia`; xterm's `CoreBrowserService` reads it on open()
// to watch the display's device pixel ratio, and throws without it.
function stubMatchMedia(target: typeof globalThis) {
  (target as unknown as { matchMedia: (query: string) => MediaQueryList }).matchMedia = (
    query: string,
  ) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList;
}
stubMatchMedia(globalThis);
if (typeof window !== "undefined") stubMatchMedia(window as unknown as typeof globalThis);

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

/**
 * Captures the callback every `ResizeObserver` created during a test was
 * given, so a test can fire one by hand — jsdom never resizes anything on its
 * own for a real observer to report.
 */
function fakeResizeObserver() {
  const callbacks: Array<() => void> = [];
  class FakeResizeObserver {
    #cb: () => void;
    constructor(cb: () => void) {
      this.#cb = cb;
      callbacks.push(cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  return { fire: () => callbacks.forEach((cb) => cb()) };
}

/** Give a pane the size jsdom never lays out, as a pane on screen has. */
function sized(el: HTMLElement, width = 640, height = 320) {
  Object.defineProperty(el, "clientWidth", { configurable: true, value: width });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: height });
}

/** A session with a backend double that never resolves anything further —
 *  enough for the store to hand out a live emulator to attach to. */
async function openSession(): Promise<number> {
  startLocalTerminal.mockImplementation(() => Promise.resolve({ send: vi.fn(), resize: vi.fn(), close: vi.fn() }));
  return startLocalSession({ context: "kind-srelens-demo" });
}

beforeEach(() => {
  startPodExec.mockReset();
  startLocalTerminal.mockReset();
  fitSpy.mockClear();
  __resetSessionsForTests();
});

afterEach(() => {
  cleanup();
  __resetSessionsForTests();
  vi.unstubAllGlobals();
});

describe("TerminalView", () => {
  it("attaches the session's emulator to its own container", async () => {
    const id = await openSession();
    const term = terminalFor(id);

    const { container } = render(<TerminalView sessionId={id} />);

    // xterm's own root node, planted inside OUR div — proof `open()` (or the
    // reattach path) actually ran against this pane's container, not just
    // that the component rendered something.
    expect(container.querySelector(".xterm")).not.toBeNull();
    expect(term?.element?.parentElement).toBe(container.firstElementChild);
  });

  it("fits the pane on mount", async () => {
    const id = await openSession();

    render(<TerminalView sessionId={id} />);

    expect(fitSpy).toHaveBeenCalled();
  });

  it("re-fits when the pane resizes", async () => {
    const observer = fakeResizeObserver();
    const id = await openSession();
    const { container } = render(<TerminalView sessionId={id} />);
    sized(container.firstElementChild as HTMLElement);
    fitSpy.mockClear();

    observer.fire();

    expect(fitSpy).toHaveBeenCalledTimes(1);
  });

  it("leaves a hidden pane alone: it measures nothing, so there is nothing to fit to", async () => {
    const observer = fakeResizeObserver();
    const id = await openSession();
    render(<TerminalView sessionId={id} />);
    fitSpy.mockClear();

    observer.fire();

    expect(fitSpy).not.toHaveBeenCalled();
  });

  it("takes the emulator back when it is showing again, from another pane that had it (#942)", async () => {
    const observer = fakeResizeObserver();
    const id = await openSession();
    const term = terminalFor(id);
    const panel = render(<TerminalView sessionId={id} />);
    const tab = render(<TerminalView sessionId={id} />);
    const panelPane = panel.container.firstElementChild as HTMLElement;
    // The last pane opened has it: the Terminals tab, here.
    expect(term?.element?.parentElement).toBe(tab.container.firstElementChild);

    // The panel's tab is selected again: only its pane has a size now.
    sized(panelPane);
    observer.fire();

    expect(term?.element?.parentElement).toBe(panelPane);
  });

  it("leaves the emulator in a pane that is showing it when a hidden pane mounts the same session", async () => {
    const observer = fakeResizeObserver();
    const id = await openSession();
    const term = terminalFor(id);
    const panel = render(<TerminalView sessionId={id} />);
    const panelPane = panel.container.firstElementChild as HTMLElement;
    sized(panelPane);
    // A hidden Terminals tab falls back to the same session and mounts it.
    const hidden = render(<TerminalView sessionId={id} />);
    expect(term?.element?.parentElement).toBe(panelPane);

    // Selected, the hidden pane has a size, and takes it then.
    const hiddenPane = hidden.container.firstElementChild as HTMLElement;
    sized(hiddenPane);
    sized(panelPane, 0, 0);
    observer.fire();
    expect(term?.element?.parentElement).toBe(hiddenPane);
  });

  it("dresses the emulator in the app's monospace size, not xterm's own 15px (#942)", async () => {
    const id = await openSession();
    const root = document.documentElement;
    root.style.setProperty("--text-label", "0.8125rem");
    try {
      render(<TerminalView sessionId={id} />);
      expect(terminalFor(id)?.options.fontSize).toBe(13);
    } finally {
      root.style.removeProperty("--text-label");
    }
  });

  it("does not dispose the emulator on unmount, and a remount shows what was already written", async () => {
    const id = await openSession();
    const term = terminalFor(id);
    if (!term) throw new Error("expected a live emulator");
    const disposeSpy = vi.spyOn(term, "dispose");

    const first = render(<TerminalView sessionId={id} />);
    await new Promise<void>((resolve) => term.write("already here\r\n", resolve));
    first.unmount();

    // The whole property this task exists to protect: closing the pane must
    // not touch the emulator the store owns.
    expect(disposeSpy).not.toHaveBeenCalled();
    // And the instance handed out is still the very same one — a store that
    // rebuilt it would satisfy the dispose assertion by accident.
    expect(terminalFor(id)).toBe(term);

    // xterm's own render is debounced onto a `requestAnimationFrame`, so the
    // buffer holding the line is not the same instant as the DOM showing it.
    const second = render(<TerminalView sessionId={id} />);
    await vi.waitFor(() => {
      const line = second.container.querySelector(".xterm-rows")?.children[0]?.textContent;
      expect(line).toContain("already here");
    });
  });
});
