import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { KV, KVList } from "./KV";

/** New: the mock shipped these components with no tests at all. (#320) */
describe("KV", () => {
  it("renders the key and the value", () => {
    render(<KV k="Namespace" v="kube-system" />);
    expect(screen.getByText("Namespace")).toBeDefined();
    expect(screen.getByText("kube-system")).toBeDefined();
  });

  it("pairs them as a name and its value", () => {
    // A key and its value are a name/value group, and dl/dt/dd is the markup
    // that says so; two spans say only "two spans".
    const { container } = render(<KV k="Namespace" v="kube-system" />);
    const row = container.querySelector("dl.kv");
    expect(row).not.toBeNull();
    expect(row?.querySelector("dt.kv-k")?.textContent).toBe("Namespace");
    expect(row?.querySelector("dd.kv-v")?.textContent).toBe("kube-system");
  });

  it("carries the whole group itself, so a lone row is valid anywhere", () => {
    // KV is used on its own as often as through KVList in the design, and a dt
    // outside a dl is markup the browser drops the semantics of.
    const { container } = render(<KV k="Namespace" v="kube-system" />);
    expect(container.firstElementChild?.tagName).toBe("DL");
  });

  it("renders the value in the code face when told to", () => {
    const { container } = render(<KV k="Image" v="nginx:1.25" mono />);
    expect(container.querySelector(".kv-v.code")).not.toBeNull();
  });

  it("leaves the value in the UI face otherwise", () => {
    // The mono face is for identifiers; prose set in it reads as a command.
    const { container } = render(<KV k="Status" v="Running" />);
    expect(container.querySelector(".kv-v.code")).toBeNull();
  });

  it("adds no title attribute", () => {
    const { container } = render(<KV k="Status" v="Running" />);
    expect(container.querySelector(".kv-v")?.hasAttribute("title")).toBe(false);
  });

  it("never writes a value into an attribute", () => {
    // KV used to accept an explicit `title` set to the value in full. `.kv-v`
    // wraps a long value onto another line rather than truncating it
    // (`overflow-wrap: anywhere`, no `text-overflow`), so the title was never
    // standing in for truncation here — it was only ever a second copy of the
    // value sitting in the DOM, the same disclosure hole `PairList` removed
    // after a `kubectl apply`-managed Secret leaked through it. (#331)
    const secret = "FAKE-NOT-A-REAL-TOKEN-aHVudGVyMg==";
    const { container } = render(<KV k="Token" v={secret} />);
    const cell = container.querySelector(".kv-v") as HTMLElement;
    for (const attribute of Array.from(cell.attributes)) {
      expect(attribute.value, `${attribute.name} carries the value`).not.toContain(secret);
    }
    expect(cell.hasAttribute("title")).toBe(false);
  });

  it("forwards className onto the row", () => {
    const { container } = render(<KV k="Status" v="Running" className="extra" />);
    expect(container.querySelector(".kv.extra")).not.toBeNull();
  });
});

/**
 * The same pair, read on a page instead of down a column: the label above the
 * value, ruled off beneath.
 *
 * A form of the ROW, chosen by whoever renders it — not a wrapper reaching
 * down into someone else's markup. `FactGrid` was that wrapper: it restyled
 * the peek's rows into the full tab's grid, which meant the tab's layout was
 * described in terms of a body the tab did not build, and every new kind of
 * child needed another exception. The tab now lays its own grid out and asks
 * for the row it wants. (#331)
 */
describe("a stacked KV", () => {
  it("says so on the row, so the form is the row's own", () => {
    const { container } = render(<KV k="Pod IP" v="10.44.21.4" stacked />);
    expect(container.querySelector<HTMLElement>(".kv")!.dataset.stacked).toBe("true");
  });

  it("is still a name and its value, with the same key and value elements", () => {
    // The markup a screen reader hears must not change with the layout.
    const { container } = render(<KV k="Pod IP" v="10.44.21.4" stacked />);
    expect(container.querySelector("dl.kv > dt.kv-k")!.textContent).toBe("Pod IP");
    expect(container.querySelector("dl.kv > dd.kv-v")!.textContent).toBe("10.44.21.4");
  });

  it("leaves an ordinary row unmarked, so nothing about the column form changes", () => {
    const { container } = render(<KV k="Pod IP" v="10.44.21.4" />);
    expect(container.querySelector<HTMLElement>(".kv")!.dataset.stacked).toBeUndefined();
  });

  it("writes no value into an attribute, stacked or not", () => {
    // The same disclosure rule the column form is held to: a value lives in
    // the document once, as text.
    const { container } = render(<KV k="Token" v="fixture-only-value" stacked />);
    const row = container.querySelector(".kv")!;
    for (const el of [row, ...row.querySelectorAll("*")]) {
      for (const attr of el.getAttributeNames()) {
        expect(el.getAttribute(attr)).not.toContain("fixture-only-value");
      }
    }
  });
});

