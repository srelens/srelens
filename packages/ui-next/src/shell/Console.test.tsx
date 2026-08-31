import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Console } from "./Console";
import { ConsoleProvider, useConsole } from "../console";
import { resetContexts, setContexts } from "../lib/clusters";
import { defaultState } from "../lib/tabs";
import * as tabsStore from "../lib/tabsStore";
import { lockWorkspace, resetLock } from "./LockGate";
import type { ClusterContext } from "@srelens/core";

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

/** The same shape `Window.test.tsx`'s own `ctx` builds, for the tests below
 *  that need a real, resolvable cluster context rather than the empty
 *  default. */
const ctx = (stableId: string, name = stableId): ClusterContext => ({
  name,
  stableId,
  cluster: name,
  server: "",
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
});

beforeEach(() => {
  // A clean workspace for every test: the route-aware suggestions test opens
  // a `/logs` tab of its own, and nothing here may leak into a later test —
  // the store is module-level, not reset between tests on its own.
  tabsStore.setState(defaultState([]));
  resetLock();
  resetContexts();
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
    // The cluster travels with the question: every MCP tool call takes an
    // explicit context, and an agent given none has to guess one.
    expect(askAgent).toHaveBeenCalledWith("why", expect.objectContaining({ context: expect.any(String) }));
    expect(input.value).toBe("");
  });

  it("pins the cluster on screen to the question, by name", async () => {
    const user = userEvent.setup();
    const prod = ctx("prod-eu-id", "prod-eu");
    setContexts([prod]);
    tabsStore.setState(defaultState([prod]));
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "what is unhealthy{Enter}");
    // By name — an `expect.any(String)` here would pass on the empty context
    // this dock sends when no cluster is active, which is the bug.
    expect(askAgent).toHaveBeenCalledWith("what is unhealthy", expect.objectContaining({ context: "prod-eu" }));
  });

  it("opens and forwards a question asked from anywhere else", async () => {
    const user = userEvent.setup();
    setup();
    // Closed to begin with: nothing to read, so no output region.
    expect(screen.queryByRole("log")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.getByRole("log", { name: "Console output" })).toBeDefined();
    expect(askAgent).toHaveBeenCalledWith("x", expect.objectContaining({ context: expect.any(String) }));
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
    expect(askAgent).toHaveBeenCalledWith(
      "Summarise the last 500 lines",
      expect.objectContaining({ context: expect.any(String) }),
    );
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

  // M14: `matched[0]` at the Enter handler had no fixture with more than one
  // match — mutating it to `matched[matched.length - 1]` passed every test in
  // this file, because "/theme" above matches exactly one command. Two
  // clusters whose names share a substring produce two "Switch to …" Cluster
  // commands in a KNOWN emission order (`clusterCommands` maps `deps.clusters`
  // in order), so the one Enter actually runs is observable in real store
  // state, not merely in which row happened to render on top.
  it("runs the FIRST matched command on Enter when the query matches several, not the last", async () => {
    const user = userEvent.setup();
    const contexts = [ctx("prod-eu-id", "prod-eu"), ctx("prod-us-id", "prod-us")];
    setContexts(contexts);
    tabsStore.setState(defaultState(contexts));
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/switch to prod{Enter}");
    // The first match is "Switch to prod-eu" (prod-eu-id is `clusters[0]`);
    // the LAST match is "Switch to prod-us". Only the first leaves the active
    // cluster at prod-eu-id — the mutation this test exists to catch would
    // leave it at prod-us-id instead.
    expect(tabsStore.currentWorkspace().activeCluster).toBe("prod-eu-id");
  });

  // P1a (#392 review): `setActiveCluster` refuses an id outside
  // `workspace.clusters` and returns the workspace untouched, but the command
  // went on to `openTab` regardless — relabelling a tab with a cluster that
  // never became active, and leaving every action under it running against
  // the previous cluster.
  it("offers no Cluster command for a context this workspace does not hold", async () => {
    const user = userEvent.setup();
    const inWorkspace = ctx("prod-eu-id", "prod-eu");
    const outside = ctx("prod-us-id", "prod-us");
    // The kubeconfig has both; the workspace holds only the first.
    setContexts([inWorkspace, outside]);
    tabsStore.setState(defaultState([inWorkspace]));
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/switch to prod");
    expect(await screen.findByText("Switch to prod-eu")).toBeTruthy();
    expect(screen.queryByText("Switch to prod-us")).toBeNull();
  });

  it("leaves the active cluster where it was when the only match is outside the workspace", async () => {
    const user = userEvent.setup();
    const inWorkspace = ctx("prod-eu-id", "prod-eu");
    const outside = ctx("staging-id", "staging");
    setContexts([inWorkspace, outside]);
    tabsStore.setState(defaultState([inWorkspace]));
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/switch to staging{Enter}");
    // No command matched, so §F's rule applies and the text is asked as a
    // question instead — what must NOT happen is a tab relabelled "staging"
    // over a workspace still pointed at prod-eu.
    expect(tabsStore.currentWorkspace().activeCluster).toBe("prod-eu-id");
    expect(tabsStore.currentWorkspace().tabs.some((t) => t.sub === "staging")).toBe(false);
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

/**
 * I7: `ConsoleDock`'s own body already declares one `role="log"` live region
 * around whatever it is given as children (`ConsoleDock.tsx:187-193`).
 * `Transcript` used to declare a SECOND, nested one unconditionally — two
 * `role="log"` regions announce inconsistently and often twice, and the
 * kit's own doc on the `live` prop names exactly the palette/suggestions
 * scenario this fixes: a screen-reader user typing `/re` would have every
 * keystroke's re-rendered `CommandRows` read out inside a polite region,
 * three times over for one matched list.
 */
describe("Console — one live region, not two nested (I7)", () => {
  it("declares exactly one role=\"log\" element for the thread, not one nested inside the other", async () => {
    const user = userEvent.setup();
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "hi", calls: [], at: 0 }] }),
    );
    setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(screen.getAllByRole("log")).toHaveLength(1);
  });

  it("keeps the dock's live region on for the ordinary transcript thread", async () => {
    const user = userEvent.setup();
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "hi", calls: [], at: 0 }] }),
    );
    setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(screen.getByRole("log").getAttribute("aria-live")).toBe("polite");
  });

  it("turns the dock's live region off for suggestions, so a keystroke that changes the list is not read out", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(screen.getByRole("log").getAttribute("aria-live")).toBe("off");
  });

  it("turns the dock's live region off for the command palette, so typing / does not re-announce the matched list", async () => {
    const user = userEvent.setup();
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/");
    expect(screen.getByRole("log").getAttribute("aria-live")).toBe("off");
  });
});


