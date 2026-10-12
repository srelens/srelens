import { afterEach, describe, it, expect, vi } from "vitest";
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
  health: "warning",
  fields: { level: "warn", msg: "Skipping", system: "camlog", "bad key": "x" },
};

function detail(props: Partial<Parameters<typeof LineDetail>[0]> = {}) {
  const onToggle = vi.fn();
  const onClose = vi.fn();
  const view = render(<LineDetail line={LINE} inView clauses={[]} onToggle={onToggle} onClose={onClose} {...props} />);
  return { ...view, onToggle, onClose };
}

describe("LineDetail", () => {
  // A test that installs a clipboard must not leave it for the next one.
  const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  afterEach(() => {
    if (clipboard !== undefined) Object.defineProperty(navigator, "clipboard", clipboard);
    else Reflect.deleteProperty(navigator, "clipboard");
  });

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

  it("says so when the copy fails, and forgets it once another line is shown", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockRejectedValue(new Error("denied"));
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { rerender, onToggle, onClose } = detail();
    await userEvent.click(screen.getByRole("button", { name: /copy line/i }));
    expect(await screen.findByText("Could not copy the line")).toBeTruthy();
    rerender(<LineDetail line={{ ...LINE, raw: `${LINE.raw} `, text: `${LINE.text} ` }} inView clauses={[]} onToggle={onToggle} onClose={onClose} />);
    expect(screen.queryByText("Could not copy the line")).toBeNull();
  });

  it("names the level in the header, in the tone core gives it, and draws nothing for a line with none", () => {
    const { container, unmount } = detail();
    const pill = container.querySelector("header .status");
    expect(pill?.textContent).toBe("warn");
    expect(pill?.getAttribute("data-kind")).toBe("warning");
    unmount();
    const bare = detail({ line: { ...LINE, level: "", health: "neutral", fields: {} } });
    expect(bare.container.querySelector("header .status")).toBeNull();
  });

  it("says so when a line came without a stamp", () => {
    detail({ line: { ...LINE, raw: TEXT } });
    expect(screen.getByText("no timestamp")).toBeTruthy();
  });

  it("gives a plain-text line's level a row of its own, and a filter for it", async () => {
    const text = "warn disk almost full";
    const { container, onToggle } = detail({
      line: { raw: `2026-08-24T14:07:41Z ${text}`, text, pod: "api-7", container: "api", level: "warn", health: "warning", fields: {} },
    });
    expect(Array.from(container.querySelectorAll('[data-slot="detail-field"] th')).map((th) => th.textContent)).toEqual(["level"]);
    await userEvent.click(screen.getByRole("button", { name: "Filter for level = warn" }));
    expect(onToggle).toHaveBeenCalledWith({ field: "level", value: "warn" }, "include");
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
