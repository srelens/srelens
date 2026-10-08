import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const openExternal = vi.hoisted(() => vi.fn(async (_url: string) => {}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  openExternal,
}));

import { settingsStorage } from "@srelens/core";
import {
  NUDGE_AFTER_LAUNCHES,
  NUDGE_AFTER_MS,
  STAR_KEY,
  __resetStarForTests,
  getStarState,
  setShowStarButton,
  type StarState,
} from "../lib/starOnGitHub";
import { lockWorkspace, resetLock } from "./LockGate";
import { StarButton } from "./StarButton";

/** Start from a stored state, as a launch would. */
function seed(patch: Partial<StarState> = {}) {
  settingsStorage.setItem(STAR_KEY, JSON.stringify(patch));
  __resetStarForTests();
}

/** A state in which the nudge is due right now. */
const due: Partial<StarState> = {
  launches: NUDGE_AFTER_LAUNCHES,
  firstLaunchAt: Date.now() - NUDGE_AFTER_MS - 1000,
};

const button = () => screen.getByRole("button", { name: "Star srelens on GitHub" });
const nudge = () => screen.queryByRole("group", { name: "Enjoying srelens?" });

beforeEach(() => {
  openExternal.mockReset().mockResolvedValue(undefined);
  settingsStorage.removeItem(STAR_KEY);
  __resetStarForTests();
  resetLock();
});

describe("StarButton", () => {
  it("opens the srelens repository, and nothing else, in the reader's browser", async () => {
    render(<StarButton />);
    await userEvent.click(button());
    expect(openExternal).toHaveBeenCalledExactlyOnceWith("https://github.com/srelens/srelens");
  });

  it("says why a star is worth the click", () => {
    render(<StarButton />);
    expect(button().getAttribute("title")).toBe(
      "Star srelens on GitHub — it helps other people find the project",
    );
  });

  it("shows the star count when there is one, and only the word when there is not", () => {
    seed({ count: 1234 });
    const { unmount } = render(<StarButton />);
    expect(button().textContent).toBe("Star1.2k");
    unmount();
    seed({});
    render(<StarButton />);
    expect(button().textContent).toBe("Star");
  });

  it("keeps the same accessible name whatever the count", () => {
    seed({ count: 98_765 });
    render(<StarButton />);
    // A name that changed with the figure would be a different control to a
    // screen reader every day.
    expect(button()).toBeDefined();
  });

  it("is not drawn once the reader has turned it off, and comes back when turned on", () => {
    render(<StarButton />);
    act(() => setShowStarButton(false));
    expect(screen.queryByRole("button")).toBeNull();
    act(() => setShowStarButton(true));
    expect(button()).toBeDefined();
  });

  it("says so when the browser could not be opened", async () => {
    openExternal.mockRejectedValue(new Error("no handler for https"));
    render(<StarButton />);
    await userEvent.click(button());
    expect((await screen.findByRole("alert")).textContent).toContain("Could not open GitHub");
  });
});

describe("the one-time nudge", () => {
  it("does not ask on an install that has barely been used", () => {
    seed({ launches: 1, firstLaunchAt: Date.now() });
    render(<StarButton />);
    expect(nudge()).toBeNull();
  });

  it("asks once the app has been used enough, without taking focus", () => {
    seed(due);
    render(<StarButton />);
    expect(nudge()).not.toBeNull();
    expect(nudge()?.textContent).toContain("it helps other people find the project");
    expect(document.activeElement).toBe(document.body);
  });

  it("goes to GitHub from its own button and never asks again", async () => {
    seed(due);
    const { unmount } = render(<StarButton />);
    await userEvent.click(screen.getByRole("button", { name: "Star on GitHub" }));
    expect(openExternal).toHaveBeenCalledExactlyOnceWith("https://github.com/srelens/srelens");
    expect(nudge()).toBeNull();
    expect(getStarState().nudged).toBe(true);
    // The next launch.
    unmount();
    __resetStarForTests();
    render(<StarButton />);
    expect(nudge()).toBeNull();
  });

  it("takes Not now for an answer, for good, and opens nothing", async () => {
    seed(due);
    const { unmount } = render(<StarButton />);
    await userEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(openExternal).not.toHaveBeenCalled();
    expect(nudge()).toBeNull();
    unmount();
    __resetStarForTests();
    render(<StarButton />);
    expect(nudge()).toBeNull();
  });

  it("takes Escape as Not now", async () => {
    seed(due);
    render(<StarButton />);
    await userEvent.keyboard("{Escape}");
    expect(nudge()).toBeNull();
    expect(getStarState().nudged).toBe(true);
    expect(openExternal).not.toHaveBeenCalled();
  });

  it("is answered by the top-bar button too, since that is where it points", async () => {
    seed(due);
    render(<StarButton />);
    await userEvent.click(button());
    expect(nudge()).toBeNull();
    expect(getStarState().nudged).toBe(true);
  });

  it("does not ask over a locked workspace, and has not been used up by waiting", () => {
    seed(due);
    lockWorkspace();
    render(<StarButton />);
    expect(nudge()).toBeNull();
    expect(button()).toBeDefined();
    expect(getStarState().nudged).toBe(false);
  });

  it("listens for Escape only while it is asking", async () => {
    render(<StarButton />);
    await userEvent.keyboard("{Escape}");
    // An Escape meant for some other dialog must not answer a question that was never put.
    expect(getStarState().nudged).toBe(false);
  });
});
