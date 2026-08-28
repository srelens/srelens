import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { KV } from "./KV";
import { Section } from "./Section";

/**
 * New with the detail pane's design mock: a run of sections divided by
 * hairline rules, which `Panel` cannot draw because it is a card. (#331)
 */
describe("Section", () => {
  it("names the block with a heading", () => {
    // The outline is how a screen reader finds Conditions, Labels and
    // Annotations inside the peek; a styled div drops all three out of it.
    render(<Section title="Conditions">rows</Section>);
    expect(screen.getByRole("heading", { level: 3, name: "Conditions" })).toBeDefined();
  });

  it("renders its content", () => {
    render(<Section title="Labels">app=web</Section>);
    expect(screen.getByText("app=web")).toBeDefined();
  });

  it("takes no heading at all when the caller has none", () => {
    // The mock puts no heading over the first fact list, and an empty heading
    // line is a visible gap rather than a no-op.
    const { container } = render(<Section>rows</Section>);
    expect(container.querySelector("h3")).toBeNull();
    expect(screen.getByText("rows")).toBeDefined();
  });

  it("is flat, not a card", () => {
    // The whole reason it exists beside Panel: no border, no lifted surface.
    const { container } = render(<Section title="Labels">rows</Section>);
    const root = container.querySelector("section");
    expect(root?.className).toContain("section");
    expect(root?.className).not.toContain("card");
  });

  it("forwards className onto the section", () => {
    const { container } = render(
      <Section title="Labels" className="extra">
        rows
      </Section>,
    );
    expect(container.querySelector("section.section.extra")).not.toBeNull();
  });
});

/**
 * The disclosure the reader's own request asks for: "first open should keep
 * everything collapsed, and from for next one remember what all was
 * uncollapsed". Remembering is the app's — see `ui-next/src/lib/sectionFolds`
 * — and this is the half the kit owns: a heading that can be a control, a
 * state it is told, and a toggle it reports.
 */
describe("a section that discloses", () => {
  it("leaves the heading a plain heading when the caller offers no toggle", () => {
    // Every call site that has one today keeps exactly what it had: a heading,
    // its content, and no control at all.
    render(<Section title="Conditions">rows</Section>);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("rows")).toBeDefined();
  });

  it("makes the heading a button that names what it opens", () => {
    // The accessible name is the block's own name, which is what the reader
    // is choosing to see. `AnnotationsToggle` counts entries instead because
    // it sits over secret data and must not name a key; a section heading is
    // already on the page whether it is open or shut.
    render(
      <Section title="Conditions" open={false} onToggle={() => {}}>
        rows
      </Section>,
    );
    const toggle = screen.getByRole("button", { name: "Conditions" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("type")).toBe("button");
  });

  it("keeps the heading in the outline, so the control does not cost the block its name", () => {
    render(
      <Section title="Conditions" open={false} onToggle={() => {}}>
        rows
      </Section>,
    );
    expect(screen.getByRole("heading", { level: 3, name: "Conditions" })).toBeDefined();
  });

  it("mounts none of its content while it is closed", () => {
    // Collapsed means ABSENT, not hidden. `AnnotationsToggle` exists for
    // exactly this reason and a section that merely hid its rows would put a
    // Secret's annotation back in the markup.
    const { container } = render(
      <Section title="Annotations" open={false} onToggle={() => {}}>
        <span>not-in-the-document</span>
      </Section>,
    );
    expect(container.innerHTML).not.toContain("not-in-the-document");
  });

  it("is still a section when it is closed, so its rule is still drawn", () => {
    // `.section + .section` is what divides a run. A closed section that
    // rendered nothing at all would take a hairline with it.
    const { container } = render(
      <>
        <Section title="Labels" open={false} onToggle={() => {}}>
          rows
        </Section>
        <Section title="Annotations" open={false} onToggle={() => {}}>
          rows
        </Section>
      </>,
    );
    expect([...container.children].every((el) => el.matches("section.section"))).toBe(true);
  });

  it("reports the state it is moving to, and keeps none of its own", async () => {
    // Controlled outright: the kit holds no app state and touches no storage,
    // so a section that flipped itself would disagree with the memory the
    // moment the app said otherwise. Same split as `Sidebar`/`ResizeHandle`.
    const onToggle = vi.fn();
    const { rerender } = render(
      <Section title="Conditions" open={false} onToggle={onToggle}>
        rows
      </Section>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Conditions" }));
    expect(onToggle).toHaveBeenCalledWith(true);
    expect(screen.queryByText("rows")).toBeNull();

    rerender(
      <Section title="Conditions" open onToggle={onToggle}>
        rows
      </Section>,
    );
    const toggle = screen.getByRole("button", { name: "Conditions" });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("rows")).toBeDefined();
    await userEvent.click(toggle);
    expect(onToggle).toHaveBeenLastCalledWith(false);
  });

  it("has nothing to disclose without a heading, so it stays open", () => {
    // The design heads the first block of a detail with nothing, so there is
    // no control to hang on it — and a pane that opens showing nothing at all
    // is hostile. An untitled section ignores both props rather than
    // vanishing.
    render(
      <Section open={false} onToggle={() => {}}>
        rows
      </Section>,
    );
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("rows")).toBeDefined();
  });

  it("keeps its content its own direct children, wrapped in nothing", () => {
    // A caller lays a section's content out — `ui-next`'s full tab grids its
    // fact rows three across inside one — so a panel element between that
    // caller's own layout and the rows would change what it is placing.
    const { container } = render(
      <Section title="Facts" open onToggle={() => {}}>
        <KV k="Status" v="Running" />
      </Section>,
    );
    expect(container.querySelector("section.section > .kv")).not.toBeNull();
  });
});

