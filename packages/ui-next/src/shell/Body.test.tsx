import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { useEffect, useState } from "react";

const { reading, stopped } = vi.hoisted(() => ({ reading: vi.fn(), stopped: vi.fn() }));
import userEvent from "@testing-library/user-event";

// Type-only, so it is erased and cannot be hoisted above the mock below.
import type { RoutedScreenProps } from "../lib/routes";

vi.mock("../lib/routes", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/routes")>();
  // Consumes everything the host injects, not only the route: `Settings` needs
  // `ported` and `onSwitchToClassic` — which ui-next cannot import, only be
  // handed — and a fake that took `route` alone would let `Body` drop them
  // silently. See `ScreenComponent` in `lib/routes.ts`.
  const Fake = ({ route, ported, onSwitchToClassic, onLocked }: RoutedScreenProps) => (
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
      <button type="button" onClick={() => onLocked()}>
        seal from the screen
      </button>
    </>
  );
  const Editor = () => {
    const [draft, setDraft] = useState("initial manifest");
    useEffect(() => { reading(); return () => { stopped(); }; }, []);
    return <textarea aria-label="Manifest" value={draft} onChange={e => setDraft(e.target.value)} />;
  };
  return { ...real, screenFor: (route: string) => (route === "/applog" ? Fake : route.startsWith("/edit/") || route.startsWith("/new/") ? Editor : null) };
});

import { Body } from "./Body";

describe("Body", () => {
  it.each(["/edit/prod-eu/ConfigMap/default/example", "/new/prod-eu"])("preserves the draft and suspends effects while paused: %s", route => {
    reading.mockClear(); stopped.mockClear();
    const props = { route, ported: [], onOpenInClassic: () => {}, onLocked: () => {} };
    const { rerender } = render(<Body {...props} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Manifest" }), { target: { value: "unsaved changes" } });
    rerender(<Body {...props} pausedContext={{ name: "prod-eu", stableId: "prod", cluster: "prod", server: "", isCurrent: false, sourceFile: "", authKind: "token" }} />);
    expect(screen.queryByRole("textbox", { name: "Manifest" })).toBeNull();
    expect(stopped).toHaveBeenCalledTimes(1);
    rerender(<Body {...props} />);
    expect((screen.getByRole("textbox", { name: "Manifest" }) as HTMLTextAreaElement).value).toBe("unsaved changes");
    expect(reading).toHaveBeenCalledTimes(2);
  });

  it("renders the screen when one is registered for the route", () => {
    render(<Body route="/applog" ported={[]} onOpenInClassic={() => {}} onLocked={() => {}} />);
    expect(screen.getByText("screen for /applog")).toBeDefined();
    expect(screen.queryByRole("button", { name: /open in classic/i })).toBeNull();
  });

  it("renders the Placeholder when none is", () => {
    render(<Body route="/helm" ported={[]} onOpenInClassic={() => {}} onLocked={() => {}} />);
    expect(screen.getByRole("heading", { level: 1, name: "Helm" })).toBeDefined();
    expect(screen.getByRole("button", { name: /open in classic/i })).toBeDefined();
  });

  it("passes the route through to the screen", () => {
    render(<Body route="/applog" ported={[]} onOpenInClassic={() => {}} onLocked={() => {}} />);
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
        onLocked={() => {}}
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

  it("hands the screen the way to raise the lock cover, untouched", async () => {
    // The third hop of the lock seam: `Window` owns the cover, `Settings`
    // owns the button, and this is the only path between them. Called with no
    // arguments — the contract is a zero-argument, fire-and-forget raise, and
    // a `Body` that curried a route or a cluster into it would be handing
    // `Settings` a different function than the one the window mounted.
    const onLocked = vi.fn();
    render(
      <Body route="/applog" ported={[]} onOpenInClassic={() => {}} onLocked={onLocked} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "seal from the screen" }));
    expect(onLocked).toHaveBeenCalledTimes(1);
  });

  it("does not mount a pinned screen while its cluster is paused", () => {
    render(
      <Body
        route="/applog"
        ported={[]}
        onOpenInClassic={() => {}}
        onLocked={() => {}}
        pausedContext={{ name: "prod-eu", stableId: "prod", cluster: "prod", server: "", isCurrent: false, sourceFile: "", authKind: "token" }}
      />,
    );
    expect(screen.getByText("prod-eu is paused")).toBeDefined();
    expect(screen.queryByText("screen for /applog")).toBeNull();
  });
});
