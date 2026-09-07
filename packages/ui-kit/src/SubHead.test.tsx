import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { SubHead } from "./SubHead";

/** New: the mock shipped this component with no tests at all. (#320) */
describe("SubHead", () => {
  it("renders its content", () => {
    render(<SubHead>Containers</SubHead>);
    expect(screen.getByText("Containers")).toBeDefined();
  });

  it("is a heading, not a bold div", () => {
    // Every call site in the design labels a group inside a panel — Labels,
    // Annotations, Conditions, Containers. A styled div drops all of them out
    // of the document outline, which is the finding Panel's h2 came from.
    render(<SubHead>Annotations</SubHead>);
    expect(screen.getByRole("heading", { level: 3, name: "Annotations" })).toBeDefined();
  });

  it("keeps the design's size and weight", () => {
    // Preflight resets a heading's font-size and weight to inherit, so the
    // utilities are what makes an h3 look like this subheading rather than a
    // browser heading.
    const { container } = render(<SubHead>Clients</SubHead>);
    const head = container.querySelector("h3");
    expect(head?.className).toContain("font-semibold");
    expect(head?.className).toContain("text-[0.75rem]");
  });

  it("forwards className", () => {
    const { container } = render(<SubHead className="mb-1">Labels</SubHead>);
    expect(container.querySelector("h3.mb-1")).not.toBeNull();
  });
});

/**
 * The design carries two voices for a block's name (§C.3): the detail body's
 * small bold line, and the small-caps signpost the pane heads wear. The
 * cluster overview's bands want the second.
 */
describe("SubHead's two voices", () => {
  it("heads a block in small caps when asked, and only then", () => {
    const { container, rerender } = render(<SubHead variant="caps">Capacity</SubHead>);
    const caps = container.querySelector("h3");
    expect(caps?.className).toContain("subhead-caps");

    rerender(<SubHead>Capacity</SubHead>);
    expect(container.querySelector("h3")?.className).not.toContain("subhead-caps");
  });

  it("emits one size and weight, never two competing ones", () => {
    // The variants are not an override of each other. `text-[0.75rem]` and a
    // caps size would both be utilities, and two utilities setting the same
    // property are resolved by the generated stylesheet's order rather than by
    // the order the JSX writes them — so the loser is a coin flip. Exactly one
    // set is emitted, and the caps one is a components-layer class.
    const { container } = render(<SubHead variant="caps">Nodes</SubHead>);
    const head = container.querySelector("h3");
    expect(head?.className).not.toContain("text-[0.75rem]");
    expect(head?.className).not.toContain("font-semibold");
  });

  it("is still a level-3 heading in either voice", () => {
    render(<SubHead variant="caps">Not ready</SubHead>);
    expect(screen.getByRole("heading", { level: 3, name: "Not ready" })).toBeDefined();
  });
});

describe("the small-caps voice's recipe", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("is the design's pane-head recipe, in the components layer", () => {
    // §C.3: 10px / 600 / 0.07em / uppercase / --ink-faint — the same recipe
    // `.pane-head` wears, so a section heading and the strip above it read as
    // one system. Asserted on the stylesheet because jsdom does no layout.
    const rule = components.slice(components.indexOf("\n  .subhead-caps {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("font-size: 0.625rem");
    expect(body).toContain("font-weight: 600");
    expect(body).toContain("letter-spacing: 0.07em");
    expect(body).toContain("text-transform: uppercase");
    expect(body).toContain("color: var(--ink-faint)");
    // Tokens, never a literal colour.
    expect(body).not.toMatch(/#[0-9a-f]{3,8}/i);
  });

  it("sits on --surface-sunk with a bottom rule, not bare canvas", () => {
    // §C.3, verbatim: ".pane-head and .section-head use the same ... recipe
    // ON --surface-sunk, WITH A BOTTOM RULE". A band with no tint behind it
    // reads as flatter than the design — this is the finding that sent the
    // heads back to the recipe.
    const rule = components.slice(components.indexOf("\n  .subhead-caps {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("background: var(--surface-sunk)");
    expect(body).toContain("border-bottom: 1px solid var(--rule)");
  });

  it("is 25px and sticky within its pane", () => {
    // §C.3: "section heads are 25 px and sticky within their pane". A heading
    // that scrolls away with the rows above it stops saying what the rows
    // below it are — seen on a 113-row node table.
    const rule = components.slice(components.indexOf("\n  .subhead-caps {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("height: 25px");
    expect(body).toContain("position: sticky");
    expect(body).toContain("top: 0");
  });

  it("does not leave prose arguing for the background and rule it now wears", () => {
    // Caught four times on this project: a comment defending behaviour the
    // code no longer has. The old text argued the bands are "divided by the
    // rule between them rather than framed by one of their own" — exactly
    // the framing this recipe now draws.
    const comment = components.slice(
      components.indexOf("/* The design's section-head voice"),
      components.indexOf("\n  .subhead-caps {"),
    );
    expect(comment).not.toContain("without the sunk");
    expect(comment).not.toContain("divided by the rule between them rather than framed");
  });
});
