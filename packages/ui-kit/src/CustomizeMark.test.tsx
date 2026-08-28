import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { CustomizeMark, type MarkAppearance } from "./CustomizeMark";
import type { IconComponent } from "./IconButton";

const Glyph: IconComponent = ({ size, className, "aria-hidden": hidden }) => (
  <svg width={size} height={size} className={className} aria-hidden={hidden} />
);

const COLORS = [
  { value: "#b4342a", label: "Brick" },
  { value: "#2a5fa8", label: "Slate blue" },
];

const ICONS = [
  { id: "server", label: "Server", icon: Glyph },
  { id: "cloud", label: "Cloud", icon: Glyph },
];

const BASE: MarkAppearance = {
  name: "prod-eu-west",
  short: "pew",
  color: "#b4342a",
  mark: "text",
  withText: true,
};

function setup(overrides: Partial<React.ComponentProps<typeof CustomizeMark>> = {}) {
  const onChange = vi.fn();
  const utils = render(
    <CustomizeMark value={BASE} onChange={onChange} colors={COLORS} icons={ICONS} {...overrides} />,
  );
  return { onChange, ...utils };
}

const fileInput = (container: HTMLElement) =>
  container.querySelector('input[type="file"]') as HTMLInputElement;

afterEach(() => vi.unstubAllGlobals());

/**
 * The mock's version read one cluster's override out of a module-level store,
 * wrote every edit straight back into it, and drew its symbols from a lucide
 * map. What is left once the store, the cluster and the icon set are the app's
 * is a controlled editor for how a mark looks — which is all of the mock's
 * behaviour and none of its dependencies. These cover the editing, the four
 * ways picking an image can fail, and the states the mock had no answer for: no
 * palette, no glyphs, and no name. (#320)
 */
