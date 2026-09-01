import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Console } from "./Console";
import { ConsoleProvider, useConsole } from "../console";
import { resetContexts, setContexts } from "../lib/clusters";
import { defaultState } from "../lib/tabs";
import { logsRoute } from "../screens/Logs";
import * as tabsStore from "../lib/tabsStore";
import { lockWorkspace, resetLock, __setKnownVaultMode } from "./LockGate";
import type { ClusterContext } from "@srelens/core";

const {
  useAgentRun,
  useRun,
  askAgent,
  clearAgentRun,
  dismissAgentError,
  selectRun,
  stopAgentRun,
  useActiveRunKey,
  chooseAgent,
} = vi.hoisted(() => ({
  chooseAgent: vi.fn(),
  selectRun: vi.fn(),
  stopAgentRun: vi.fn(),
  // Typed, or `vi.fn(() => null)` infers `() => null` and a test cannot hand
  // it a real key.
  useActiveRunKey: vi.fn<() => string | null>(() => null),
  useAgentRun: vi.fn(),
  // The dock reads its OWN route's run, not the active one — so this is the
  // hook under test here, and tests that care about which key it was handed
  // assert on its argument.
  useRun: vi.fn(),
  askAgent: vi.fn(),
  clearAgentRun: vi.fn(),
  dismissAgentError: vi.fn(),
}));
vi.mock("../lib/agentRun", () => ({
  useAgentRun,
  useRun,
  askAgent,
  clearAgentRun,
  dismissAgentError,
  selectRun,
  stopAgentRun,
  useActiveRunKey,
  chooseAgent,
}));

// The dock is desktop-only: nothing in a browser can answer a question, since
// `api_command.rs` has no `chat_*` arm. jsdom is not Tauri, so without this
// every test below would be asserting about a dock that correctly renders
// nothing. The web case has its own test.
const { isTauri, listAgents, isApplePlatform } = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  listAgents: vi.fn(),
  // `apple` comes from the PROVIDER now, which derives it rather than taking a
  // prop — so the platform is mocked here instead of passed to `setup`.
  isApplePlatform: vi.fn(() => true),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  isTauri,
  listAgents,
  isApplePlatform,
}));

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

function setup({ onToggleTheme = () => {} }: { onToggleTheme?: () => void } = {}) {
  // The dock takes no props now: it reads `apple` and `onToggleTheme` from the
  // provider, so it can be mounted anywhere beneath it — the window's bottom
  // edge on most screens, and `/agent`'s own main column on that one.
  return render(
    <ConsoleProvider onToggleTheme={onToggleTheme}>
      <Elsewhere />
      <Console />
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
  isTauri.mockReturnValue(true);
  isApplePlatform.mockReturnValue(true);
  listAgents.mockResolvedValue([
    { kind: "claude", label: "Claude Code", available: true, gated: false, path: "/c", version: "1", installUrl: "" },
  ]);
  // Saying `isTauri` is true means a vault EXISTS, and a fresh lock store has
  // read no mode — which counts as covered, so `sealed` would hide the dock
  // for a reason that has nothing to do with the test. Same beforeEach line,
  // and same reason, as `AgentConsent.test.tsx`.
  __setKnownVaultMode("unlocked");
  resetContexts();
  useAgentRun.mockReset().mockReturnValue(runState());
  useRun.mockReset().mockReturnValue(runState());
  askAgent.mockReset();
  clearAgentRun.mockReset();
  selectRun.mockReset();
  stopAgentRun.mockReset();
  chooseAgent.mockReset();
  useActiveRunKey.mockReset().mockReturnValue(null);
});

