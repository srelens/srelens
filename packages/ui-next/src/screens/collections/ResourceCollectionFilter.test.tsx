import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage, type Collection } from "@srelens/core";

(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
Element.prototype.scrollIntoView ??= () => {};

const core = vi.hoisted(() => ({ resolveResourceCollections: vi.fn(), validateCollection: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
const openSettings = vi.hoisted(() => vi.fn());
vi.mock("../../lib/settingsRequest", () => ({ openSettings }));

import { COLLECTIONS_KEY, __resetCollectionsForTests, getCollections } from "../../lib/collections";
import type { ResourceCollectionFilter, ResourceFilterState } from "../../lib/resourceCollections";
import { ResourceCollectionNotice, ResourceCollectionPicker } from "./ResourceCollectionFilter";

const RELEASE: Collection = { name: "release", type: "resource", kinds: ["apps/Deployment"], members: ["payment-service"] };
const CANARY: Collection = { name: "canary", type: "resource", kinds: ["apps/Deployment"], patterns: ["*-canary"] };

function fakeFilter(over: Partial<ResourceCollectionFilter> = {}): ResourceCollectionFilter {
  return {
    offered: [RELEASE, CANARY],
    chosen: [],
    choose: vi.fn(),
    state: { status: "off" },
    apply: (rows) => rows,
    ...over,
  };
}

function picker(filter: ResourceCollectionFilter, ticked: string[] = []) {
  return render(
    <ResourceCollectionPicker
      filter={filter}
      context="kind-dev"
      kind={{ kind: "Deployment" }}
      noun="deployments"
      namespaces={["dev"]}
      ticked={ticked}
    />,
  );
}

const trigger = () => screen.getByRole("combobox", { name: "Collections" });

beforeEach(() => {
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  core.validateCollection.mockReset().mockResolvedValue([]);
  core.resolveResourceCollections.mockReset().mockResolvedValue({ keys: [], collections: [], partial: false });
  openSettings.mockReset();
});

describe("ResourceCollectionPicker", () => {
  it("offers the collections that cover the kind, and chooses them", async () => {
    const filter = fakeFilter();
    picker(filter);
    expect(trigger().textContent).toBe("Collections");
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "canary" }));
    expect(filter.choose).toHaveBeenCalledWith(["canary"]);
  });

  it("names what is chosen on the trigger", () => {
    picker(fakeFilter({ chosen: [RELEASE] }));
    expect(trigger().textContent).toBe("release");
  });

  it("says how to make one when none covers the kind", async () => {
    picker(fakeFilter({ offered: [] }));
    await userEvent.click(trigger());
    expect(screen.getByText("No collections cover deployments yet. Tick rows, then Save as collection…")).toBeDefined();
  });

  /** #885: Save as collection… pre-fills the ticked rows, and the kind defaults to the list's. */
  it("saves the ticked rows as a collection of this kind, by name, and narrows the list to it", async () => {
    const filter = fakeFilter();
    picker(filter, ["payment-service", "order-service"]);
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("button", { name: "Save as collection…" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New resource collection")).toBeDefined();
    expect((within(dialog).getByLabelText("Members") as HTMLTextAreaElement).value).toBe("payment-service\norder-service");
    expect((within(dialog).getByRole("textbox", { name: "Kinds" }) as HTMLInputElement).value).toBe("Deployment");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Name" }), "release-oct");
    await waitFor(() =>
      expect(core.validateCollection).toHaveBeenLastCalledWith(
        { name: "release-oct", type: "resource", kinds: ["Deployment"], members: ["payment-service", "order-service"] },
        [],
      ),
    );
    // Previewed on this list's kind and namespaces.
    await waitFor(() =>
      expect(core.resolveResourceCollections).toHaveBeenCalledWith(
        "kind-dev",
        { kind: "Deployment" },
        ["dev"],
        [expect.objectContaining({ type: "resource" })],
      ),
    );
    await userEvent.click(within(dialog).getByRole("button", { name: "Save collection" }));
    expect(getCollections()).toEqual([
      { name: "release-oct", type: "resource", kinds: ["Deployment"], members: ["payment-service", "order-service"] },
    ]);
    expect(filter.choose).toHaveBeenCalledWith(["release-oct"]);
  });

  it("asks for ticked rows before it can save, and opens Settings from Manage…", async () => {
    picker(fakeFilter());
    await userEvent.click(trigger());
    expect(screen.getByRole("button", { name: "Save as collection…" }).hasAttribute("disabled")).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Manage collections…" }));
    expect(openSettings).toHaveBeenCalledExactlyOnceWith("collections");
  });
});

describe("ResourceCollectionNotice", () => {
  const notice = (state: ResourceFilterState, chosen: Collection[] = [RELEASE]) => {
    const filter = fakeFilter({ state, chosen });
    render(<ResourceCollectionNotice filter={filter} noun="deployments" context="kind-dev" />);
    return filter;
  };

  it("says nothing while no collection is chosen", () => {
    const { container } = render(<ResourceCollectionNotice filter={fakeFilter()} noun="deployments" context="kind-dev" />);
    expect(container.textContent).toBe("");
  });

  it("says it is working the collection out", () => {
    notice({ status: "resolving" });
    expect(screen.getByText(/Working out which deployments release covers on kind-dev/)).toBeDefined();
  });

  /** #885: zero matches is a clear empty state, not an error. */
  it("says plainly that nothing matched in the current scope, and offers every row back", async () => {
    const filter = notice({ status: "ready", keys: new Set(), resolution: { keys: [], collections: [], partial: false } });
    expect(screen.getByText("No deployments match release in the current scope")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Show all deployments" }));
    expect(filter.choose).toHaveBeenCalledWith([]);
  });

  it("says when some namespaces could not be listed, and when it failed", () => {
    notice({ status: "ready", keys: new Set(["dev/a"]), resolution: { keys: ["dev/a"], collections: [], partial: true } });
    expect(screen.getByText(/Some namespaces could not be listed on kind-dev/)).toBeDefined();
  });

  it("says why when it could not be worked out", () => {
    notice({ status: "failed", error: "deployments.apps is forbidden" });
    expect(screen.getByText("Collection release could not be worked out")).toBeDefined();
    expect(screen.getByText("deployments.apps is forbidden")).toBeDefined();
  });
});
