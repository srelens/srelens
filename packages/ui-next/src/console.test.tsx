import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import type { ClusterContext } from "@srelens/core";
import { ConsoleProvider, useAsk, useConsole } from "./console";
import { setAssistantEnabled, setClusterAssistant } from "./lib/assistant";
import { resetContexts, ScreenCluster, setContexts } from "./lib/clusters";

/** A stand-in for the dock: registers a submitter and records what it is asked. */
function Dock({ asked }: { asked: (q: string) => void }) {
  const { registerSubmit } = useConsole();
  useEffect(() => registerSubmit(asked), [registerSubmit, asked]);
  return null;
}

function Opener() {
  const { open } = useConsole();
  return <span data-testid="open">{String(open)}</span>;
}

describe("ConsoleProvider", () => {
  it("opens the console when something asks it a question", () => {
    const ask = vi.fn();
    render(
      <ConsoleProvider>
        <Opener />
        <Asker question="why is checkout-api restarting?" onReady={ask} />
      </ConsoleProvider>,
    );
    act(() => ask.mock.calls[0][0]());
    expect(screen.getByTestId("open").textContent).toBe("true");
  });

  it("delivers a question asked before the dock existed", async () => {
    // The mock waited 10ms and hoped. A dock that mounts slower than that —
    // under load, on a cold render — drops the question with no trace.
    const asked = vi.fn();
    const ask = vi.fn();
    const { rerender } = render(
      <ConsoleProvider>
        <Asker question="q" onReady={ask} />
      </ConsoleProvider>,
    );
    act(() => ask.mock.calls[0][0]());
    expect(asked).not.toHaveBeenCalled();

    rerender(
      <ConsoleProvider>
        <Asker question="q" onReady={ask} />
        <Dock asked={asked} />
      </ConsoleProvider>,
    );
    await waitFor(() => expect(asked).toHaveBeenCalledWith("why is checkout-api restarting?"));
  });

  it("delivers straight away once a dock is listening", async () => {
    const asked = vi.fn();
    const ask = vi.fn();
    render(
      <ConsoleProvider>
        <Dock asked={asked} />
        <Asker question="q" onReady={ask} />
      </ConsoleProvider>,
    );
    act(() => ask.mock.calls[0][0]());
    await waitFor(() => expect(asked).toHaveBeenCalledWith("why is checkout-api restarting?"));
  });

  it("forgets a dock that has gone away", async () => {
    // The mock never cleared its ref, so a question asked after the dock
    // unmounted called into a component that no longer existed.
    const asked = vi.fn();
    const ask = vi.fn();
    const { rerender } = render(
      <ConsoleProvider>
        <Dock asked={asked} />
        <Asker question="q" onReady={ask} />
      </ConsoleProvider>,
    );
    rerender(
      <ConsoleProvider>
        <Asker question="q" onReady={ask} />
      </ConsoleProvider>,
    );
    act(() => ask.mock.calls[0][0]());
    await new Promise((r) => setTimeout(r, 20));
    expect(asked).not.toHaveBeenCalled();
  });

  it("keeps the scope the console is asking about", () => {
    render(
      <ConsoleProvider initialScope="prod-eu / checkout-api">
        <Scope />
      </ConsoleProvider>,
    );
    expect(screen.getByTestId("scope").textContent).toBe("prod-eu / checkout-api");
  });

  it("refuses to be used outside a provider, rather than failing quietly", () => {
    // A hook returning undefined turns into "cannot read property of
    // undefined" three components away from the mistake.
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => render(<Scope />)).toThrow(/ConsoleProvider/);
    quiet.mockRestore();
  });
});

/** #968: a screen's Ask exists only where the AI Assistant may act. */
describe("useAsk", () => {
  const ctx = (name: string): ClusterContext => ({
    name, stableId: `/config#${name}`, key: `/config#${name}`, cluster: name, server: `https://${name}`,
    sourceFile: "/config", authKind: "token", isCurrent: false,
  });
  const prod = ctx("prod");
  const staging = ctx("staging");

  function Probe({ cluster }: { cluster?: string }) {
    const ask = useAsk(cluster);
    return <span data-testid="ask">{ask ? "ask" : "none"}</span>;
  }

  function renderOn(screenCluster: ClusterContext, cluster?: string) {
    return render(
      <ConsoleProvider>
        <ScreenCluster.Provider value={screenCluster}>
          <Probe cluster={cluster} />
        </ScreenCluster.Provider>
      </ConsoleProvider>,
    );
  }

  beforeEach(() => {
    resetContexts();
    setContexts([prod, staging]);
  });

  it("hands a screen the console's ask while the assistant is on for its cluster", () => {
    renderOn(prod);
    expect(screen.getByTestId("ask").textContent).toBe("ask");
  });

  it("hands nothing for a screen whose cluster has it switched off, and follows the switch", () => {
    renderOn(prod);
    act(() => setClusterAssistant(prod.stableId, false));
    expect(screen.getByTestId("ask").textContent).toBe("none");
  });

  it("asks about the cluster it is told, over the screen's", () => {
    setClusterAssistant(staging.stableId, false);
    renderOn(prod, "staging");
    expect(screen.getByTestId("ask").textContent).toBe("none");
  });

  it("hands nothing anywhere while the assistant is off", () => {
    setAssistantEnabled(false);
    renderOn(prod);
    expect(screen.getByTestId("ask").textContent).toBe("none");
  });
});

function Asker({ question, onReady }: { question: string; onReady: (fn: () => void) => void }) {
  const { ask } = useConsole();
  useEffect(() => {
    onReady(() => ask("why is checkout-api restarting?"));
  }, [ask, onReady, question]);
  return null;
}

function Scope() {
  const { scope } = useConsole();
  return <span data-testid="scope">{scope}</span>;
}
