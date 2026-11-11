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

  it.each([
    ["undefined", undefined],
    ["null", null],
    ["an empty string", ""],
  ])("draws one dash for a value that is %s", (_, value) => {
    // Missing was written four ways across the details: a dash, nothing at
    // all, a muted dash and a zero. The row says it, so no caller has to.
    const { container } = render(<KV k="External IP" v={value} />);
    expect(container.querySelector(".kv-v")?.textContent).toBe("—");
  });

  it("keeps a zero, which is a value", () => {
    const { container } = render(<KV k="Restarts" v={0} />);
    expect(container.querySelector(".kv-v")?.textContent).toBe("0");
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

  it("wraps a long key onto a second line, then clips it, and never paints it over the value", () => {
    // In a 352px peek the column is about 120px, so "Volume claim templates"
    // and "Image pull secrets" could not be read on one line (#1001). Two lines
    // read them. The clamp is for the other kind of key: the overview rail's
    // Fleet rows are keyed on the kube CONTEXT name, a real kubeconfig carries
    // `kdev-1787048764-kubernetes-admin@cluster-...`, and a row four lines tall
    // would push the rows under it down for the sake of one name.
    const rule = components.slice(components.indexOf("  .kv-k {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).not.toContain("white-space: nowrap");
    expect(body).toContain("-webkit-line-clamp: 2");
    expect(body).toContain("overflow-wrap: anywhere");
    // Without this the grid item's automatic minimum is its min-content size,
    // which is what painted a long key through the value column and out past
    // the right edge of the 286px rail: "kind-srelens-dem30/30 running".
    expect(body).toContain("min-width: 0");
    expect(body).toContain("overflow: hidden");
  });

  it("caps the key column, so a wide pane does not hold its values a third of the way across", () => {
    // A third of the pane is right at 352px (about 120px) and wrong at the
    // ~700px a reader drags the peek to: the column grew to ~240px, and Pod
    // facts sat across a gap from their labels while a toleration broke onto a
    // second line for want of the width the labels were not using (#1070).
    // 9rem is under a third at every width past ~420px and over it below, so
    // a narrow pane keeps exactly the column it had.
    const rule = components.slice(components.indexOf("  .kv {"));
    expect(rule.slice(0, rule.indexOf("}"))).toContain(
      "grid-template-columns: minmax(88px, min(34%, 9rem)) 1fr;",
    );
  });

  it("sets a code value at the size of any other value", () => {
    // Identifiers were the smallest text in the pane: 11px beside 12px.
    expect(components).not.toMatch(/\.kv-v\.code\s*\{/);
  });

  it("sets a list of pairs at the size of any other value", () => {
    // A `PairList` IS the value on a Selector or Node selector row.
    const rule = components.slice(components.indexOf("  .pairs {"));
    expect(rule.slice(0, rule.indexOf("}"))).toContain("font-size: 0.75rem");
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

// Machine text does not wrap (AGENTS.md): a context, a server URL or a kubeconfig
// path broken across two lines is misread, and `prod` beside `prod-eu` must stay
// whole. Wrapping stays the default, as for `DiffLines`. (#1001)
describe("a KV that does not wrap", () => {
  it("keeps the value on one line, spaces and all, and scrolls it sideways", () => {
    const { container } = render(<KV k="API server" v="https://10.0.0.1:6443" mono wrap={false} />);
    const value = container.querySelector(".kv-v") as HTMLElement;
    expect(value.className).toMatch(/(^|\s)whitespace-pre(\s|$)/);
    expect(value.className).toMatch(/(^|\s)overflow-x-auto(\s|$)/);
  });

  it("wraps by default", () => {
    const { container } = render(<KV k="Detail" v="Two replicas are not ready" />);
    const value = container.querySelector(".kv-v") as HTMLElement;
    expect(value.className).not.toMatch(/whitespace-pre|overflow-x-auto/);
  });
});
