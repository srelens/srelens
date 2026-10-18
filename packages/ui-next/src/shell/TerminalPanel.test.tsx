import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

/**
 * The panel around a session, not the emulator in it: `TerminalView` (which
 * has its own tests against a real xterm) is a marker here, and the session
 * store is doubled to the rows the panel reads.
 */
type Row = { id: number; title: string; context: string; state: "attached" | "idle" | "closed"; error?: string };
const { store, endSession, clear } = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const store = {
    rows: [] as Row[],
    set(rows: Row[]) {
      store.rows = rows;
      listeners.forEach((l) => l());
    },
    listeners,
  };
  return { store, endSession: vi.fn(), clear: vi.fn() };
});
vi.mock("../lib/sessions", () => ({
  getSessions: () => store.rows,
  subscribeSessions: (l: () => void) => {
    store.listeners.add(l);
    return () => store.listeners.delete(l);
  },
  endSession: (id: number) => endSession(id),
  terminalFor: () => ({ clear }),
}));
vi.mock("../screens/terminals/TerminalView", () => ({
  TerminalView: ({ sessionId }: { sessionId: number }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

const { PageWithTerminal } = await import("./TerminalPanel");
const panels = await import("../lib/terminalPanel");
const tabs = await import("../lib/tabsStore");
const { defaultState } = await import("../lib/tabs");

const dev = {
  name: "kind-dev", stableId: "dev", key: "dev", cluster: "c", server: "", isCurrent: false,
  sourceFile: "/home/user/.kube/config", authKind: "client certificate",
};

/** Not inside anything `hidden` — what jsdom can say about being on screen. */
const shown = (el: Element) => el.closest("[hidden]") === null;

function renderPage() {
  return render(
    <div style={{ height: 600 }}>
      <PageWithTerminal tabId="pods">
        <p>Pods list</p>
      </PageWithTerminal>
    </div>,
  );
}

beforeEach(() => {
  panels.__resetTerminalPanelsForTests();
  tabs.setState(defaultState([dev]));
  store.set([{ id: 7, title: "Local shell", context: "kind-dev", state: "attached" }]);
  endSession.mockReset();
  clear.mockReset();
});

afterEach(() => cleanup());

describe("PageWithTerminal", () => {
  it("is only the page while its tab has no terminal open", () => {
    renderPage();
    expect(shown(screen.getByText("Pods list"))).toBe(true);
    expect(screen.queryByRole("region", { name: /Terminal/ })).toBeNull();
  });

  it("shows the session under the page, named, with its verdict word and the shell's caveat", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    const panel = screen.getByRole("region", { name: "Terminal: Local shell" });
    expect(panel.textContent).toContain("Local shell · kind-dev");
    expect(panel.textContent).toContain("Attached");
    expect(panel.textContent).toMatch(/not gated/);
    expect(screen.getByTestId("terminal-view").textContent).toContain("7");
    // Half the page, and the page still there above it.
    expect(panel.style.height).toBe("50%");
    expect(shown(screen.getByText("Pods list"))).toBe(true);
  });

  it("is resized from its top edge, as a horizontal separator", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    // jsdom lays nothing out; give the surface a height to share.
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
    try {
      renderPage();
      const grip = screen.getByRole("separator", { name: "Resize the terminal" });
      expect(grip.getAttribute("aria-orientation")).toBe("horizontal");
      expect(grip.getAttribute("aria-valuenow")).toBe("300");
      fireEvent.keyDown(grip, { key: "ArrowUp" });
      expect(panels.terminalPanelShare()).toBeCloseTo(316 / 600);
    } finally {
      delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
    }
  });

  it("fills the page when maximized, keeping the page mounted behind it, and restores", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Maximize terminal" }));
    expect(shown(screen.getByText("Pods list"))).toBe(false);
    expect(screen.getByRole("region", { name: /Terminal/ }).getAttribute("data-maximized")).toBe("true");
    expect(screen.queryByRole("separator")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Restore terminal size" }));
    expect(shown(screen.getByText("Pods list"))).toBe(true);
  });

  it("closes without ending the shell", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Close terminal panel" }));
    expect(screen.queryByRole("region", { name: /Terminal/ })).toBeNull();
    expect(endSession).not.toHaveBeenCalled();
    expect(panels.terminalPanel("pods")).toEqual({ shellId: 7, open: false, maximized: false });
  });

  it("moves the session to the Terminals tab, asking that tab to show it", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Open terminal in a tab" }));
    expect(tabs.activeRoute()).toBe("/terminals");
    expect(panels.terminalPanel("pods")).toBeUndefined();
    expect(screen.queryByRole("region", { name: /Terminal/ })).toBeNull();
  });

  it("ends the shell on Detach, and clears only the transcript on Clear", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(clear).toHaveBeenCalled();
    expect(endSession).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Detach" }));
    expect(endSession).toHaveBeenCalledWith(7);
  });

  it("goes, and forgets its session, once the session has ended anywhere", () => {
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    act(() => store.set([]));
    expect(screen.queryByRole("region", { name: /Terminal/ })).toBeNull();
    expect(panels.terminalPanel("pods")).toBeUndefined();
  });

  it("says why a session ended, over its transcript", () => {
    store.set([{ id: 7, title: "Local shell", context: "kind-dev", state: "closed", error: "kubectl was not found" }]);
    act(() => panels.showTerminalPanel("pods", 7));
    renderPage();
    expect(screen.getByRole("alert").textContent).toContain("kubectl was not found");
    expect(screen.getByRole("region", { name: /Terminal/ }).textContent).toContain("Closed");
  });
});
