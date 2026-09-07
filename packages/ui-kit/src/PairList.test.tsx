import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { PairList } from "./PairList";

/** New: the mock shipped this component with no tests at all. (#320) */
describe("PairList", () => {
  const pairs: Array<[string, string]> = [
    ["app", "web"],
    ["app.kubernetes.io/managed-by", "Helm"],
  ];

  it("prints each pair the way kubectl does", () => {
    const { container } = render(<PairList pairs={pairs} />);
    expect(Array.from(container.querySelectorAll("li")).map((e) => e.textContent)).toEqual([
      "app=web",
      "app.kubernetes.io/managed-by=Helm",
    ]);
  });

  it("is a list, and says so", () => {
    // Labels and annotations are a set of things, and how many there are is
    // part of reading them.
    render(<PairList pairs={pairs} />);
    expect(screen.getByRole("list")).toBeDefined();
    expect(screen.getAllByRole("listitem").length).toBe(2);
  });

  it("keeps each pair on one line by default", () => {
    // A wall of annotations is scanned by key; a value that wraps to four
    // lines buries the next key.
    const { container } = render(<PairList pairs={pairs} />);
    const row = container.querySelector("li");
    expect(row?.className).toContain("truncate");
    expect(container.querySelector(".v")?.className).not.toContain("break-all");
  });

  it("lets long values wrap when asked", () => {
    const { container } = render(<PairList pairs={pairs} breakValues />);
    const row = container.querySelector("li");
    expect(row?.className ?? "").not.toContain("truncate");
    expect(container.querySelector(".v")?.className).toContain("break-all");
  });

  it("never writes a value into an attribute", () => {
    // This row used to carry `title={`${k}=${v}`}`, which put the value in the
    // DOM even when it was visually truncated — and a `kubectl apply`-managed
    // Secret keeps its whole base64 `data` map inside the
    // `last-applied-configuration` annotation, which arrives here as a pair.
    // The detail pane hid annotations behind a toggle to work around exactly
    // this. Truncation is a visual affordance and must not be a disclosure
    // boundary that leaks. (#331)
    const secret: Array<[string, string]> = [
      ["kubectl.kubernetes.io/last-applied-configuration", '{"data":{"password":"aHVudGVyMg=="}}'],
    ];
    const { container } = render(<PairList pairs={secret} />);
    const row = container.querySelector("li") as HTMLElement;
    for (const attribute of Array.from(row.attributes)) {
      expect(attribute.value, `${attribute.name} carries the value`).not.toContain("aHVudGVyMg==");
    }
    expect(row.getAttribute("title")).toBeNull();
  });

  it("offers no way to opt the value back into an attribute", () => {
    // A prop that puts it back is a prop someone passes on a Secret.
    const source = readFileSync(join(__dirname, "PairList.tsx"), "utf8");
    expect(source.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "")).not.toContain("title=");
  });

  it("reaches the un-truncated form the mock draws", () => {
    // Full-width monospace lines, one per label, no truncation — which is now
    // the only way to read a long value in full.
    const { container } = render(<PairList pairs={pairs} breakValues />);
    expect(container.querySelector("li")?.className ?? "").not.toContain("truncate");
    expect(container.querySelector("li")?.textContent).toBe("app=web");
  });

  it("renders nothing at all for an empty list", () => {
    // `.pairs` sets a line-height and sits between two blocks; an empty one is
    // a gap the caller did not ask for.
    const { container } = render(<PairList pairs={[]} />);
    expect(container.querySelector(".pairs")).toBeNull();
    expect(container.innerHTML).toBe("");
  });

  it("forwards className onto the list", () => {
    const { container } = render(<PairList pairs={pairs} className="extra" />);
    expect(container.querySelector(".pairs.extra")).not.toBeNull();
  });
});
