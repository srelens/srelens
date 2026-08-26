import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import { Rail } from "./Rail";
import { activeCluster, currentWorkspace, setState } from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { resetView, setLink } from "../lib/workspace";
import { defaultMark, getMark, loadMarks, setMark } from "../lib/marks";
import { probeCluster, resetProbes } from "../lib/probe";

// jsdom has no ResizeObserver and Radix's popper — which the kit's Tooltip, and
// so every rail button, sits on — watches its trigger with one. The same stub
// Chrome.test.tsx carries, kept per file so the requirement stays visible.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

const ctx = (name: string): ClusterContext => ({
  name,
  stableId: name,
  cluster: name,
  server: `https://${name}.example`,
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
});

const CONTEXTS = [ctx("prod-eu"), ctx("staging")];

beforeEach(() => {
  setState(defaultState(CONTEXTS));
  resetView();
  resetProbes();
  // Marks persist through `settingsStorage`; the shared setup clears
  // localStorage between tests, so this puts the in-memory copy back with it.
  loadMarks();
  vi.clearAllMocks();
});

function setup(props: Partial<Parameters<typeof Rail>[0]> = {}) {
  const onConnect = vi.fn();
  const view = render(<Rail contexts={CONTEXTS} onConnect={onConnect} {...props} />);
  return { onConnect, ...view };
}

/** Right-click a mark and wait for the menu that names it. */
async function openMenu(cluster: string) {
  fireEvent.contextMenu(screen.getByRole("button", { name: cluster }));
  return screen.findByRole("menu", { name: `${cluster} actions` });
}

async function pick(cluster: string, item: string) {
  const menu = await openMenu(cluster);
  fireEvent.click(within(menu).getByRole("menuitem", { name: item }));
}

/** The Customise dialog, named by the cluster it is about. */
const dialog = (cluster: string) => screen.getByRole("dialog", { name: `Customise ${cluster}` });

