import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sidebar } from "./Sidebar";

function nav() {
  return screen.getByRole("navigation", { name: "Resources" });
}

describe("Sidebar", () => {
  it("is a named landmark, so it can be jumped to", () => {
    render(
      <Sidebar label="Resources">
        <p>tree</p>
      </Sidebar>,
    );
    expect(nav()).toBeDefined();
  });

  it("shows the slots it was given", () => {
    render(
      <Sidebar label="Resources" header={<p>prod-eu</p>} footer={<p>agent is watching</p>}>
        <p>tree</p>
      </Sidebar>,
    );
    expect(screen.getByText("prod-eu")).toBeDefined();
    expect(screen.getByText("tree")).toBeDefined();
    expect(screen.getByText("agent is watching")).toBeDefined();
  });

  it("spends no band on a slot that turned out to be empty", () => {
    // `header={showHeader && <X />}` is how a caller makes a slot conditional,
    // and it hands over `false` — which renders nothing but would still take
    // its padding and its rule.
    const { container } = render(
      <Sidebar label="Resources" header={false} footer={[]}>
        <p>tree</p>
      </Sidebar>,
    );
    expect(container.querySelector('[data-slot="header"]')).toBeNull();
    expect(container.querySelector('[data-slot="footer"]')).toBeNull();
  });

  it("says the sidebar is empty rather than showing a blank column", () => {
    render(<Sidebar label="Resources" emptyTitle="No resources" emptyHint="Connect a cluster." />);
    expect(screen.getByText("No resources")).toBeDefined();
    expect(screen.getByText("Connect a cluster.")).toBeDefined();
  });
});

