import { beforeEach, describe, expect, it } from "vitest";
import {
  __resetTerminalPanelsForTests,
  closeTerminalPanel,
  DEFAULT_TERMINAL_PANEL_SHARE,
  forgetTerminalPanel,
  MAX_TERMINAL_PANEL_SHARE,
  MIN_TERMINAL_PANEL_SHARE,
  parseStoredTerminalPanelShare,
  saveTerminalPanelShare,
  setTerminalPanelMaximized,
  setTerminalPanelMinimized,
  showTerminalPanel,
  TERMINAL_PANEL_SHARE_KEY,
  terminalPanel,
  terminalPanelShare,
} from "./terminalPanel";

function memory(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
    map,
  };
}

beforeEach(() => __resetTerminalPanelsForTests());

describe("a tab's terminal panel", () => {
  it("is each tab's own", () => {
    showTerminalPanel("pods", 1);
    showTerminalPanel("services", 2);
    expect(terminalPanel("pods")).toEqual({ shellId: 1, open: true, maximized: false, minimized: false });
    expect(terminalPanel("services")?.shellId).toBe(2);
    expect(terminalPanel("events")).toBeUndefined();
  });

  it("remembers its session when closed, and forgets it when told to", () => {
    showTerminalPanel("pods", 1);
    setTerminalPanelMaximized("pods", true);
    closeTerminalPanel("pods");
    // Closed restores the split: reopened, it is under the page again.
    expect(terminalPanel("pods")).toEqual({ shellId: 1, open: false, maximized: false, minimized: false });
    forgetTerminalPanel("pods");
    expect(terminalPanel("pods")).toBeUndefined();
  });

  it("minimizes to its head and back, never maximized at the same time", () => {
    showTerminalPanel("pods", 1);
    setTerminalPanelMaximized("pods", true);
    setTerminalPanelMinimized("pods", true);
    expect(terminalPanel("pods")).toEqual({ shellId: 1, open: true, maximized: false, minimized: true });
    setTerminalPanelMaximized("pods", true);
    expect(terminalPanel("pods")?.minimized).toBe(false);
    setTerminalPanelMinimized("pods", true);
    // Shown again, as Terminal does, it is up again.
    showTerminalPanel("pods", 1);
    expect(terminalPanel("pods")?.minimized).toBe(false);
  });

  it("keeps a maximized panel maximized when the same session is shown again, not a new one", () => {
    showTerminalPanel("pods", 1);
    setTerminalPanelMaximized("pods", true);
    showTerminalPanel("pods", 1);
    expect(terminalPanel("pods")?.maximized).toBe(true);
    showTerminalPanel("pods", 2);
    expect(terminalPanel("pods")?.maximized).toBe(false);
  });

});

describe("the panel's size", () => {
  it("is half the page until the reader drags it", () => {
    expect(terminalPanelShare(memory())).toBe(DEFAULT_TERMINAL_PANEL_SHARE);
  });

  it("is kept between launches, within its bounds", () => {
    const storage = memory();
    saveTerminalPanelShare(0.3, storage);
    expect(storage.map.get(TERMINAL_PANEL_SHARE_KEY)).toBe("0.3");
    __resetTerminalPanelsForTests();
    expect(terminalPanelShare(storage)).toBe(0.3);

    saveTerminalPanelShare(0.99, storage);
    expect(terminalPanelShare(storage)).toBe(MAX_TERMINAL_PANEL_SHARE);
    saveTerminalPanelShare(0.01, storage);
    expect(terminalPanelShare(storage)).toBe(MIN_TERMINAL_PANEL_SHARE);
  });

  it("reads anything but a share of the page as nothing saved", () => {
    expect(parseStoredTerminalPanelShare(null)).toBeNull();
    expect(parseStoredTerminalPanelShare("not json")).toBeNull();
    expect(parseStoredTerminalPanelShare('"0.4"')).toBeNull();
    expect(parseStoredTerminalPanelShare("1.5")).toBeNull();
    expect(parseStoredTerminalPanelShare("0")).toBeNull();
    expect(parseStoredTerminalPanelShare("0.4")).toBe(0.4);
  });

  it("survives a storage that refuses to be read or written", () => {
    const refusing = {
      getItem: () => {
        throw new Error("storage disabled");
      },
      setItem: () => {
        throw new Error("storage disabled");
      },
      removeItem: () => {},
    };
    const quiet = console.error;
    console.error = () => {};
    try {
      expect(terminalPanelShare(refusing)).toBe(DEFAULT_TERMINAL_PANEL_SHARE);
      saveTerminalPanelShare(0.4, refusing);
      expect(terminalPanelShare(refusing)).toBe(0.4);
    } finally {
      console.error = quiet;
    }
  });
});