describe("CustomizeMark", () => {
  it("shows the current name and short text", () => {
    setup();
    expect((screen.getByLabelText(/Display name/) as HTMLInputElement).value).toBe("prod-eu-west");
    expect((screen.getByLabelText(/Short text/) as HTMLInputElement).value).toBe("pew");
  });

  it("reports a new name without keeping any state of its own", () => {
    const { onChange } = setup();
    fireEvent.change(screen.getByLabelText(/Display name/), { target: { value: "prod" } });
    expect(onChange).toHaveBeenCalledWith({ ...BASE, name: "prod" });
  });

  it("caps the name rather than letting it run past the rail", () => {
    const { onChange } = setup();
    fireEvent.change(screen.getByLabelText(/Display name/), { target: { value: "x".repeat(40) } });
    expect(onChange.mock.calls[0][0].name).toHaveLength(28);
  });

  it("strips whitespace out of the short text and caps it at three", () => {
    // Three characters is what the mark can draw; the mock leaned on maxLength,
    // which a paste goes straight past.
    const { onChange } = setup();
    fireEvent.change(screen.getByLabelText(/Short text/), { target: { value: " a b c d " } });
    expect(onChange).toHaveBeenCalledWith({ ...BASE, short: "abc" });
  });

  it("says so when the name has been emptied", () => {
    setup({ value: { ...BASE, name: "  " } });
    expect(screen.getByText("A display name is required.")).toBeDefined();
    expect(screen.getByLabelText(/Display name/).getAttribute("aria-invalid")).toBe("true");
  });

  it("does not cry wolf over a name that is fine", () => {
    setup();
    expect(screen.queryByText("A display name is required.")).toBeNull();
    expect(screen.getByLabelText(/Display name/).getAttribute("aria-invalid")).toBeNull();
  });

  it("previews the mark at three sizes without announcing it three times", () => {
    const { container } = setup();
    const preview = container.querySelector('[data-slot="preview"]') as HTMLElement;
    expect(preview.querySelectorAll('[data-slot="chip-mark"]')).toHaveLength(3);
    // The chips are decorative: the name sits beside them in text, and three
    // identical "prod-eu-west, image" announcements say nothing three times.
    expect(within(preview).queryAllByRole("img")).toHaveLength(0);
    expect(within(preview).getByText("prod-eu-west")).toBeDefined();
    // Which three sizes: the rail's. Three unnamed squares beside each other
    // otherwise read as three marks rather than as one at every size it is
    // ever drawn at.
    expect(within(preview).getByText("preview at all three rail sizes")).toBeDefined();
  });

  it("offers the palette as one radio group, named by colour rather than by hex", () => {
    // The mock labelled each swatch "Colour #b4342a", which a screen reader
    // reads out digit by digit and which names nothing.
    setup();
    const brick = screen.getByRole("radio", { name: "Brick" }) as HTMLInputElement;
    const blue = screen.getByRole("radio", { name: "Slate blue" }) as HTMLInputElement;
    expect(brick.checked).toBe(true);
    expect(blue.checked).toBe(false);
    // One name means one tab stop and arrow keys between the swatches, which is
    // the whole keyboard contract of a radio group for the price of an attribute.
    expect(blue.name).toBe(brick.name);
  });

  it("reports the colour a swatch was chosen for", () => {
    const { onChange } = setup();
    fireEvent.click(screen.getByRole("radio", { name: "Slate blue" }));
    expect(onChange).toHaveBeenCalledWith({ ...BASE, color: "#2a5fa8" });
  });

  it("matches the current swatch whatever case it is written in", () => {
    setup({ value: { ...BASE, color: "#B4342A" } });
    expect((screen.getByRole("radio", { name: "Brick" }) as HTMLInputElement).checked).toBe(true);
  });

  it("drops the swatch row when the app offers no palette", () => {
    const { container } = setup({ colors: [] });
    expect(container.querySelector('[data-slot="swatches"]')).toBeNull();
    expect(screen.getByLabelText("Custom colour")).toBeDefined();
  });

  it("lets a colour be picked outside the palette", () => {
    const { onChange } = setup();
    const custom = screen.getByLabelText("Custom colour") as HTMLInputElement;
    expect(custom.value).toBe("#b4342a");
    fireEvent.change(custom, { target: { value: "#123456" } });
    expect(onChange).toHaveBeenCalledWith({ ...BASE, color: "#123456" });
  });

  it("paints the custom colour well from the colour itself, token and all", () => {
    // The well used to be the native input's own swatch, handed `""` for
    // anything that was not a six-digit hex. There is no empty state to put a
    // colour input in: HTML's value sanitization replaces any invalid value,
    // `""` included, with #000000 — the test below pins that down. ui-next
    // passes `var(--mark-*)` for all eleven swatches, so the reader picked
    // Green and got a green mark, a green swatch, the text `var(--mark-green)`
    // and a black well beside them.
    const { container } = setup({ value: { ...BASE, color: "var(--mark-green)" } });
    const well = container.querySelector('[data-slot="custom-colour"]') as HTMLElement;
    expect(well.style.background).toBe("var(--mark-green)");
    expect(screen.getByText("var(--mark-green)")).toBeDefined();
  });

  it("keeps the native input's unblankable swatch out of sight", () => {
    // Both halves of the claim above, in one place: the input really is
    // sanitized to black, and the thing the reader sees is not it. jsdom
    // applies no stylesheet, so the covering is asserted through the class
    // that drives it, the same proxy Table's tests use for `.tbl-resized`.
    setup({ value: { ...BASE, color: "var(--mark-green)" } });
    const custom = screen.getByLabelText("Custom colour") as HTMLInputElement;
    expect(custom.value).toBe("#000000");
    expect(custom.className.split(/\s+/)).toContain("opacity-0");
  });

  it("opens the picker on the current colour when that colour is a hex", () => {
    setup({ value: { ...BASE, color: "#123456" } });
    expect((screen.getByLabelText("Custom colour") as HTMLInputElement).value).toBe("#123456");
  });

  it("offers the three marks and reports the one chosen", () => {
    const { onChange } = setup();
    expect(screen.getByRole("radio", { name: "Text" })).toBeDefined();
    expect(screen.getByRole("radio", { name: "Image" })).toBeDefined();
    fireEvent.click(screen.getByRole("radio", { name: "Symbol" }));
    expect(onChange).toHaveBeenCalledWith({ ...BASE, mark: "icon" });
  });

  it("does not offer a symbol when the app ships no symbols", () => {
    // The kit has no icon set of its own, so an empty catalogue means the
    // choice leads to an empty grid.
    setup({ icons: [] });
    expect(screen.queryByRole("radio", { name: "Symbol" })).toBeNull();
    expect(screen.getByRole("radio", { name: "Text" })).toBeDefined();
  });

  it("shows the symbol grid only for a symbol mark, named by label not by id", () => {
    const { container, rerender } = setup();
    expect(container.querySelector('[data-slot="glyphs"]')).toBeNull();
    rerender(
      <CustomizeMark
        value={{ ...BASE, mark: "icon", icon: "server" }}
        onChange={() => {}}
        colors={COLORS}
        icons={ICONS}
      />,
    );
    expect(container.querySelector('[data-slot="glyphs"]')).not.toBeNull();
    expect((screen.getByRole("radio", { name: "Server" }) as HTMLInputElement).checked).toBe(true);
    expect(screen.getByRole("radio", { name: "Cloud" })).toBeDefined();
  });

  it("reports the symbol that was chosen", () => {
    const { onChange } = setup({ value: { ...BASE, mark: "icon", icon: "server" } });
    fireEvent.click(screen.getByRole("radio", { name: "Cloud" }));
    expect(onChange).toHaveBeenCalledWith({ ...BASE, mark: "icon", icon: "cloud" });
  });

  it("offers the short text on the mark only where there is a mark to put it on", () => {
    const { onChange, rerender } = setup();
    expect(screen.queryByRole("switch")).toBeNull();
    rerender(
      <CustomizeMark
        value={{ ...BASE, mark: "icon", icon: "server" }}
        onChange={onChange}
        colors={COLORS}
        icons={ICONS}
      />,
    );
    fireEvent.click(screen.getByRole("switch", { name: "Text on mark" }));
    expect(onChange).toHaveBeenCalledWith({ ...BASE, mark: "icon", icon: "server", withText: false });
  });

  it("opens the file picker from a real, named button", () => {
    const { container } = setup({ value: { ...BASE, mark: "image" } });
    const clicked = vi.fn();
    fileInput(container).addEventListener("click", clicked);
    fireEvent.click(screen.getByRole("button", { name: "Choose image" }));
    expect(clicked).toHaveBeenCalled();
  });

  it("reports a chosen image as a data URL", async () => {
    const { onChange, container } = setup({ value: { ...BASE, mark: "image" } });
    fireEvent.change(fileInput(container), {
      target: { files: [new File(["x"], "logo.png", { type: "image/png" })] },
    });
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const next = onChange.mock.calls[0][0] as MarkAppearance;
    expect(next.mark).toBe("image");
    expect(next.imageSrc?.startsWith("data:image/png")).toBe(true);
  });

  it("turns away a file that is not an image, out loud", () => {
    const { onChange, container } = setup({ value: { ...BASE, mark: "image" } });
    fireEvent.change(fileInput(container), {
      target: { files: [new File(["x"], "notes.txt", { type: "text/plain" })] },
    });
    expect(screen.getByRole("alert").textContent).toContain("not an image");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("turns away an image too big to keep, and says how big it was", () => {
    const { onChange, container } = setup({ value: { ...BASE, mark: "image" }, maxImageBytes: 1024 });
    fireEvent.change(fileInput(container), {
      target: { files: [new File(["x".repeat(4096)], "logo.png", { type: "image/png" })] },
    });
    expect(screen.getByRole("alert").textContent).toContain("4 KB");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("says so when the file cannot be read at all", async () => {
    // The mock wired up `onload` and nothing else, so an unreadable file left
    // the dialog sitting there as though nothing had been chosen.
    class FailingReader {
      onerror: (() => void) | null = null;
      onload: (() => void) | null = null;
      readAsDataURL() {
        this.onerror?.();
      }
    }
    vi.stubGlobal("FileReader", FailingReader);
    const { onChange, container } = setup({ value: { ...BASE, mark: "image" } });
    fireEvent.change(fileInput(container), {
      target: { files: [new File(["x"], "logo.png", { type: "image/png" })] },
    });
    await waitFor(() => expect(screen.getByRole("alert")).toBeDefined());
    expect(onChange).not.toHaveBeenCalled();
  });

  it("offers to remove an image only once there is one", () => {
    const { rerender, onChange } = setup({ value: { ...BASE, mark: "image" } });
    expect(screen.queryByRole("button", { name: "Remove image" })).toBeNull();
    rerender(
      <CustomizeMark
        value={{ ...BASE, mark: "image", imageSrc: "data:image/png;base64,AA" }}
        onChange={onChange}
        colors={COLORS}
        icons={ICONS}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove image" }));
    // Clearing the source but staying on the image mark leaves the editor
    // showing a mark with nothing in it; the mock left the stale source behind
    // as well.
    expect(onChange).toHaveBeenCalledWith({ ...BASE, mark: "text", imageSrc: undefined });
  });

  it("offers a reset only when there is somewhere to reset to", () => {
    const onReset = vi.fn();
    setup();
    expect(screen.queryByRole("button", { name: "Reset" })).toBeNull();
    setup({ onReset });
    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    expect(onReset).toHaveBeenCalled();
  });

  it("says type=button on every button it owns", () => {
    // A bare button inside a form is a submit button, and this editor is a form
    // full of them: the mock's reset, done, swatch, symbol, choose and remove
    // buttons would every one of them have submitted the form around it.
    const { container } = setup({
      value: { ...BASE, mark: "image", imageSrc: "data:image/png;base64,AA" },
      onReset: () => {},
    });
    const buttons = Array.from(container.querySelectorAll("button"));
    expect(buttons.length).toBeGreaterThan(2);
    expect(buttons.filter((b) => b.getAttribute("type") !== "button")).toEqual([]);
  });

  it("puts a title only on controls that can be hovered", () => {
    const { container } = setup({ onReset: () => {} });
    expect(container.querySelectorAll("button").length).toBeGreaterThan(0);
    expect(container.querySelectorAll(":not(button)[title]")).toHaveLength(0);
  });

  it("forwards className onto the editor", () => {
    const { container } = setup({ className: "extra" });
    expect(container.querySelector(".extra")).not.toBeNull();
    expect(container.querySelector(".extra")).toBe(container.firstElementChild);
  });
});
