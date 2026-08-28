import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusPill } from "./StatusPill";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { toneColor } from "./tone";

describe("StatusPill", () => {
  it("renders the status label with a coloured dot", () => {
    const { container } = render(<StatusPill status="Running" kind="success" />);
    expect(screen.getByText("Running")).toBeDefined();
    const dot = container.querySelector(".dot") as HTMLElement;
    expect(dot).not.toBeNull();
    expect(dot.style.background).toContain("--ok");
  });

  it("defaults to a neutral dot", () => {
    const { container } = render(<StatusPill status="Unknown" />);
    expect(screen.getByText("Unknown")).toBeDefined();
    expect((container.querySelector(".dot") as HTMLElement).style.background).toContain("--ink-muted");
  });

  it("resolves every kind to a token, never a palette colour", () => {
    // The classic component wrote `bg-emerald-500`, `bg-amber-500` and
    // `bg-sky-500` — raw palette values that do not follow the theme, and whose
    // failure is invisible until someone switches to light.
    const kinds = ["success", "warning", "danger", "info", "neutral"] as const;
    for (const kind of kinds) {
      const { container, unmount } = render(<StatusPill status="x" kind={kind} />);
      const background = (container.querySelector(".dot") as HTMLElement).style.background;
      expect(background, `${kind} should resolve to a token`).toContain("var(--");
      unmount();
    }
  });

  it("never tells the status in colour alone", () => {
    // A dot with no words is unreadable to a colour-blind user and silent to a
    // screen reader.
    render(<StatusPill status="CrashLoopBackOff" kind="danger" />);
    expect(screen.getByText("CrashLoopBackOff")).toBeDefined();
  });

  it("maps success to the ok tone", () => {
    const { container } = render(<StatusPill status="x" kind="success" />);
    expect((container.querySelector(".dot") as HTMLElement).style.background).toBe(toneColor("ok"));
  });
});

/**
 * The mock's colouring rule: the status WORD takes the tone when the state is
 * bad, and stays plain when it is not — red `Degraded`, ordinary `Running`.
 * The same rule names conditions, which is the other thing this pill draws.
 *
 * Opt-in rather than the default. The pill has some forty call sites across
 * tables, headers and detail panes, and colouring every warning word in every
 * table cell is a different decision from the one the mock asked for. (#331)
 */
describe("StatusPill's colouring rule", () => {
  it("leaves the word plain by default, however bad the state", () => {
    const { container } = render(<StatusPill status="Degraded" kind="danger" />);
    const pill = container.querySelector(".status") as HTMLElement;
    expect(pill.getAttribute("data-bad")).toBeNull();
    expect(pill.style.color).toBe("");
  });

  it("colours the word for danger when asked", () => {
    const { container } = render(<StatusPill status="Degraded" kind="danger" tinted />);
    const pill = container.querySelector(".status") as HTMLElement;
    expect(pill.style.color).toBe(toneColor("sev"));
  });

  it("colours the word for a warning too", () => {
    const { container } = render(<StatusPill status="Progressing" kind="warning" tinted />);
    expect((container.querySelector(".status") as HTMLElement).style.color).toBe(toneColor("warn"));
  });

  it("leaves an ok state uncoloured even when tinted", () => {
    // Frame B: `Running` beside an ok dot is plain ink, and the mock's third
    // condition has an ok dot with an uncoloured name.
    for (const kind of ["success", "info", "neutral"] as const) {
      const { container, unmount } = render(<StatusPill status="Running" kind={kind} tinted />);
      const pill = container.querySelector(".status") as HTMLElement;
      expect(pill.style.color, `${kind} should stay plain`).toBe("");
      expect(pill.getAttribute("data-bad"), `${kind} is not bad`).toBeNull();
      unmount();
    }
  });

  it("marks the bad state so the stylesheet can weight it", () => {
    // `.status[data-bad="true"]` was written into kit.css and never emitted;
    // this is the attribute that wires it up.
    const { container } = render(<StatusPill status="Degraded" kind="danger" tinted />);
    expect((container.querySelector(".status") as HTMLElement).getAttribute("data-bad")).toBe("true");
  });

  it("still tells the state in words, not in the colour", () => {
    render(<StatusPill status="Degraded" kind="danger" tinted />);
    expect(screen.getByText("Degraded")).toBeDefined();
  });

  it("names no colour of its own when tinting", () => {
    const { container } = render(<StatusPill status="Degraded" kind="danger" tinted />);
    expect((container.querySelector(".status") as HTMLElement).style.color).toContain("var(--");
  });
});

describe("the status stylesheet", () => {
  it("keeps no rule that nothing can ever match", () => {
    const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
    const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));
    const selectors = components.match(/\.status\[data-[a-z-]+/g) ?? [];
    // Comments stripped first. The doc comment beside the JSX names the
    // selector in prose, so reading the whole file let the guard pass on the
    // comment alone — delete `data-bad` from the markup and the rule goes
    // dead again with the suite still green. PairList's "offers no way to opt
    // the value back in" strips for the same reason.
    const source = readFileSync(join(__dirname, "StatusPill.tsx"), "utf8").replace(
      /\/\*[\s\S]*?\*\/|\/\/.*/g,
      "",
    );
    for (const selector of selectors) {
      const attribute = selector.slice(".status[".length);
      expect(source, `${selector} matches nothing StatusPill emits`).toContain(attribute);
    }
  });
});
