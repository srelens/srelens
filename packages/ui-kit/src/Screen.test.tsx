import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Screen } from "./Screen";

describe("Screen", () => {
  it("renders the title as the page's level-one heading", () => {
    // The mock draws the title as a styled span. That loses the document
    // outline: a screen-reader user navigating by heading lands on nothing on
    // any page in the app. The look is the mock's; the semantics are the
    // classic PageHeader's. (#318)
    render(<Screen title="Pods">body</Screen>);
    expect(screen.getByRole("heading", { level: 1, name: "Pods" })).toBeDefined();
  });

  it("renders its children", () => {
    render(<Screen title="Pods">the table</Screen>);
    expect(screen.getByText("the table")).toBeDefined();
  });

  it("renders the eyebrow and actions when given", () => {
    render(
      <Screen title="Pods" eyebrow="Workloads" actions={<button>Refresh</button>}>
        body
      </Screen>,
    );
    expect(screen.getByText("Workloads")).toBeDefined();
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDefined();
  });

  it("omits the eyebrow and actions elements entirely when not given", () => {
    // Empty flex children still take gap space and push the title off centre.
    const { container } = render(<Screen title="Pods">body</Screen>);
    expect(container.querySelector(".crumb")).toBeNull();
    expect(container.querySelector('[data-slot="screen-actions"]')).toBeNull();
  });

  it("renders the description under the toolbar, not inside it", () => {
    // The toolbar is a fixed-height strip; a paragraph does not fit in it. The
    // description belongs to the content, so it opens the body.
    const { container } = render(
      <Screen title="Pods" description="Everything scheduled in this namespace.">
        body
      </Screen>,
    );
    const toolbar = container.querySelector(".toolbar") as HTMLElement;
    const description = screen.getByText("Everything scheduled in this namespace.");
    expect(toolbar.contains(description)).toBe(false);
  });

  it("scrolls its body by default", () => {
    const { container } = render(<Screen title="Pods">body</Screen>);
    expect(container.querySelector(".scroll")).not.toBeNull();
  });

  it("lets the body fill instead of scroll when asked", () => {
    // A screen whose content scrolls internally — a table with a sticky header,
    // a terminal — must not sit inside a second scroller.
    const { container } = render(
      <Screen title="Pods" fill>
        body
      </Screen>,
    );
    expect(container.querySelector(".scroll")).toBeNull();
  });

  it("forwards className onto the screen", () => {
    const { container } = render(
      <Screen title="Pods" className="extra">
        body
      </Screen>,
    );
    expect(container.querySelector(".extra")).not.toBeNull();
  });
  it("treats conditional slots that resolved to false as absent", () => {
    // (#325 review)
    const { container } = render(
      <Screen title="Pods" eyebrow={false} description={false} actions={false}>
        body
      </Screen>,
    );
    expect(container.querySelector(".crumb")).toBeNull();
    expect(container.querySelector('[data-slot="screen-actions"]')).toBeNull();
    expect(container.querySelector("p")).toBeNull();
  });
  it("treats an empty list of actions as no actions", () => {
    // `actions={items.map(...)}` over an empty list. (#325 review)
    const { container } = render(
      <Screen title="Pods" actions={[]}>
        body
      </Screen>,
    );
    expect(container.querySelector('[data-slot="screen-actions"]')).toBeNull();
  });
  it("treats an empty fragment of actions as no actions", () => {
    // (#325 review)
    const { container } = render(
      <Screen title="Pods" actions={<></>}>
        body
      </Screen>,
    );
    expect(container.querySelector('[data-slot="screen-actions"]')).toBeNull();
  });
});
