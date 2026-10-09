import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defaultState } from "../../lib/tabs";
import { activeRoute, setState } from "../../lib/tabsStore";
import { WhatsNew } from "./WhatsNew";

const core = vi.hoisted(() => ({ isTauri: vi.fn(() => true), checkForUpdate: vi.fn(), appVersion: vi.fn() }));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), ...core }));

const UPDATE = { version: "0.17.0", currentVersion: "0.16.0", notes: "## Highlights", external: false, elevates: false };

beforeEach(() => {
  core.isTauri.mockReset().mockReturnValue(true);
  core.checkForUpdate.mockReset();
  core.appVersion.mockReset().mockResolvedValue("0.16.0");
  setState(defaultState([]));
});

describe("WhatsNew", () => {
  it("says an update is waiting, and opens its notes", async () => {
    core.checkForUpdate.mockResolvedValue(UPDATE);
    render(<WhatsNew />);
    expect(screen.getByRole("heading", { name: "What's new", level: 2 })).toBeTruthy();
    expect(await screen.findByText("srelens 0.17.0 is available")).toBeTruthy();
    expect(screen.getByText("You are on 0.16.0.")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "See what's new in 0.17.0" }));
    expect(activeRoute()).toBe("/notes");
  });

  it("names the version when it is current, and links to the release notes", async () => {
    core.checkForUpdate.mockResolvedValue(null);
    render(<WhatsNew />);
    expect(await screen.findByText("srelens 0.16.0 is up to date")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Release notes" }));
    expect(activeRoute()).toBe("/notes");
  });

  it("says a check that failed, and never that srelens is up to date", async () => {
    core.checkForUpdate.mockRejectedValue(new Error("error sending request for url (https://releases.srelens.dev/latest.json)"));
    render(<WhatsNew />);
    expect(await screen.findByText("Could not check for updates")).toBeTruthy();
    expect(screen.queryByText(/is up to date/)).toBeNull();
  });

  it("is not drawn on the web host, where the server owns updates", () => {
    core.isTauri.mockReturnValue(false);
    const { container } = render(<WhatsNew />);
    expect(container.innerHTML).toBe("");
    expect(core.checkForUpdate).not.toHaveBeenCalled();
  });
});
