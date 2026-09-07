import { describe, it, expect, vi } from "vitest";
import type { FormEvent } from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher";

// jsdom has no ResizeObserver and Radix's popper watches with one. Same stub
// the other Radix-backed suites carry, kept here rather than in the shared
// setup so the requirement stays visible.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

const WORKSPACES = [
  { id: "prod", name: "Production", clusters: 2, tabs: 11 },
  { id: "local", name: "Local & staging", clusters: 5, tabs: 2 },
  { id: "platform", name: "Platform", clusters: 4, tabs: 3 },
];

function setup(props: Partial<Parameters<typeof WorkspaceSwitcher>[0]> = {}) {
  const onSelect = vi.fn();
  const view = render(
    <WorkspaceSwitcher
      workspaces={WORKSPACES}
      activeId="local"
      onSelect={onSelect}
      {...props}
    />,
  );
  return { onSelect, ...view };
}

/**
 * The chip itself. Named by the active workspace, which the active row in the
 * open panel is called too — and the panel is portalled to the end of the
 * document, so the chip is the first of the two.
 */
const trigger = () => screen.getAllByRole("button")[0];

async function open(props: Partial<Parameters<typeof WorkspaceSwitcher>[0]> = {}) {
  const view = setup(props);
  await userEvent.click(trigger());
  await screen.findByRole("dialog");
  return view;
}

/** A row inside the open panel, as distinct from the chip. */
const row = (name: RegExp) =>
  screen.getAllByRole("button", { name }).find((el) => el.classList.contains("ns-row"))!;

/**
 * The chip at the left of the titlebar and the panel it opens.
 *
 * The mock's version was a `<span class="ws-chip">` inside a Popover trigger —
 * so the control that switches everything you are looking at could not be
 * reached by keyboard at all. That, and the fact that it read `useTabs()` and
 * called three store mutators directly, is what this is fixing. (#332)
 */
describe("WorkspaceSwitcher", () => {
  it("names the current workspace on the trigger", () => {
    setup();
    expect(trigger().textContent).toContain("Local & staging");
  });

  it("is a button, so it can be reached and pressed from the keyboard", async () => {
    // The mock's trigger was a span. Focusable by nothing, operable by nothing.
    setup();
    await userEvent.tab();
    expect(document.activeElement).toBe(trigger());
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("dialog")).toBeDefined();
  });

  it("does not submit a form it is standing in", async () => {
    const onSubmit = vi.fn((e: FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <WorkspaceSwitcher workspaces={WORKSPACES} activeId="local" onSelect={() => {}} />
      </form>,
    );
    await userEvent.click(trigger());
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("lists every workspace with what it holds", async () => {
    await open();
    const production = row(/Production/);
    expect(production.textContent).toContain("2 clusters");
    expect(production.textContent).toContain("11 tabs");
  });

  it("counts one cluster and one tab in the singular", async () => {
    // "1 clusters · 1 tabs" is the kind of thing that survives a mock and
    // embarrasses a product.
    await open({ workspaces: [{ id: "a", name: "Solo", clusters: 1, tabs: 1 }], activeId: "a" });
    const solo = row(/Solo/);
    expect(solo.textContent).toContain("1 cluster ");
    expect(solo.textContent).not.toContain("1 clusters");
    expect(solo.textContent).toContain("1 tab");
    expect(solo.textContent).not.toContain("1 tabs");
  });

  it("says which one you are in, not only in colour", async () => {
    await open();
    expect(row(/Local & staging/).getAttribute("aria-current")).toBe("true");
    expect(row(/Production/).getAttribute("aria-current")).toBeNull();
  });

  it("reports the workspace that was chosen, and closes", async () => {
    const { onSelect } = await open();
    await userEvent.click(row(/Production/));
    expect(onSelect).toHaveBeenCalledWith("prod");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("offers no remove control when there is only one workspace", async () => {
    // Removing the last one leaves nothing to be in.
    const onRemove = vi.fn();
    await open({ workspaces: [{ id: "a", name: "Solo", clusters: 1, tabs: 1 }], activeId: "a", onRemove });
    expect(screen.queryByRole("button", { name: /Remove/ })).toBeNull();
  });

  it("offers no remove control when the caller cannot remove", async () => {
    await open();
    expect(screen.queryByRole("button", { name: /Remove/ })).toBeNull();
  });

  it("says what removing a workspace would cost, in the control's own name", async () => {
    // One click, no undo, and eleven tabs inside it. The name is the only
    // warning a screen-reader user gets before pressing it. (#332)
    const onRemove = vi.fn();
    await open({ onRemove });
    const remove = screen.getByRole("button", { name: /Remove Production/ });
    expect(remove.getAttribute("aria-label")).toContain("2 clusters");
    expect(remove.getAttribute("aria-label")).toContain("11 tabs");
    await userEvent.click(remove);
    expect(onRemove).toHaveBeenCalledWith("prod");
  });

  it("does not switch workspace when the remove beside it is pressed", async () => {
    const onRemove = vi.fn();
    const { onSelect } = await open({ onRemove });
    await userEvent.click(screen.getByRole("button", { name: /Remove Production/ }));
    expect(onRemove).toHaveBeenCalledWith("prod");
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("offers a way to make a new workspace, and closes after", async () => {
    const onCreate = vi.fn();
    await open({ onCreate });
    await userEvent.click(screen.getByRole("button", { name: "New workspace" }));
    expect(onCreate).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("omits the footer entirely when there is no creating to do", async () => {
    // An empty ruled band under the list is a visible artefact.
    const { container } = await open();
    expect(container.ownerDocument.querySelector('[data-slot="workspace-new"]')).toBeNull();
  });

  it("names the panel", async () => {
    await open();
    expect(screen.getByRole("dialog", { name: "Workspaces" })).toBeDefined();
  });

  it("closes on Escape", async () => {
    await open();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("says so when there are no workspaces at all", async () => {
    // In the panel as well as on the chip: the chip falls back to it because
    // there is no name to show, and the panel says it because an empty list
    // that renders nothing looks like a panel that failed to load.
    await open({ workspaces: [], activeId: "", emptyLabel: "No workspaces" });
    const panel = screen.getByRole("dialog");
    expect(panel.textContent).toContain("No workspaces");
    expect(trigger().textContent).toContain("No workspaces");
  });
});