describe("Rail", () => {
  it("lists the workspace's clusters and selects the one clicked", async () => {
    setup();
    expect(screen.getByRole("button", { name: "prod-eu" })).toBeDefined();
    expect(screen.getByRole("button", { name: "staging" })).toBeDefined();

    await userEvent.click(screen.getByRole("button", { name: "staging" }));
    expect(activeCluster()).toBe("staging");
  });

  it("says why a cluster is out of reach in its name, classified rather than quoted", () => {
    setLink("prod-eu", "error", "dial tcp 10.1.2.3:6443: connect: connection refused");
    setup();
    expect(screen.getByRole("button", { name: "prod-eu, Can't reach the cluster" })).toBeDefined();
  });

  it("never reads a cluster's raw refusal out as the name of a button", () => {
    // The rail is 46px wide and this string is not drawn — it joins the mark's
    // accessible name. What was reaching a screen reader was three hundred
    // characters of `Status { metadata: Some(ListMeta { … })` announced as the
    // name of a button; two words say the same thing. (The original is not
    // lost — the overview's Fleet row for this cluster has it.)
    setLink(
      "prod-eu",
      "error",
      'Error: handler error: ApiError: Unauthorized: Unauthorized (Status { code: Some(401), ' +
        "metadata: Some(ListMeta { continue_: None, resource_version: None }) })",
    );
    setup();
    expect(screen.getByRole("button", { name: "prod-eu, Not authorized" })).toBeDefined();
    expect(screen.queryByRole("button", { name: /ListMeta/ })).toBeNull();
  });

  it("still says something for a failure that arrived with no message", () => {
    setLink("prod-eu", "error", "");
    setup();
    expect(screen.getByRole("button", { name: "prod-eu, Unreachable" })).toBeDefined();
  });

  it("opens a context menu on the menu gesture, not a drawer", async () => {
    setup();
    const menu = await openMenu("prod-eu");
    const items = within(menu)
      .getAllByRole("menuitem")
      .map((item) => item.getAttribute("aria-label"));
    expect(items).toEqual(["Open prod-eu", "Customise…", "Connection details", "Remove from workspace"]);
    expect(screen.queryByRole("complementary", { name: "Details" })).toBeNull();
  });

  it("makes the cluster active from the menu", async () => {
    setup();
    await pick("staging", "Open staging");
    expect(activeCluster()).toBe("staging");
  });

  it("opens the Connections tab", async () => {
    setup();
    await pick("prod-eu", "Connection details");
    expect(currentWorkspace().tabs.map((t) => t.route)).toContain("/connections");
  });

  it("removes the cluster from the workspace", async () => {
    setup();
    await pick("prod-eu", "Remove from workspace");
    expect(currentWorkspace().clusters).toEqual(["staging"]);
  });

  it("asks the app to connect a cluster", async () => {
    const { onConnect } = setup();
    await userEvent.click(screen.getByRole("button", { name: "Connect a cluster" }));
    expect(onConnect).toHaveBeenCalled();
  });

  it("draws a customised mark, and still names the button after the context", () => {
    setMark("prod-eu", { ...defaultMark("prod-eu"), name: "Production EU", short: "PX" });
    setup();
    // The rail is a list of the workspace's contexts: what a button is called
    // is the context's business, and what the square says is the mark's.
    expect(screen.getByRole("button", { name: "prod-eu" })).toBeDefined();
    expect(screen.getByText("PX")).toBeDefined();
  });

  it("draws a stored image mark", () => {
    const png = "data:image/png;base64,iVBORw0KGgo=";
    setMark("prod-eu", { ...defaultMark("prod-eu"), mark: "image", imageSrc: png });
    const { container } = setup();
    expect(container.querySelector("img")?.getAttribute("src")).toBe(png);
  });

  // The hint's contents, not the subscription: `probeCluster` always moves the
  // link through `connecting` on its way, so the workspace store emits on every
  // probe and this would pass on a rail subscribed to nothing at all. That the
  // probe store is readable on its own is `probe.test.ts`'s `useInfos` test.
  it("puts the version and the server in the hint once the probe lands", async () => {
    setup();
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true, version: "v1.31.0" });
    await act(async () => {
      await probeCluster(CONTEXTS[0], connect as never);
    });
    // Focus rather than hover: Radix opens the tooltip on focus with no delay.
    await act(async () => screen.getByRole("button", { name: "prod-eu" }).focus());
    const tip = await screen.findAllByText("prod-eu — v1.31.0 · https://prod-eu.example");
    expect(tip.length).toBeGreaterThan(0);
  });

  it("drops a dialog whose context has gone, and does not reopen it when it returns", async () => {
    const { rerender } = setup();
    await pick("prod-eu", "Customise…");
    await waitFor(() => expect(dialog("prod-eu")).toBeDefined());

    rerender(<Rail contexts={[CONTEXTS[1]]} onConnect={() => {}} />);
    expect(screen.queryByRole("dialog")).toBeNull();

    // A kubeconfig that flickers must not reopen a panel nobody asked for.
    rerender(<Rail contexts={CONTEXTS} onConnect={() => {}} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

/**
 * The mark editor: a compact centred dialog rather than a full-height drawer,
 * with the whole palette the design draws and every kind of mark it offers.
 */
describe("Rail's Customise dialog", () => {
  it("edits the mark in a dialog named after the cluster", async () => {
    setup();
    await pick("prod-eu", "Customise…");
    await waitFor(() => expect(dialog("prod-eu")).toBeDefined());
    expect((within(dialog("prod-eu")).getByLabelText(/Display name/) as HTMLInputElement).value).toBe("prod-eu");
  });

  it("offers the eleven colours of the design, each named", async () => {
    setup();
    await pick("prod-eu", "Customise…");
    const swatches = within(await screen.findByRole("radiogroup", { name: "Colour" })).getAllByRole("radio");
    expect(swatches).toHaveLength(11);
    // A hex read aloud names nothing, and the palette is what a colour-blind
    // reader has instead of the swatch.
    expect(swatches.every((s) => (s.getAttribute("aria-label") ?? "").trim().length > 0)).toBe(true);
  });

  it("offers a symbol mark alongside text and image", async () => {
    setup();
    await pick("prod-eu", "Customise…");
    const marks = within(await screen.findByRole("radiogroup", { name: "Mark" })).getAllByRole("radio");
    expect(marks.map((m) => m.closest("label")?.textContent)).toEqual(["Text", "Symbol", "Image"]);
  });

  it("keeps the mark and closes on Done", async () => {
    setup();
    await pick("prod-eu", "Customise…");
    const panel = await screen.findByRole("dialog", { name: "Customise prod-eu" });
    fireEvent.change(within(panel).getByLabelText(/Display name/), { target: { value: "Production EU" } });
    fireEvent.click(within(panel).getByRole("button", { name: "Done" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(getMark("prod-eu", "prod-eu").name).toBe("Production EU");
  });
});

describe("Rail draws a symbol mark", () => {
  it("draws the stored glyph", () => {
    setMark("prod-eu", { ...defaultMark("prod-eu"), mark: "icon", icon: "server", withText: false });
    const { container } = setup();
    expect(container.querySelector('[data-slot="chip-mark"] svg')).not.toBeNull();
  });

  it("falls back to the initials when the glyph is not one this build knows", () => {
    // A mark this build cannot read degrades to no mark rather than to a
    // broken one: an empty coloured square says less than the initials do.
    setMark("prod-eu", { ...defaultMark("prod-eu"), mark: "icon", icon: "nonesuch", withText: false });
    const { container } = setup();
    expect(container.querySelector('[data-slot="chip-mark"] svg')).toBeNull();
    expect(screen.getByText("PE")).toBeDefined();
  });
});