describe("Sidebar filter", () => {
  it("has a filter box only when the caller wants one", () => {
    render(
      <Sidebar label="Resources">
        <p>tree</p>
      </Sidebar>,
    );
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("names the filter box, since the placeholder is not a label", async () => {
    render(
      <Sidebar label="Resources" query="" onQueryChange={() => {}}>
        <p>tree</p>
      </Sidebar>,
    );
    expect(screen.getByRole("searchbox", { name: "Filter resources" })).toBeDefined();
  });

  it("takes the caller's words for the filter", () => {
    render(
      <Sidebar label="Resources" query="" onQueryChange={() => {}} queryLabel="Filter clusters">
        <p>tree</p>
      </Sidebar>,
    );
    expect(screen.getByRole("searchbox", { name: "Filter clusters" })).toBeDefined();
  });

  it("reports what was typed, and never filters anything itself", async () => {
    const onQueryChange = vi.fn();
    render(
      <Sidebar label="Resources" query="po" onQueryChange={onQueryChange}>
        <p>tree</p>
      </Sidebar>,
    );
    const box = screen.getByRole("searchbox", { name: "Filter resources" });
    expect((box as HTMLInputElement).value).toBe("po");
    await userEvent.type(box, "d");
    expect(onQueryChange).toHaveBeenCalledWith("pod");
    // The rows are the caller's; the sidebar is a frame around them.
    expect(screen.getByText("tree")).toBeDefined();
  });
});

describe("Sidebar back bar", () => {
  it("appears only when there is somewhere to go back to", () => {
    render(
      <Sidebar label="Resources">
        <p>tree</p>
      </Sidebar>,
    );
    expect(screen.queryByRole("button", { name: /All clusters/ })).toBeNull();
  });

  it("goes back, and says how many are waiting there", async () => {
    const onClick = vi.fn();
    render(
      <Sidebar label="Resources" back={{ label: "All clusters", count: 8, onClick }}>
        <p>tree</p>
      </Sidebar>,
    );
    const back = screen.getByRole("button", { name: /All clusters/ });
    expect(back.textContent).toContain("8");
    await userEvent.click(back);
    expect(onClick).toHaveBeenCalled();
  });
});

/**
 * The drag handle. The mock made it a `role="separator"` with a mousedown
 * listener and nothing else — a control announced to assistive technology that
 * only a pointer can work, which is the worst of both. Naming it and giving it
 * the arrow keys costs a dozen lines.
 */
describe("Sidebar resize handle", () => {
  it("is a named separator carrying its current width", () => {
    render(
      <Sidebar label="Resources" defaultWidth={240} minWidth={180} maxWidth={420}>
        <p>tree</p>
      </Sidebar>,
    );
    const handle = screen.getByRole("separator", { name: "Resize Resources" });
    expect(handle.getAttribute("aria-orientation")).toBe("vertical");
    expect(handle.getAttribute("aria-valuenow")).toBe("240");
    expect(handle.getAttribute("aria-valuemin")).toBe("180");
    expect(handle.getAttribute("aria-valuemax")).toBe("420");
    expect(nav().style.width).toBe("240px");
  });

  it("widens and narrows with the arrow keys", async () => {
    const onWidthChange = vi.fn();
    render(
      <Sidebar label="Resources" defaultWidth={240} onWidthChange={onWidthChange}>
        <p>tree</p>
      </Sidebar>,
    );
    screen.getByRole("separator", { name: "Resize Resources" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(nav().style.width).toBe("256px");
    expect(onWidthChange).toHaveBeenLastCalledWith(256);
    await userEvent.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(nav().style.width).toBe("224px");
    expect(onWidthChange).toHaveBeenLastCalledWith(224);
  });

  it("goes to the extremes with Home and End", async () => {
    render(
      <Sidebar label="Resources" defaultWidth={240} minWidth={180} maxWidth={420}>
        <p>tree</p>
      </Sidebar>,
    );
    screen.getByRole("separator", { name: "Resize Resources" }).focus();
    await userEvent.keyboard("{End}");
    expect(nav().style.width).toBe("420px");
    await userEvent.keyboard("{Home}");
    expect(nav().style.width).toBe("180px");
  });

  it("refuses to go past either end", async () => {
    render(
      <Sidebar label="Resources" defaultWidth={188} minWidth={180} maxWidth={200}>
        <p>tree</p>
      </Sidebar>,
    );
    screen.getByRole("separator", { name: "Resize Resources" }).focus();
    await userEvent.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(nav().style.width).toBe("180px");
    await userEvent.keyboard("{ArrowRight}{ArrowRight}{ArrowRight}");
    expect(nav().style.width).toBe("200px");
  });

  it("is in the tab order", async () => {
    render(
      <Sidebar label="Resources">
        <button type="button">row</button>
      </Sidebar>,
    );
    screen.getByRole("button", { name: "row" }).focus();
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("separator", { name: "Resize Resources" }));
  });

  it("drags from where the pointer went down, not from the window's edge", () => {
    // The mock computed the width as `clientX - 46`, baking in the width of the
    // rail beside it: drop the sidebar anywhere else and the drag jumps.
    const onWidthChange = vi.fn();
    render(
      <Sidebar label="Resources" defaultWidth={240} onWidthChange={onWidthChange}>
        <p>tree</p>
      </Sidebar>,
    );
    const handle = screen.getByRole("separator", { name: "Resize Resources" });
    fireEvent.mouseDown(handle, { clientX: 600 });
    fireEvent.mouseMove(window, { clientX: 640 });
    expect(nav().style.width).toBe("280px");
    // Reported once it settles, not forty times on the way there.
    expect(onWidthChange).not.toHaveBeenCalled();
    fireEvent.mouseUp(window);
    expect(onWidthChange).toHaveBeenCalledTimes(1);
    expect(onWidthChange).toHaveBeenCalledWith(280);
  });

  it("stops listening once the drag is over", () => {
    render(
      <Sidebar label="Resources" defaultWidth={240}>
        <p>tree</p>
      </Sidebar>,
    );
    const handle = screen.getByRole("separator", { name: "Resize Resources" });
    fireEvent.mouseDown(handle, { clientX: 600 });
    fireEvent.mouseUp(window);
    fireEvent.mouseMove(window, { clientX: 900 });
    expect(nav().style.width).toBe("240px");
  });
});

describe("Sidebar inside a form", () => {
  it("never submits the form it is standing in", async () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const { container } = render(
      <form onSubmit={onSubmit}>
        <Sidebar label="Resources" back={{ label: "All clusters", onClick: () => {} }}>
          <p>tree</p>
        </Sidebar>
      </form>,
    );
    await userEvent.click(screen.getByRole("button", { name: /All clusters/ }));
    expect(onSubmit).not.toHaveBeenCalled();
    for (const button of container.querySelectorAll("button")) {
      expect(button.getAttribute("type")).toBe("button");
    }
  });
});
