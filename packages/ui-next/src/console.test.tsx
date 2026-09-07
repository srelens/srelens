import { describe, it, expect, vi } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import { ConsoleProvider, useConsole } from "./console";

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
