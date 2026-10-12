import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LineDetail, type DetailLine } from "./LineDetail";

const TEXT = '{"level":"warn","msg":"Skipping","system":"camlog","bad key":"x"}';
const LINE: DetailLine = {
  raw: `2026-08-24T14:07:41.208123456Z ${TEXT}`,
  text: TEXT,
  pod: "api-7",
  container: "api",
  level: "warn",
  fields: { level: "warn", msg: "Skipping", system: "camlog", "bad key": "x" },
};

function detail(props: Partial<Parameters<typeof LineDetail>[0]> = {}) {
  const onToggle = vi.fn();
  const onClose = vi.fn();
  const view = render(<LineDetail line={LINE} inView clauses={[]} onToggle={onToggle} onClose={onClose} {...props} />);
  return { ...view, onToggle, onClose };
}

describe("LineDetail", () => {
  it("heads the panel with the line's own stamp and where it came from", () => {
    detail();
    expect(screen.getByText("2026-08-24T14:07:41.208123456Z")).toBeTruthy();
    expect(screen.getByText("api-7 · api")).toBeTruthy();
  });

  it("lists each field once, with filter buttons only where the query can name the key", () => {
    const { container } = detail();
    expect(Array.from(container.querySelectorAll('[data-slot="detail-field"] th')).map((th) => th.textContent)).toEqual([
      "level",
      "msg",
      "system",
      "bad key",
    ]);
    expect(screen.getByRole("button", { name: "Filter for system = camlog" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Filter for bad key = x" })).toBeNull();
  });

  it("filters for and against a field through the one toggle", async () => {
    const { onToggle } = detail();
    await userEvent.click(screen.getByRole("button", { name: "Filter out system = camlog" }));
    expect(onToggle).toHaveBeenCalledWith({ field: "system", value: "camlog" }, "exclude");
  });

  it("copies the raw line, stamp and all", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    detail();
    await userEvent.click(screen.getByRole("button", { name: /copy line/i }));
    expect(writeText).toHaveBeenCalledWith(LINE.raw);
  });

  it("says when the line has left the view", () => {
    detail({ inView: false });
    expect(screen.getByText("No longer in view")).toBeTruthy();
  });

  it("closes on Escape", async () => {
    const { onClose } = detail();
    screen.getByRole("button", { name: /copy line/i }).focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("pretty-prints a JSON line under Raw", () => {
    const { container } = detail();
    expect(container.querySelector("pre")?.textContent).toContain('\n  "level": "warn"');
  });
});
