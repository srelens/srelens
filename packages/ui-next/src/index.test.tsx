import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// `vi.hoisted` because `vi.mock` is hoisted above every declaration in the
// file, and the tabsPersist factory reads these the moment the Window imports
// the module — a plain `const` is still in its temporal dead zone by then.
const { listContexts } = vi.hoisted(() => ({
  listContexts: vi.fn().mockResolvedValue({ contexts: [] }),
}));

vi.mock("@srelens/core", async (importOriginal) => {
  const real = await importOriginal<typeof import("@srelens/core")>();
  return { ...real, listContexts: (...a: unknown[]) => listContexts(...a) };
});

// Nothing here is testing persistence; the real module would write the test's
// tabs into settingsStorage and read the previous test's back.
vi.mock("./lib/tabsPersist", () => ({
  loadTabsState: () => null,
  scheduleSave: () => {},
  installFlushOnUnload: () => () => {},
  flushSave: () => {},
}));

// jsdom has no ResizeObserver; TabStrip's overflow Popover wants one.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import { NextApp } from "./index";
import { openTab, currentWorkspace } from "./lib/tabsStore";

describe("NextApp", () => {
  // jsdom keeps one window.location for the whole file, so a test that
  // navigates hands the next one a gallery instead of the window.
  beforeEach(() => {
    window.location.hash = "";
    listContexts.mockClear();
  });

  it("renders the window, with a tab strip and a home tab", async () => {
    render(<NextApp onExit={() => null} />);
    expect(await screen.findByRole("tablist")).toBeDefined();
    expect(screen.getByRole("tab", { name: /Control room/ })).toBeDefined();
  });

  it("the Placeholder's way back to classic is the onExit the app supplied", async () => {
    // Settings does not exist in this tree yet, so the Placeholder's button is
    // the only exit — it has to be wired to the app's switch, not to nothing.
    const onExit = vi.fn(() => null);
    render(<NextApp onExit={onExit} />);
    await screen.findByRole("tablist");
    await userEvent.click(screen.getByRole("button", { name: /open in classic/i }));
    expect(onExit).toHaveBeenCalledTimes(1);
  });

  it("shows why it could not leave, since there is no toast host here", async () => {
    // The Toaster lives in the classic tree, so a failure on the way out would
    // be invisible and the button would look inert. (#314 review)
    render(<NextApp onExit={() => "storage refused the preference"} />);
    await screen.findByRole("tablist");
    await userEvent.click(screen.getByRole("button", { name: /open in classic/i }));
    expect(screen.getByRole("alert").textContent).toContain("storage refused");
  });

  it("keeps the exit error on screen rather than below the fold", async () => {
    // `body` is `overflow: hidden` and `#root` is `height: 100%`, so an alert
    // rendered as a *sibling* of the Window's `h-full` root starts at the
    // bottom edge and is clipped: present in the DOM and the a11y tree, and
    // invisible — the silent failure #314 closed. jsdom has no layout, so the
    // structure that prevents it is what gets pinned here.
    render(<NextApp onExit={() => "storage refused the preference"} />);
    await screen.findByRole("tablist");
    await userEvent.click(screen.getByRole("button", { name: /open in classic/i }));

    const alert = screen.getByRole("alert");
    const container = alert.parentElement;
    expect(container?.className).toMatch(/\bflex\b/);
    expect(container?.className).toMatch(/\bflex-col\b/);
    expect(container?.className).toMatch(/\bh-full\b/);

    // The Window is boxed in a flexible sibling rather than sitting next to
    // the alert at full height, so the alert gets the room it needs.
    const host = alert.previousElementSibling;
    expect(host?.contains(screen.getByRole("tablist"))).toBe(true);
    expect(host?.className).toMatch(/\bflex-1\b/);
    expect(host?.className).toMatch(/\bmin-h-0\b/);
  });

  it("offers a way into the component gallery", async () => {
    // The gallery has been reachable at #gallery since #317, and nothing said
    // so: a review surface nobody can reach is one nobody reviews. (#318, #327)
    // Clicked rather than navigated to by setting the hash, because the
    // affordance is the whole point — asserting the route only would have gone
    // on passing after the way in was deleted.
    render(<NextApp onExit={() => null} />);
    await screen.findByRole("tablist");
    await userEvent.click(screen.getByRole("button", { name: /component gallery/i }));
    expect(await screen.findByRole("heading", { name: /design system/i })).toBeDefined();
    // The gallery replaces the window rather than rendering inside it. Asked
    // of the Placeholder's button rather than of a tablist, because the gallery
    // demonstrates the TabStrip and so has tablists — and tabs — of its own.
    expect(screen.queryByRole("button", { name: /open in classic/i })).toBeNull();
  });

  it("keeps the window mounted under the gallery, so the session survives the trip", async () => {
    // Returning `<Gallery />` *instead of* the Window unmounted it, and coming
    // back re-booted it: with session restore off the boot wrote a fresh
    // default state and every tab of the session was gone; with it on, only
    // whatever the 300ms debounce had already flushed came back.
    render(<NextApp onExit={() => null} />);
    await screen.findByRole("tablist");
    act(() => openTab("/k/pods"));
    const before = currentWorkspace().tabs.map((t) => t.id);
    expect(before).toHaveLength(2);

    window.location.hash = "#gallery";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    await screen.findByRole("heading", { name: /design system/i });

    window.location.hash = "";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    await screen.findByRole("tablist");
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      expect.stringContaining("Control room"),
      expect.stringContaining("Pods"),
    ]);
    expect(currentWorkspace().tabs.map((t) => t.id)).toEqual(before);
    // The proof that it was never re-booted: boot is what lists the contexts.
    expect(listContexts).toHaveBeenCalledTimes(1);
  });

  it("follows the hash after mount, not only on a fresh load", async () => {
    // Reading window.location.hash during render subscribes to nothing, so
    // navigating to #gallery left the window up and navigating away left the
    // gallery up, until a reload. (#317 review)
    render(<NextApp onExit={() => null} />);
    expect(await screen.findByRole("tablist")).toBeDefined();

    window.location.hash = "#gallery";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    expect(await screen.findByRole("heading", { name: /design system/i })).toBeDefined();

    window.location.hash = "";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    expect(await screen.findByRole("tablist")).toBeDefined();
  });

  it("takes the window's chrome down at the gallery, and puts it back after", async () => {
    // The gallery demos TabStrips of its own — even one labelled Open tabs —
    // so "no tablist anywhere" is not the claim. The claim is about the
    // window's: queried inside the window host with hidden elements included,
    // an unmounted strip is absent while a merely hidden one would still be
    // found. The bodies stay mounted either way; that is what keeps the
    // session alive across the trip.
    const view = render(<NextApp onExit={() => null} />);
    // The hidden wrapper around the Window — the host whose contents survive
    // the gallery trip. Structural on purpose: it is this file's subject.
    const windowHost = (view.container.firstElementChild as HTMLElement)
      .firstElementChild as HTMLElement;
    await within(windowHost).findByRole("tablist", { name: "Open tabs" });

    window.location.hash = "#gallery";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    await screen.findByRole("heading", { name: /design system/i });
    expect(windowHost.hidden).toBe(true);
    expect(within(windowHost).queryByRole("tablist", { hidden: true })).toBeNull();
    expect(windowHost.textContent).toContain("is not in the new design yet");

    window.location.hash = "";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    await within(windowHost).findByRole("tablist", { name: "Open tabs" });
  });

  it("hands onExit the route and the cluster it is leaving", async () => {
    // Classic reopens at the place the new design was on, so the way out has
    // to carry more than a route: without the cluster, "Open in classic" from
    // a pod list would land classic wherever it happened to be last. Boot's
    // Default workspace makes the one context active, so no seeding is needed.
    const onExit = vi.fn(() => null);
    listContexts.mockResolvedValue({
      contexts: [{ name: "prod", stableId: "prod", cluster: "prod", server: "", isCurrent: false }],
    });
    render(<NextApp onExit={onExit} />);
    await screen.findByRole("tablist");
    await userEvent.click(screen.getByRole("button", { name: /open in classic/i }));
    expect(onExit).toHaveBeenCalledWith("/", "prod");
  });
});