describe("a run of sections", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("rules between siblings and not around them, in the components layer", () => {
    // A rule per section would draw one above the first and below the last;
    // the mock divides, it does not box. Asserted on the stylesheet because
    // jsdom does no layout. The components layer matters: a utility applied in
    // the JSX has to be able to override this, and Tailwind's utilities layer
    // is declared after it.
    expect(components).toContain(".section + .section { border-top: 1px solid var(--rule); }");
    const rule = components.slice(components.indexOf("\n  .section {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).not.toContain("border:");
    expect(body).not.toContain("background:");
  });

  it("draws the disclosure's own line in the components layer too", () => {
    // Same reason as the rule above: a utility in the JSX has to be able to
    // override it, and Tailwind's utilities layer is declared after this one.
    const rule = components.slice(components.indexOf("\n  .section-toggle {"));
    expect(rule.slice(0, rule.indexOf("}"))).toContain("width: 100%");
    expect(components).toContain(".section-caret[data-open=\"true\"]");
  });
});

/**
 * The two shapes the design gives a run of sections. The detail body keeps
 * what it had; the cluster overview's bands are headed in small caps and run
 * their content to both edges of the surface (§7, §D's `padded: false`).
 */
describe("a section that heads a page", () => {
  it("stays inset and bold unless asked otherwise", () => {
    // Every call site that exists today is the detail pane's, and none of them
    // passes either prop: the defaults have to be what they already had.
    const { container } = render(<Section title="Conditions">rows</Section>);
    const root = container.querySelector("section");
    expect(root?.getAttribute("data-padded")).toBeNull();
    expect(container.querySelector("h3")?.className).toContain("font-semibold");
  });

  it("heads the band in small caps when the frame asks for it", () => {
    const { container } = render(
      <Section title="Capacity" smallCaps>
        rows
      </Section>,
    );
    expect(container.querySelector("h3")?.className).toContain("subhead-caps");
    // Still the block's name in the outline, and still the word itself in the
    // DOM — the uppercase is CSS, so a screen reader hears "Capacity".
    expect(screen.getByRole("heading", { level: 3, name: "Capacity" })).toBeDefined();
  });

  it("marks an unpadded band on the section, so the rule can drop the sides", () => {
    const { container } = render(
      <Section title="Nodes" padded={false}>
        rows
      </Section>,
    );
    expect(container.querySelector("section")?.getAttribute("data-padded")).toBe("false");
  });

  it("keeps the content its own direct children whether padded or not", () => {
    // Unpadding must not be a wrapper. A caller lays a section's content out,
    // and a box between that layout and the rows changes what it is placing.
    const { container } = render(
      <Section title="Nodes" padded={false}>
        <KV k="Status" v="Running" />
      </Section>,
    );
    expect(container.querySelector("section.section > .kv")).not.toBeNull();
  });
});

describe("a sticky section head over a sticky table head", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("offsets the table's own sticky header below the section head, not under it", () => {
    // Both `.subhead-caps` and `.tbl thead th` stick to `top: 0` of the same
    // scrolling pane — the overview's Nodes band wraps a `Table` directly
    // (`<Section title="Nodes" smallCaps padded={false}><Table .../></Section>`).
    // Left alone, the table's own column names would stick to the identical
    // coordinates the section head already occupies and lose to its higher
    // z-index, painted over and hidden the moment the list scrolls. Offsetting
    // the table head by the section head's own height (25px) stacks the two
    // instead of collapsing them onto each other. Asserted on the stylesheet:
    // jsdom does no layout, so nothing here can be observed by rendering.
    expect(components).toContain(".subhead-caps ~ .tbl thead th { top: 25px; }");
  });
});

describe("an unpadded band's rule", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("drops the sides only, and keeps the heading's inset", () => {
    // The band runs edge to edge; the heading is a label sitting over it and
    // lines up with nothing when it is flush to the window. The vertical
    // padding is the rhythm between one band and the next and must survive.
    expect(components).toContain(
      '.section[data-padded="false"] { padding-left: 0; padding-right: 0; }',
    );
    expect(components).toContain(
      '.section[data-padded="false"] > .section-title { padding-left: 0.75rem; padding-right: 0.75rem; }',
    );
    const rule = components.slice(components.indexOf('.section[data-padded="false"] {'));
    expect(rule.slice(0, rule.indexOf("}"))).not.toContain("padding-top");
  });
});
