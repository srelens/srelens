import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { DiffLines, type DiffRow } from "./DiffLines";

/**
 * `DiffLines` draws a `DiffRow[]` — same/insert/delete/replace — in the
 * design's tones. The trap this exists to catch: a `replace` row carries
 * *both* `left` and `right` on one object, and a component that does not
 * special-case it will either drop a side or render it in the wrong tone.
 */
describe("DiffLines", () => {
  const line = (container: HTMLElement, i: number) =>
    container.querySelectorAll('[data-slot="line"]')[i] as HTMLElement;

  it("renders context lines in the ink-soft tone, unmarked", () => {
    const rows: DiffRow[] = [{ tag: "same", left: "replicaCount: 12", right: "replicaCount: 12" }];
    const { container } = render(<DiffLines rows={rows} />);
    const el = line(container, 0);
    expect(el.textContent).toContain("replicaCount: 12");
    expect(el.style.color).toContain("var(--ink-soft)");
    expect(el.getAttribute("data-tag")).toBe("same");
  });

  it("renders an insert on the ok wash, in ok", () => {
    const rows: DiffRow[] = [{ tag: "insert", left: null, right: 'tag: "4f2a1c"' }];
    const { container } = render(<DiffLines rows={rows} />);
    const el = line(container, 0);
    expect(el.textContent).toContain('tag: "4f2a1c"');
    expect(el.style.color).toContain("var(--ok)");
    expect(el.style.backgroundColor || el.style.background).toContain("var(--ok-wash)");
  });

  it("renders a delete on the sev wash, in sev", () => {
    const rows: DiffRow[] = [{ tag: "delete", left: 'tag: "118a7e"', right: null }];
    const { container } = render(<DiffLines rows={rows} />);
    const el = line(container, 0);
    expect(el.textContent).toContain('tag: "118a7e"');
    expect(el.style.color).toContain("var(--sev)");
    expect(el.style.backgroundColor || el.style.background).toContain("var(--sev-wash)");
  });

  it("draws a replace row as its own deletion followed by its own addition, from one row", () => {
    // The row-shape trap: one DiffRow with both sides present, not a pair of
    // rows in the array. Two lines come out, but from a single input row.
    const rows: DiffRow[] = [{ tag: "replace", left: "DB_POOL_MAX: \"40\"", right: "DB_POOL_MAX: \"5\"" }];
    const { container } = render(<DiffLines rows={rows} />);
    const lines = container.querySelectorAll('[data-slot="line"]');
    expect(lines.length).toBe(2);

    const del = lines[0] as HTMLElement;
    expect(del.getAttribute("data-tag")).toBe("delete");
    expect(del.textContent).toContain('DB_POOL_MAX: "40"');
    expect(del.style.color).toContain("var(--sev)");

    const ins = lines[1] as HTMLElement;
    expect(ins.getAttribute("data-tag")).toBe("insert");
    expect(ins.textContent).toContain('DB_POOL_MAX: "5"');
    expect(ins.style.color).toContain("var(--ok)");
  });

  it("does not double a normal delete/insert pair that already arrives as two rows", () => {
    // The common case: diffTextLines' LCS path never emits "replace" — a
    // changed line already arrives as a delete row followed by an insert row.
    // Those must stay exactly two lines, not four.
    const rows: DiffRow[] = [
      { tag: "delete", left: 'DB_POOL_MAX: "40"', right: null },
      { tag: "insert", left: null, right: 'DB_POOL_MAX: "5"' },
    ];
    const { container } = render(<DiffLines rows={rows} />);
    expect(container.querySelectorAll('[data-slot="line"]').length).toBe(2);
  });

  it("renders nothing rather than an empty frame for an empty list", () => {
    const { container } = render(<DiffLines rows={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("wraps rather than forcing the pane to scroll horizontally", () => {
    const longLine = "x".repeat(400);
    const rows: DiffRow[] = [{ tag: "same", left: longLine, right: longLine }];
    const { container } = render(<DiffLines rows={rows} />);
    const el = line(container, 0);
    expect(el.className).toMatch(/whitespace-pre-wrap/);
  });

  it("sets no title attribute anywhere in the tree", () => {
    // A Secret leaked through a `title` attribute before; a diff line is
    // exactly the kind of content that could carry one again.
    const rows: DiffRow[] = [
      { tag: "same", left: "a", right: "a" },
      { tag: "delete", left: "b", right: null },
      { tag: "insert", left: null, right: "c" },
      { tag: "replace", left: "d", right: "e" },
    ];
    const { container } = render(<DiffLines rows={rows} />);
    expect(container.querySelectorAll("[title]").length).toBe(0);
  });

  it("keeps rows in order across a mixed hunk", () => {
    const rows: DiffRow[] = [
      { tag: "same", left: "env:", right: "env:" },
      { tag: "replace", left: 'DB_POOL_MAX: "40"', right: 'DB_POOL_MAX: "5"' },
      { tag: "same", left: "DB_POOL_TIMEOUT: \"30s\"", right: "DB_POOL_TIMEOUT: \"30s\"" },
    ];
    const { container } = render(<DiffLines rows={rows} />);
    const lines = container.querySelectorAll('[data-slot="line"]');
    expect(lines.length).toBe(4);
    expect(lines[0].textContent).toContain("env:");
    expect(lines[1].getAttribute("data-tag")).toBe("delete");
    expect(lines[2].getAttribute("data-tag")).toBe("insert");
    expect(lines[3].textContent).toContain("DB_POOL_TIMEOUT");
  });
});
