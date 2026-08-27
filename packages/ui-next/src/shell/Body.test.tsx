import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Type-only, so it is erased and cannot be hoisted above the mock below.
import type { RoutedScreenProps } from "../lib/routes";

vi.mock("../lib/routes", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/routes")>();
  // Consumes everything the host injects, not only the route: `Settings` needs
  // `ported` and `onSwitchToClassic` — which ui-next cannot import, only be
  // handed — and a fake that took `route` alone would let `Body` drop them
  // silently. See `ScreenComponent` in `lib/routes.ts`.
  const Fake = ({ route, ported, onSwitchToClassic }: RoutedScreenProps) => (
    <>
      <p>screen for {route}</p>
      <ul>
        {ported.map((name) => (
          <li key={name} data-testid="injected-ported">
            {name}
          </li>
        ))}
      </ul>
      <button type="button" onClick={onSwitchToClassic}>
        leave from the screen
      </button>
    </>
  );
  return { ...real, screenFor: (route: string) => (route === "/applog" ? Fake : null) };
});

import { Body } from "./Body";

describe("Body", () => {
  it("renders the screen when one is registered for the route", () => {
    render(<Body route="/applog" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByText("screen for /applog")).toBeDefined();
    expect(screen.queryByRole("button", { name: /open in classic/i })).toBeNull();
  });

  it("renders the Placeholder when none is", () => {
    render(<Body route="/helm" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByRole("heading", { level: 1, name: "Helm" })).toBeDefined();
    expect(screen.getByRole("button", { name: /open in classic/i })).toBeDefined();
  });

  it("passes the route through to the screen", () => {
    render(<Body route="/applog" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByText(/\/applog/)).toBeDefined();
  });

  it("hands the screen what the host injected, the way it hands the Placeholder", async () => {
    // `Settings` renders the Appearance pane, whose ported list and design
    // toggle are step-11 scaffolding the host owns. Both already reach
    // `Placeholder` down this path; a screen gets the same two rather than a
    // second mechanism of its own.
    const onOpenInClassic = vi.fn();
    render(
      <Body
        route="/applog"
        clusterName="prod-eu"
        ported={["Aardvark ledger", "Basalt tally"]}
        onOpenInClassic={onOpenInClassic}
      />,
    );
    expect(screen.getAllByTestId("injected-ported").map((n) => n.textContent)).toEqual([
      "Aardvark ledger",
      "Basalt tally",
    ]);
    await userEvent.click(screen.getByRole("button", { name: "leave from the screen" }));
    // The route the screen is ON, and the cluster its tab is looking at — the
    // same pair `Placeholder`'s own button sends.
    expect(onOpenInClassic).toHaveBeenCalledWith("/applog", "prod-eu");
  });
});
