import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Console } from "./Console";
import { ConsoleProvider, useConsole } from "../console";
import { defaultState } from "../lib/tabs";
import * as tabsStore from "../lib/tabsStore";
import { lockWorkspace, resetLock } from "./LockGate";

const { useAgentRun, askAgent, clearAgentRun } = vi.hoisted(() => ({
  useAgentRun: vi.fn(),
  askAgent: vi.fn(),
  clearAgentRun: vi.fn(),
}));
vi.mock("../lib/agentRun", () => ({ useAgentRun, askAgent, clearAgentRun }));

/** The store's shape, defaulted to idle-and-empty — every test overrides only
 *  the fields it cares about, the same convention `Composer.test.tsx` uses
 *  for the same store. */
function runState(overrides: { turns?: unknown[]; gates?: unknown[]; busy?: boolean } = {}) {
  return { turns: [], gates: [], busy: false, generation: 0, agentKind: "claude", ...overrides };
}

/** Anything else on screen, putting a question to the console from outside it. */
function Elsewhere() {
  const { ask } = useConsole();
  return (
    <button type="button" onClick={() => ask("x")}>
      Ask from elsewhere
    </button>
  );
}

function setup({ apple = true, onToggleTheme = () => {} }: { apple?: boolean; onToggleTheme?: () => void } = {}) {
  return render(
    <ConsoleProvider>
      <Elsewhere />
      <Console apple={apple} onToggleTheme={onToggleTheme} />
    </ConsoleProvider>,
  );
}

beforeEach(() => {
  // A clean workspace for every test: the route-aware suggestions test opens
  // a `/logs` tab of its own, and nothing here may leak into a later test —
  // the store is module-level, not reset between tests on its own.
  tabsStore.setState(defaultState([]));
  resetLock();
  useAgentRun.mockReset().mockReturnValue(runState());
  askAgent.mockReset();
  clearAgentRun.mockReset();
});

describe("Console", () => {
  it("asks the agent with what was typed at the prompt, and clears it", async () => {
    const user = userEvent.setup();
    setup();
    const input = screen.getByRole("textbox", { name: "Console prompt" }) as HTMLInputElement;
    await user.type(input, "why{Enter}");
    expect(askAgent).toHaveBeenCalledWith("why");
    expect(input.value).toBe("");
  });

  it("opens and forwards a question asked from anywhere else", async () => {
    const user = userEvent.setup();
    setup();
    // Closed to begin with: nothing to read, so no output region.
    expect(screen.queryByRole("log")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.getByRole("log", { name: "Console output" })).toBeDefined();
    expect(askAgent).toHaveBeenCalledWith("x");
  });

  it("prints the console accelerator for the platform", () => {
    setup({ apple: true });
    expect(screen.getByText("⌘K")).toBeDefined();
  });

  it("shows route-aware suggestions when there is nothing to show yet", async () => {
    const user = userEvent.setup();
    // Route-aware: `/logs` is what makes "Summarise the last 500 lines" the
    // right set rather than the control room's own (`suggestionsFor`'s doc).
    tabsStore.openTab("/logs/Pod/checkout/api-0");
    setup();
    // Suggestions live in the expanded panel, not the always-visible prompt —
    // focusing the prompt is what opens it (`ConsoleDock`'s own contract).
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(await screen.findByText("Summarise the last 500 lines")).toBeTruthy();
  });

  it("asks the agent immediately when a suggestion is picked", async () => {
    const user = userEvent.setup();
    tabsStore.openTab("/logs/Pod/checkout/api-0");
    setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    await user.click(await screen.findByText("Summarise the last 500 lines"));
    expect(askAgent).toHaveBeenCalledWith("Summarise the last 500 lines");
  });

  it("turns into the command palette when the query starts with a slash", async () => {
    const user = userEvent.setup();
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/");
    expect(await screen.findByText(/^Command$/)).toBeTruthy();
  });

  it("says so, verbatim, when no command matches", async () => {
    const user = userEvent.setup();
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/zzzz");
    expect(await screen.findByText("No command matches. Press ⏎ to ask the agent instead.")).toBeTruthy();
  });

  it("runs the matched command on Enter, rather than asking it as a question", async () => {
    const user = userEvent.setup();
    const onToggleTheme = vi.fn();
    setup({ onToggleTheme });
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/theme{Enter}");
    expect(onToggleTheme).toHaveBeenCalledTimes(1);
    expect(askAgent).not.toHaveBeenCalled();
  });

  it("clicking a command row runs it too, not only Enter", async () => {
    const user = userEvent.setup();
    const onToggleTheme = vi.fn();
    setup({ onToggleTheme });
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/theme");
    await user.click(await screen.findByText("Toggle theme"));
    expect(onToggleTheme).toHaveBeenCalledTimes(1);
  });

  it("does not accept a question while the workspace is covered", () => {
    lockWorkspace();
    setup();
    expect(screen.queryByRole("textbox")).toBeNull();
  });
});
