import { describe, it, expect } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Section } from "./Section";
import { SideRail } from "./SideRail";

/**
 * The layout almost every screen in the design is: one main region beside a
 * fixed-width rail. Four screens want it at four different widths — events
 * 250, custom resources 264, cluster overview 286, logs 272 — and the resource
 * list hand-rolled it once with a comment saying a single call site did not
 * justify inventing a component. It is now four.
 */
describe("SideRail", () => {
  it("names its rail region and draws it at the width asked for", () => {
    render(
      <SideRail head="By reason" rail={<p>rail body</p>} width={250}>
        main
      </SideRail>,
    );
    const aside = screen.getByRole("complementary", { name: "By reason" });
    // The width is the caller's, per screen, and arrives as an inline style —
    // there is no `w-` utility for four one-off numbers and no CSS width on
    // the class, which would only fight the prop.
    expect(aside.style.width).toBe("250px");
    expect(within(aside).getByText("rail body")).toBeDefined();
  });

  it("lets the main region shrink, so a wide table scrolls inside itself", () => {
    // The one property that has to carry over from the hand-rolled version.
    // Without it a flex item refuses to shrink below its content, and a table
    // with fifteen columns pushes the rail off the window instead of
    // scrolling inside its own box.
    const { container } = render(
      <SideRail head="h" rail="r" width={250}>
        main
      </SideRail>,
    );
    const main = container.querySelector("[data-slot='rail-main']");
    expect(main?.className).toContain("min-w-0");
    expect(main?.textContent).toBe("main");
  });

  it("offers no way to resize it", () => {
    // The point of the component. The design is explicit that only the detail
    // inspector and the left sidebar resize; every other rail is fixed. This
    // fails the moment someone adds a grip "for consistency" — and with the
    // grip would come a measured clamp, a ResizeObserver and a stored width,
    // three mechanisms answering a question this rail does not ask.
    render(
      <SideRail head="h" rail="r" width={250}>
        main
      </SideRail>,
    );
    expect(screen.queryByRole("separator")).toBeNull();
  });

  it("keeps the rail's sections direct siblings, so the hairlines are drawn", () => {
    // A rail's content is a run of `Section`s, and `.section + .section` is
    // what rules between them. Wrapping each child in anything at all breaks
    // that adjacency and the run reads as one undivided block.
    const { container } = render(
      <SideRail head="Definition" rail={<><Section title="A">a</Section><Section title="B">b</Section></>} width={264}>
        main
      </SideRail>,
    );
    const body = container.querySelector("[data-slot='rail-body']");
    const kids = Array.from(body?.children ?? []);
    expect(kids.length).toBe(2);
    expect(kids.every((el) => el.tagName === "SECTION" && el.className.includes("section"))).toBe(true);
  });
});

describe("the rail's stylesheet", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("styles the rail in the components layer, so a utility still wins", () => {
    expect(components).toContain("  .side-rail {");
  });

  it("names no width of its own", () => {
    // Four screens, four widths, one class. A width here would be a fifth
    // answer that silently disagrees with every call site.
    const rule = components.slice(components.indexOf("  .side-rail {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("border-left");
    expect(body).not.toMatch(/[^-]width:/);
  });
});

/**
 * The design heads the MAIN region too on the cluster overview —
 * `prod-eu · v1.31.4`, level with the rail's own `At a glance`.
 */
describe("SideRail's main head", () => {
  it("draws the main region's head in the same strip the rail's wears", () => {
    const { container } = render(
      <SideRail head="At a glance" mainHead="prod-eu · v1.31.4" rail="r" width={286}>
        main
      </SideRail>,
    );
    const main = container.querySelector("[data-slot='rail-main']");
    const head = main?.querySelector("[data-slot='main-head']");
    expect(head?.textContent).toBe("prod-eu · v1.31.4");
    // `.pane-head`, not a class of its own: it is the same small-caps bar, and
    // a second copy of the recipe is a second thing to keep in step.
    expect(head?.className).toContain("pane-head");
  });

  it("draws nothing at all when the screen has no head for its main region", () => {
    // Most screens put a filter bar or a table straight under the toolbar. An
    // empty strip is still a strip: it holds height and draws a rule.
    const { container } = render(
      <SideRail head="By reason" rail="r" width={250}>
        main
      </SideRail>,
    );
    expect(container.querySelector("[data-slot='main-head']")).toBeNull();
  });

  it("is not a second landmark", () => {
    // The rail is `complementary` because it is material a reader may want to
    // skip. The main region is where the reader already is.
    render(
      <SideRail head="At a glance" mainHead="prod-eu" rail="r" width={286}>
        main
      </SideRail>,
    );
    expect(screen.getAllByRole("complementary")).toHaveLength(1);
  });
});
