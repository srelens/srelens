import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { switchDesignMock } = vi.hoisted(() => ({ switchDesignMock: vi.fn() }));
vi.mock("../design", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../design")>()),
  switchDesign: switchDesignMock,
}));

import { AppearanceSettingsSection } from "./AppearanceSettingsSection";

beforeEach(() => {
  switchDesignMock.mockReset().mockResolvedValue({ ok: true });
  localStorage.clear();
});

describe("AppearanceSettingsSection", () => {
  it("says the new design is unfinished, before anyone opts in", () => {
    // Shipping a half-built UI behind a toggle is only defensible if the
    // toggle says so. Someone who opts in and finds empty screens should have
    // been told, not surprised.
    render(<AppearanceSettingsSection />);
    // Said in the description, not only in the button label — someone scanning
    // the setting should learn it without reading the options.
    expect(screen.getByText(/most screens are not there yet/i)).toBeDefined();
  });

  it("lists the screens that are already in the new design", () => {
    // "Most screens are not there yet" is only actionable if it says which
    // ones are. The list comes from PORTED_SCREENS, the same one the new
    // design's Placeholder reads, so the two cannot disagree.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Application log");
    expect(items).toContain("Release notes");
  });

  it("lists the resource screens among the ported ones, so the toggle is honest", () => {
    // This branch ported the entire resource surface: every /k/<kind> list,
    // the resource detail beneath them, and the Workloads view at
    // /resources. The toggle would be dishonest without them.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Workloads");
    expect(items).toContain("Resource lists and details");
  });

  it("lists Events, which has its own screen rather than riding the kind lists", () => {
    // `/events` is not one of the `/k/<kind>` lists: it has its own chrome —
    // the by-reason rail and the type filter — so it earns its own line here.
    // Without it the new design's own Placeholder tells a reader Events is
    // not ported while the screen is sitting there working.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Events");
  });

  it("lists the cluster overview, the first thing the new design's sidebar opens", () => {
    // `/overview` is the sidebar's first cluster node, so a reader trying the
    // new design lands on it before anything else. Leaving it off this list
    // would tell them the screen they are looking at does not exist.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Cluster overview");
  });

  it("lists Logs, which is a live stream and not one of the kind lists", () => {
    // `/logs` tails a workload's pods or one pod: its own screen, its own
    // route shape, and the one that keeps running after you switch away from
    // it. Someone weighing the toggle is weighing this in particular.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Logs");
  });

  it("lists port forwards, which run on regardless of which screen is open", () => {
    // A tunnel outlives the tab that started it — someone weighing the toggle
    // is weighing whether they can see and stop the ones they have running.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Port forwards");
  });

  it("lists terminals, whose sessions outlive the tab that started them", () => {
    // `/terminals` is where a shell opened from a resource row lands, and its
    // sessions keep running after the tab closes. A reader weighing the toggle
    // is weighing whether they can get a shell at all.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Terminals");
  });

  it("lists Helm, whose operations run on after the dialog that started them", () => {
    // The release table, the diff pane and the four operations. An upgrade or
    // a rollback outlives its dialog, so a reader weighing the toggle is
    // weighing whether they can drive Helm at all in the new design.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Helm");
  });

  it("lists the toolbox, the only screen that is about the machine and not a cluster", () => {
    // The managed kubectl, helm and krew under ~/.srelens/bin, plus what a
    // context's exec-auth needs. Nothing about it is cluster-scoped, so a
    // reader who does not see it named assumes it did not come along.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Toolbox");
  });

  it("lists the connections screen and the door a first run comes through", () => {
    // `/connections` is where a reader sees every cluster srelens can see and
    // which file each came from; `/connect` is the first-run door, and the only
    // screen either design shows with no cluster connected at all. A reader
    // weighing the toggle with nothing connected is weighing that one in
    // particular, so leaving it unnamed would tell them the new design has no
    // way in.
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Connections");
    expect(items).toContain("Connect a cluster");
  });

  it("lists the settings screen, which is where the way back lives on the other side", () => {
    // `/settings` is the new design's own six-pane screen, and its Appearance
    // pane is where this very toggle lives over there. A reader weighing the
    // switch is weighing whether they can find their way back at all, so
    // leaving it unnamed reads as "there is no settings screen once you go".
    render(<AppearanceSettingsSection />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toContain("Settings");
  });

  it("introduces the list, so the names are not a bare list under the hint", () => {
    render(<AppearanceSettingsSection />);
    expect(screen.getByText(/in the new design so far/i)).toBeDefined();
  });

  it("warns that switching reloads the window", () => {
    render(<AppearanceSettingsSection />);
    expect(screen.getByText(/reload/i)).toBeDefined();
  });

  it("switches to the new design when asked", async () => {
    render(<AppearanceSettingsSection />);
    await userEvent.click(screen.getByRole("button", { name: /new design/i }));
    expect(switchDesignMock).toHaveBeenCalledWith("next");
  });

  it("marks the design in use, so the current state is visible", () => {
    render(<AppearanceSettingsSection />);
    // No jest-dom in this project, so read the attribute directly.
    expect(
      screen.getByRole("button", { name: /classic design/i }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("does not reload when the chosen design is already active", async () => {
    // Re-picking the current design should be inert, not a pointless reload.
    render(<AppearanceSettingsSection />);
    await userEvent.click(screen.getByRole("button", { name: /classic design/i }));
    expect(switchDesignMock).not.toHaveBeenCalled();
  });

  it("re-enables the buttons and says why when a switch is refused", async () => {
    // A successful switch reloads, so only a refusal returns here. Leaving
    // busy set disabled both buttons for good, which read as broken rather
    // than unavailable. (#314 review)
    switchDesignMock.mockResolvedValue({ ok: false, reason: "storage refused it" });
    render(<AppearanceSettingsSection />);
    const next = screen.getByRole("button", { name: /new design/i });
    await userEvent.click(next);

    expect(screen.getByRole("alert").textContent).toContain("storage refused it");
    expect(next.hasAttribute("disabled")).toBe(false);
    // And it can be retried, rather than needing a manual reload.
    await userEvent.click(next);
    expect(switchDesignMock).toHaveBeenCalledTimes(2);
  });
});
