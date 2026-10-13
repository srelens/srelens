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

  describe("a field named level", () => {
    // `level:` in the query means the line's level word, not a field of that name, so a button on the raw
    // value would write a filter that matches nothing the row shows.
    const lineWith = (text: string, level: string, fields: Record<string, string>): DetailLine => ({
      raw: text,
      text,
      pod: "api-7",
      container: "api",
      level,
      health: level === "" ? "neutral" : "danger",
      fields,
    });

    it("filters on the level the line is read as, whatever spelling the field has", async () => {
      const { onToggle } = detail({
        line: lineWith('{"Level":30,"msg":"error pool timeout"}', "error", { Level: "30", msg: "error pool timeout" }),
      });
      expect(screen.getByText("30")).toBeTruthy();
      await userEvent.click(screen.getByRole("button", { name: "Filter out level = error" }));
      expect(onToggle).toHaveBeenCalledWith({ field: "level", value: "error" }, "exclude");
    });

    it("offers no filter when the line has no level word to filter on, as a numeric level has none", () => {
      detail({ line: lineWith('{"level":30,"msg":"hello"}', "", { level: "30", msg: "hello" }) });
      expect(screen.getByText("30")).toBeTruthy();
      expect(screen.queryByRole("button", { name: /level/i })).toBeNull();
      // The other fields are unaffected.
      expect(screen.getByRole("button", { name: "Filter for msg = hello" })).toBeTruthy();
    });
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

  // Machine text does not wrap (AGENTS.md): a thousand-character annotation
  // run through `break-all` becomes a paragraph that hides the short lines
  // around it. jsdom lays nothing out, so what can be held is the classes.
  describe("long machine text", () => {
    const long = "x".repeat(1000);

    it("lets a field value run on and scroll sideways, in a cell that cannot widen the panel", () => {
      const { container } = detail({ line: { ...LINE, level: "", fields: { annotation: long } } });
      const value = container.querySelector('[data-slot="detail-value"]') as HTMLElement;
      expect(value.textContent).toBe(long);
      expect(value.classList.contains("whitespace-pre")).toBe(true);
      expect(value.classList.contains("overflow-x-auto")).toBe(true);
      expect(value.classList.contains("break-all")).toBe(false);
      // A table cell grows to its content; `max-w-0` is what bounds it to the room the row leaves.
      expect(value.closest("td")?.classList.contains("max-w-0")).toBe(true);
    });

    it("lets the Raw block run on and scroll sideways", () => {
      const { container } = detail({ line: { ...LINE, raw: long, text: long, fields: {} } });
      const raw = container.querySelector("pre") as HTMLElement;
      expect(raw.textContent).toBe(long);
      expect(raw.classList.contains("whitespace-pre")).toBe(true);
      expect(raw.classList.contains("overflow-x-auto")).toBe(true);
      expect(raw.classList.contains("whitespace-pre-wrap")).toBe(false);
      expect(raw.classList.contains("break-all")).toBe(false);
    });

    it("breaks no text anywhere in the panel", () => {
      const { container } = detail();
      expect(container.innerHTML).not.toContain("break-all");
      expect(container.innerHTML).not.toContain("whitespace-pre-wrap");
    });
  });
});