describe("the stacked row's rules", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("stacks the label over the value and rules beneath the pair", () => {
    const rule = components.slice(components.indexOf('  .kv[data-stacked="true"] {'));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("display: block");
    expect(body).toContain("border-bottom");
  });

  it("leaves no rule that lays a body out from a surface wrapping it", () => {
    // `FactGrid` is gone. Its rules were the counter-example: a wrapper
    // restyling children it did not build, which made the full tab's layout
    // a property of the peek's DOM and needed a fresh exception for every
    // child that was not a fact row.
    expect(components).not.toContain(".factgrid");
  });

  it("keeps every one of those rules on the row itself, never on an ancestor", () => {
    // What went wrong with `FactGrid`: its rules were descendant selectors
    // under a wrapper, so the layout belonged to whoever wrapped the body
    // rather than to the row being laid out.
    const stacked = components.match(/^\s*\S*\.kv\[data-stacked="true"\][^\n]*\{/gm) ?? [];
    expect(stacked.length).toBeGreaterThan(0);
    expect(stacked.every((r) => r.trim().startsWith(".kv[data-stacked"))).toBe(true);
  });
});

describe("the key column's rules", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("truncates a key too long for its column instead of letting it paint over the value", () => {
    // Found on the live cluster: the overview rail's Fleet section keys its
    // rows on the kube CONTEXT name, and a real kubeconfig carries names like
    // `kdev-1787048764-kubernetes-admin@cluster-...`. The key column is a
    // `minmax(88px, 34%)` track — its max is fixed, so the track cannot grow —
    // and a `white-space: nowrap` key with no `min-width: 0` and no overflow
    // rule simply painted straight through the value column and out past the
    // right edge of the 286px rail: "kind-srelens-dem30/30 running".
    //
    // An ellipsis rather than a wrap: every other name in this design that
    // outgrows its space is clipped the same way — the tab strip's titles, the
    // sidebar's cluster name — and a key that wraps to three lines pushes its
    // own value off the row's baseline.
    const rule = components.slice(components.indexOf("  .kv-k {"));
    const body = rule.slice(0, rule.indexOf("}"));
    // Without this the grid item's automatic minimum is its min-content size,
    // which for a nowrap key is the whole string — the overflow rules below
    // never get a chance to apply.
    expect(body).toContain("min-width: 0");
    expect(body).toContain("overflow: hidden");
    expect(body).toContain("text-overflow: ellipsis");
  });
});

describe("KVList", () => {
  const rows: Array<[string, string]> = [
    ["Kind", "Pod"],
    ["Namespace", "kube-system"],
    ["Image", "nginx:1.25"],
  ];

  it("renders a row per tuple, in the order given", () => {
    const { container } = render(<KVList rows={rows} />);
    expect(Array.from(container.querySelectorAll(".kv-k")).map((e) => e.textContent)).toEqual([
      "Kind",
      "Namespace",
      "Image",
    ]);
    expect(Array.from(container.querySelectorAll(".kv-v")).map((e) => e.textContent)).toEqual([
      "Pod",
      "kube-system",
      "nginx:1.25",
    ]);
  });

  it("adds no wrapper of its own", () => {
    // The rows are meant to land as children of whatever laid the panel out; a
    // block wrapper between them and a flex or grid parent changes the layout.
    const { container } = render(<KVList rows={rows} />);
    expect(container.children.length).toBe(3);
    expect(container.firstElementChild?.classList.contains("kv")).toBe(true);
  });

  it("applies the mono predicate value by value", () => {
    const { container } = render(<KVList rows={rows} mono={(v) => v.includes(":")} />);
    const mono = Array.from(container.querySelectorAll(".kv-v")).map((e) =>
      e.classList.contains("code"),
    );
    expect(mono).toEqual([false, false, true]);
  });

  it("leaves every value in the UI face when no predicate is given", () => {
    const { container } = render(<KVList rows={rows} />);
    expect(container.querySelectorAll(".kv-v.code").length).toBe(0);
  });

  it("does not ask the predicate about a value that is not a string", () => {
    // The predicate is written against text; a node has no text to test, and
    // handing it one would throw inside a caller's one-line arrow function.
    const { container } = render(
      <KVList rows={[["Owner", <a key="o" href="#x">rs/web</a>]]} mono={(v) => v.length > 0} />,
    );
    expect(container.querySelector(".kv-v.code")).toBeNull();
    expect(container.querySelector(".kv-v")?.hasAttribute("title")).toBe(false);
  });

  it("never writes a value into an attribute", () => {
    // This row used to derive `title` from every string value, unasked — the
    // same disclosure hole `PairList` removed after a `kubectl
    // apply`-managed Secret leaked through it via an annotation. (#331)
    const secret = "FAKE-NOT-A-REAL-TOKEN-aHVudGVyMg==";
    const { container } = render(<KVList rows={[["Token", secret]]} />);
    const cell = container.querySelector(".kv-v") as HTMLElement;
    for (const attribute of Array.from(cell.attributes)) {
      expect(attribute.value, `${attribute.name} carries the value`).not.toContain(secret);
    }
    expect(cell.hasAttribute("title")).toBe(false);
  });

  it("renders nothing at all for an empty list", () => {
    const { container } = render(<KVList rows={[]} />);
    expect(container.innerHTML).toBe("");
  });

  it("offers no way to opt a value back into an attribute", () => {
    // A prop that puts it back is a prop someone passes on a Secret. Guards
    // both KV and KVList, which share this file. (#331)
    const source = readFileSync(join(__dirname, "KV.tsx"), "utf8");
    expect(source.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "")).not.toContain("title=");
  });
});