describe("Console", () => {
  it("asks the agent with what was typed at the prompt, and clears it", async () => {
    const user = userEvent.setup();
    setup();
    const input = screen.getByRole("textbox", { name: "Console prompt" }) as HTMLInputElement;
    await user.type(input, "why{Enter}");
    // The cluster travels with the question: every MCP tool call takes an
    // explicit context, and an agent given none has to guess one.
    expect(askAgent).toHaveBeenCalledWith("why", expect.objectContaining({ about: expect.objectContaining({ cluster: expect.any(String) }) }));
    expect(input.value).toBe("");
  });

  /**
   * The dock shows the conversation about the thing the reader is LOOKING at,
   * and follows them as they navigate. `/agent` is the surface that stays put,
   * on whatever the rail selected — these are the two halves of "one
   * conversation per subject".
   */
  it("reads the run for its OWN route, not whichever was asked into last", async () => {
    const prod = ctx("prod-eu-id", "prod-eu");
    setContexts([prod]);
    tabsStore.setState(defaultState([prod]));
    tabsStore.openTab(logsRoute("Pod", "ns", "ai-editor"), { clusterName: "prod-eu" });
    setup();
    // The key it asked for names the pod, not the cluster alone.
    expect(useRun).toHaveBeenCalledWith(expect.stringContaining("Pod"));
    expect(useRun).toHaveBeenCalledWith(expect.stringContaining("ai-editor"));
    // And never the ambient "whatever is active" read.
    expect(useAgentRun).not.toHaveBeenCalled();
  });

  it("changes which conversation it shows when the reader navigates", async () => {
    const prod = ctx("prod-eu-id", "prod-eu");
    setContexts([prod]);
    tabsStore.setState(defaultState([prod]));
    tabsStore.openTab(logsRoute("Pod", "ns", "ai-editor"), { clusterName: "prod-eu" });
    const view = setup();
    const onPod = useRun.mock.calls.at(-1)?.[0];

    view.unmount();
    useRun.mockClear();
    tabsStore.openTab("/k/statefulsets", { clusterName: "prod-eu" });
    setup();
    const onList = useRun.mock.calls.at(-1)?.[0];

    expect(onList).not.toBe(onPod);
    expect(onList).toContain("statefulsets");
  });

  /**
   * "How to go to full mode, tab view" — there was no way from the dock to
   * `/agent` at all; a reader had to know the left nav has an Agent entry
   * under Investigate.
   */
  it("offers a way into the full view, and opens it on the dock's own conversation", async () => {
    const user = userEvent.setup();
    const prod = ctx("prod-eu-id", "prod-eu");
    setContexts([prod]);
    tabsStore.setState(defaultState([prod]));
    tabsStore.openTab(logsRoute("Pod", "ns", "ai-editor"), { clusterName: "prod-eu" });
    setup();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));

    await user.click(screen.getByRole("button", { name: /full view/i }));

    // The /agent tab is open...
    const routes = tabsStore.currentWorkspace().tabs.map((t) => t.route);
    expect(routes).toContain("/agent");
    // ...and it shows the subject the dock was on, not whichever conversation
    // happened to be active.
    expect(selectRun).toHaveBeenCalledWith(expect.stringContaining("ai-editor"));
  });

  it("clears its OWN conversation, not whichever is active", async () => {
    const user = userEvent.setup();
    const prod = ctx("prod-eu-id", "prod-eu");
    setContexts([prod]);
    tabsStore.setState(defaultState([prod]));
    tabsStore.openTab(logsRoute("Pod", "ns", "ai-editor"), { clusterName: "prod-eu" });
    useRun.mockReturnValue(runState({ turns: [{ id: 1, role: "user", text: "q", calls: [], at: 1 }] }));
    setup();
    // The Clear control exists only on an open dock.
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    await user.click(screen.getByRole("button", { name: /clear console/i }));
    // Keyed, not ambient: the dock and `/agent` can be showing different runs.
    expect(clearAgentRun).toHaveBeenCalledWith(expect.stringContaining("ai-editor"));
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
    expect(askAgent).toHaveBeenCalledWith(
      "what is unhealthy",
      expect.objectContaining({ about: expect.objectContaining({ cluster: "prod-eu" }) }),
    );
  });

  /**
   * The end-to-end shape of the defect a screenshot caught. On a pod's logs
   * tab, "Summarise this stream" reached the agent with the cluster alone —
   * no namespace, no pod — so it had no target and went hunting through
   * `kube-system` and three unrelated namespaces for a stream to read.
   */
  it("carries the pod and namespace when the question is asked from a logs tab", async () => {
    const user = userEvent.setup();
    const prod = ctx("prod-eu-id", "prod-eu");
    setContexts([prod]);
    tabsStore.setState(defaultState([prod]));
    // The reader is on ai-editor's logs, exactly as in the report.
    tabsStore.openTab(logsRoute("Pod", "m01-cnips-01-services", "ai-editor"), { clusterName: "prod-eu" });
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "summarise this stream{Enter}");
    expect(askAgent).toHaveBeenCalledWith(
      "summarise this stream",
      expect.objectContaining({
        about: expect.objectContaining({
          cluster: "prod-eu",
          namespace: "m01-cnips-01-services",
          kind: "Pod",
          name: "ai-editor",
          surface: "logs",
        }),
      }),
    );
  });

  it("opens and forwards a question asked from anywhere else", async () => {
    const user = userEvent.setup();
    setup();
    // Closed to begin with: nothing to read, so no output region.
    expect(screen.queryByRole("log")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.getByRole("log", { name: "Console output" })).toBeDefined();
    expect(askAgent).toHaveBeenCalledWith("x", expect.objectContaining({ about: expect.objectContaining({ cluster: expect.any(String) }) }));
  });

  it("prints the console accelerator for the platform", () => {
    setup();
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
      expect.objectContaining({ about: expect.objectContaining({ cluster: expect.any(String) }) }),
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

  /**
   * P1 (#392 review round 7): every dock question begins with `chat_start`,
   * and the web command dispatcher (`crates/server/src/api_command.rs`) has no
   * `chat_*` or `agent_list` arm — it answers `404 unknown command`. A prompt
   * fixed to the bottom of every tab that can only fail is worse than no
   * prompt. `/agent` carries the explanation, which is where a reader looking
   * for the agent goes.
   */
  it("draws no dock at all in the browser, where no question could be answered", () => {
    isTauri.mockReturnValue(false);
    const { container } = setup();
    expect(screen.queryByRole("textbox", { name: "Console prompt" })).toBeNull();
    expect(container.querySelector("[data-slot]")).toBeNull();
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
    useRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "hi", calls: [], at: 0 }] }),
    );
    setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(screen.getAllByRole("log")).toHaveLength(1);
  });

  it("keeps the dock's live region on for the ordinary transcript thread", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(
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
        <Console />
      </ConsoleProvider>,
    );
    const input = screen.getByRole("textbox", { name: "Console prompt" });
    expect(input.getAttribute("placeholder")).toBe("Ask about prod-eu / checkout-api");
  });

  it("shows the exchange count in the header, pluralised", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "hi", calls: [], at: 0 }] }),
    );
    const { rerender } = setup();
    await user.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(screen.getByText("1 exchange")).toBeDefined();

    useRun.mockReturnValue(
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
        <Console />
      </ConsoleProvider>,
    );
    expect(screen.getByText("2 exchanges")).toBeDefined();
  });

  it("wires the dock's Stop to the store, and only while a turn is in flight", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(
      runState({ busy: true, turns: [{ id: 1, role: "user", text: "q", calls: [], at: 1 }] }),
    );
    setup();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(stopAgentRun).toHaveBeenCalledTimes(1);
  });

  /**
   * `/agent` is the FULL VIEW of whichever conversation is selected, not a
   * subject of its own. The dock there was keyed on `/agent`, so the two
   * surfaces on that screen sat on different runs: a full transcript above,
   * and "Start here" suggestions in the dock beneath it.
   */
  it("shows the active conversation on /agent, not one keyed by that route", () => {
    useActiveRunKey.mockReturnValue("prod-eu|Pod|ns|mongodb-0");
    tabsStore.setState(defaultState([]));
    tabsStore.openTab("/agent", {});
    setup();
    expect(useRun).toHaveBeenCalledWith("prod-eu|Pod|ns|mongodb-0");
    // Never the route-derived key, which is what made the dock a second,
    // empty conversation on that screen.
    expect(useRun).not.toHaveBeenCalledWith(expect.stringContaining("/agent"));
  });

  /**
   * Reported from use, with a full transcript on screen above the dock: the
   * suggestions came back on `/agent`. Neither existing test covers it — one
   * pins the KEY the dock is handed there, the other pins the empty-run case
   * off `/agent`. This is the pair: on `/agent`, an empty dock run must still
   * show no suggestions, because that screen has its own empty state saying
   * how to begin and the dock sits directly beneath a transcript.
   */
  it("offers no Start here suggestions on /agent even when its own run is empty", async () => {
    const user = userEvent.setup();
    // Nothing selected, so the dock's run is genuinely empty — the state that
    // used to reach the suggestions branch.
    useActiveRunKey.mockReturnValue(null);
    useRun.mockReturnValue(runState());
    tabsStore.setState(defaultState([]));
    tabsStore.openTab("/agent", {});
    setup();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.queryByText(/start here/i)).toBeNull();
  });

  /**
   * The structural half. The route string was the only guard, and "Start here"
   * came back under a full transcript anyway — so the mount site that KNOWS it
   * is the full view says so, and that alone is enough.
   */
  it("offers no Start here suggestions when mounted as the full view's own composer", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(runState());
    // Deliberately NOT an /agent tab: this pins the prop, not the route.
    tabsStore.setState(defaultState([]));
    render(
      <ConsoleProvider onToggleTheme={() => {}}>
        <Elsewhere />
        <Console fullView />
      </ConsoleProvider>,
    );
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.queryByText(/start here/i)).toBeNull();
  });

  it("offers no Full view link when mounted as the full view's own composer", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(runState());
    tabsStore.setState(defaultState([]));
    render(
      <ConsoleProvider onToggleTheme={() => {}}>
        <Elsewhere />
        <Console fullView />
      </ConsoleProvider>,
    );
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.queryByRole("button", { name: /full view/i })).toBeNull();
  });

  /**
   * `Full view` opens `/agent`. Offered while ALREADY on `/agent` it is a
   * control that does nothing a reader can see, on the one screen where the
   * dock is the full view's own composer.
   */
  it("offers no Full view link on /agent", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(runState());
    tabsStore.setState(defaultState([]));
    tabsStore.openTab("/agent", {});
    setup();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.queryByRole("button", { name: /full view/i })).toBeNull();
  });

  it("offers no Start here suggestions once the conversation has questions in it", async () => {
    const user = userEvent.setup();
    useRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "asked already", calls: [], at: 1 }] }),
    );
    setup();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.queryByText(/start here/i)).toBeNull();
  });

  it("offers none while a question is being answered either", async () => {
    const user = userEvent.setup();
    // No turns yet AND busy: suggesting a second question that would only be
    // refused is worse than showing nothing.
    useRun.mockReturnValue(runState({ busy: true }));
    setup();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.queryByText(/start here/i)).toBeNull();
    // And the bar says what is happening.
    expect(screen.getByText(/working/i)).toBeTruthy();
  });

  describe("pasting a screenshot", () => {
    const shot = () => new File([new Uint8Array([1, 2, 3])], "shot.png", { type: "image/png" });

    it("shows what is attached, and sends it with the question", async () => {
      const user = userEvent.setup();
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      const box = screen.getByRole("textbox", { name: "Console prompt" });

      fireEvent.paste(box, { clipboardData: { files: [shot()], types: ["Files"] } });
      // Shown BEFORE it is sent: a screenshot the reader believes is attached
      // and is not would only be discovered by the answer ignoring it.
      expect(await screen.findByAltText("Attachment 1")).toBeTruthy();

      await user.type(box, "what is wrong here");
      fireEvent.keyDown(box, { key: "Enter" });
      await vi.waitFor(() => {
        expect(askAgent).toHaveBeenCalledWith(
          "what is wrong here",
          expect.objectContaining({ images: [expect.stringContaining("data:image/png")] }),
        );
      });
    });

    it("lets the reader take one off again", async () => {
      const user = userEvent.setup();
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      fireEvent.paste(screen.getByRole("textbox", { name: "Console prompt" }), {
        clipboardData: { files: [shot()], types: ["Files"] },
      });
      await screen.findByAltText("Attachment 1");
      await user.click(screen.getByRole("button", { name: /remove attachment 1/i }));
      expect(screen.queryByAltText("Attachment 1")).toBeNull();
    });

    it("holds nothing over to the next question", async () => {
      const user = userEvent.setup();
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      const box = screen.getByRole("textbox", { name: "Console prompt" });
      fireEvent.paste(box, { clipboardData: { files: [shot()], types: ["Files"] } });
      await screen.findByAltText("Attachment 1");

      await user.type(box, "first");
      fireEvent.keyDown(box, { key: "Enter" });
      await vi.waitFor(() => expect(askAgent).toHaveBeenCalled());
      askAgent.mockClear();

      await user.type(box, "second");
      fireEvent.keyDown(box, { key: "Enter" });
      await vi.waitFor(() => {
        // The screenshot went with the FIRST question; sending it again with
        // the second would attach something the reader did not mean.
        expect(askAgent).toHaveBeenCalledWith("second", expect.objectContaining({ images: undefined }));
      });
    });
  });

  describe("the agent picker", () => {
    it("sits in the composer's footer, on every screen", async () => {
      const user = userEvent.setup();
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      // Beside `+` in the footer, not on one screen's rail: choosing the agent
      // is part of asking, and the composer is everywhere.
      expect(await screen.findByRole("button", { name: /claude code/i })).toBeTruthy();
    });

    it("switches which agent the next question goes to", async () => {
      const user = userEvent.setup();
      listAgents.mockResolvedValue([
        { kind: "claude", label: "Claude Code", available: true, gated: false, path: "/c", version: "1", installUrl: "" },
        { kind: "codex", label: "Codex", available: true, gated: false, path: "/x", version: "1", installUrl: "" },
      ]);
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      await user.click(await screen.findByRole("button", { name: /claude code/i }));
      await user.click(await screen.findByRole("option", { name: /codex/i }));
      expect(chooseAgent).toHaveBeenCalledWith("codex");
    });

    it("offers no picker when nothing can be offered, rather than an empty one", async () => {
      const user = userEvent.setup();
      listAgents.mockResolvedValue([]);
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      expect(screen.queryByRole("button", { name: /claude/i })).toBeNull();
    });

    it("does not offer a gated agent, even where one is installed", async () => {
      const user = userEvent.setup();
      listAgents.mockResolvedValue([
        { kind: "codex", label: "Codex", available: true, gated: true, path: "/x", version: "1", installUrl: "" },
      ]);
      setup();
      await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
      expect(screen.queryByRole("button", { name: /codex/i })).toBeNull();
    });
  });
});