/**
 * I2's own point: `agentCommands.test.ts` proves `commandsFor` builds the
 * right commands against a MOCK `CommandDeps` — it never runs the real
 * closures `Console.tsx` assembles (`openAction`/`openResource` calling into
 * `detailRoute`/`logsRoute`/`openTab`). These tests run a command end to end
 * through the real dock, and check the real `tabsStore`'s resulting state —
 * not a spy on `openTab`, so a passing test means the actual navigation
 * happened, not merely that some function was invoked.
 */
describe("Console — debt 2's real navigation", () => {
  it("Follow logs opens the real logs route, carrying the resource's identity and the cluster", async () => {
    const user = userEvent.setup();
    const contexts = [ctx("prod", "prod-eu")];
    setContexts(contexts);
    // `defaultState`, not `setActiveCluster` on top of the empty default: the
    // latter refuses unless the id is already in the WORKSPACE's own
    // `clusters` list, which the plain `defaultState([])` from `beforeEach`
    // never populated.
    tabsStore.setState(defaultState(contexts));
    // No `clusterName` here on purpose: the tab starts with no `sub`, so a
    // `sub` of "prod-eu" after the command can only have come from the real
    // `openAction`/`openResource` closure actually calling `openTab` with it.
    tabsStore.openTab("/k/Pod/checkout/api-0");
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/follow logs{Enter}");
    expect(tabsStore.activeRoute()).toBe("/logs/Pod/checkout/api-0");
    const tab = tabsStore.currentWorkspace().tabs.find((t) => t.route === "/logs/Pod/checkout/api-0");
    expect(tab?.sub).toBe("prod-eu");
  });

  it("Open a shell reaches Terminals — generic navigation, per debt 2's resolution", async () => {
    const user = userEvent.setup();
    tabsStore.openTab("/k/Pod/checkout/api-0");
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/open a shell{Enter}");
    expect(tabsStore.activeRoute()).toBe("/terminals");
  });

  it("Port forward reaches Forwards — generic navigation, per debt 2's resolution", async () => {
    const user = userEvent.setup();
    tabsStore.openTab("/k/Pod/checkout/api-0");
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/port forward{Enter}");
    expect(tabsStore.activeRoute()).toBe("/forwards");
  });

  it("an Action command opens the detail route with the resource's identity and the cluster", async () => {
    const user = userEvent.setup();
    const contexts = [ctx("prod", "prod-eu")];
    setContexts(contexts);
    tabsStore.setState(defaultState(contexts));
    // Same "no `sub` yet" setup as the logs test above, and for the same
    // reason: a `sub` of "prod-eu" afterward can only be the real
    // `openAction` closure's own `openTab({ clusterName })`.
    tabsStore.openTab("/k/Deployment/checkout/api");
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "/restart{Enter}");
    expect(tabsStore.activeRoute()).toBe("/k/Deployment/checkout/api");
    const tab = tabsStore.currentWorkspace().tabs.find((t) => t.route === "/k/Deployment/checkout/api");
    expect(tab?.sub).toBe("prod-eu");
  });
});

describe("Console — header details", () => {
  it("uses a fallback placeholder before the dock has a scope", () => {
    setup();
    const input = screen.getByRole("textbox", { name: "Console prompt" });
    expect(input.getAttribute("placeholder")).toBe("Ask about this cluster");
  });

  it("uses the scope in the placeholder once Window has scoped the dock", () => {
    render(
      <ConsoleProvider initialScope="prod-eu / checkout-api">
        <Console apple onToggleTheme={() => {}} />
      </ConsoleProvider>,
    );
    const input = screen.getByRole("textbox", { name: "Console prompt" });
    expect(input.getAttribute("placeholder")).toBe("Ask about prod-eu / checkout-api");
  });

  it("shows the exchange count in the header, pluralised", async () => {
    const user = userEvent.setup();
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "hi", calls: [], at: 0 }] }),
    );
    const { rerender } = setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(screen.getByText("1 exchange")).toBeDefined();

    useAgentRun.mockReturnValue(
      runState({
        turns: [
          { id: 1, role: "user", text: "hi", calls: [], at: 0 },
          { id: 2, role: "agent", text: "hello", calls: [], at: 0 },
          { id: 3, role: "user", text: "again", calls: [], at: 0 },
        ],
      }),
    );
    rerender(
      <ConsoleProvider>
        <Elsewhere />
        <Console apple onToggleTheme={() => {}} />
      </ConsoleProvider>,
    );
    expect(screen.getByText("2 exchanges")).toBeDefined();
  });
});
