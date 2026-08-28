import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Console } from "./Console";
import { ConsoleProvider, useConsole } from "../console";

/** Anything else on screen, putting a question to the console from outside it. */
function Elsewhere() {
  const { ask } = useConsole();
  return (
    <button type="button" onClick={() => ask("x")}>
      Ask from elsewhere
    </button>
  );
}

function setup({ apple = true }: { apple?: boolean } = {}) {
  return render(
    <ConsoleProvider>
      <Elsewhere />
      <Console apple={apple} />
    </ConsoleProvider>,
  );
}

describe("Console", () => {
  it("shows what was asked at the prompt", async () => {
    const user = userEvent.setup();
    setup();
    await user.type(screen.getByRole("textbox", { name: "Console prompt" }), "why{Enter}");
    expect(screen.getByText("You asked: why")).toBeDefined();
  });

  it("opens and answers a question asked from anywhere else", async () => {
    const user = userEvent.setup();
    setup();
    // Closed to begin with: nothing to read, so no output region.
    expect(screen.queryByRole("log")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Ask from elsewhere" }));
    expect(screen.getByRole("log", { name: "Console output" })).toBeDefined();
    expect(screen.getByText("You asked: x")).toBeDefined();
  });

  it("prints the console accelerator for the platform", () => {
    setup({ apple: true });
    expect(screen.getByText("⌘K")).toBeDefined();
  });
});
