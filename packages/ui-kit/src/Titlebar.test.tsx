import { describe, it, expect } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { Titlebar } from "./Titlebar";
import { toneColor } from "./tone";

function setup(props: Partial<Parameters<typeof Titlebar>[0]> = {}) {
  return render(
    <Titlebar
      leading={<button type="button">prod</button>}
      title={<span>srelens — prod-us-east</span>}
      actions={<button type="button">Zoom in</button>}
      {...props}
    />,
  );
}

const bar = () => screen.getByRole("banner");
const lights = () => bar().querySelectorAll("[data-light]");
const noDragStyle = (el: Element | null) =>
  (el as HTMLElement | null)?.style as (CSSStyleDeclaration & { WebkitAppRegion?: string }) | undefined;

/**
 * The window's own chrome: the bar across the top holding the platform's window
 * controls, whatever names the window, and the controls that act on it.
 *
 * The visual only. Which platform's controls belong up here, and whether the
 * app draws its own chrome at all, is a decision that lives with the window —
 * so it arrives as a prop and this component never asks the machine it is
 * running on. (#320)
 */
describe("Titlebar", () => {
  it("puts each slot where the design's three columns expect it", () => {
    setup();
    expect(screen.getByRole("button", { name: "prod" })).toBeDefined();
    expect(screen.getByText("srelens — prod-us-east")).toBeDefined();
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeDefined();
  });

  it("is a banner, named, so it can be skipped past", () => {
    setup({ label: "Window" });
    expect(screen.getByRole("banner", { name: "Window" })).toBeDefined();
  });

  it("wears the design's own chrome styling", () => {
    setup();
    expect(bar().className).toContain("titlebar");
    expect(screen.getByText("srelens — prod-us-east").closest(".path")).not.toBeNull();
  });

  it("renders no wrapper for a slot left empty", () => {
    // `filled`, not a null check: `actions={canZoom && <Zoom />}` is how a slot
    // is made conditional and hands over `false`, which renders nothing but
    // still takes a wrapper's padding and its share of the row's gap.
    const { container } = render(<Titlebar title="srelens" actions={false} leading={[]} />);
    expect(screen.getByText("srelens")).toBeDefined();
    expect(container.querySelectorAll("[data-slot]")).toHaveLength(0);
  });
});

/**
 * The half of this component that is a window rather than a page. The bar is
 * the drag handle; anything pressable standing on it has to opt out, or the
 * press is swallowed by the window move.
 */
describe("Titlebar's drag region", () => {
  it("makes the bar itself draggable", () => {
    // `.titlebar` carries `-webkit-app-region: drag` in the stylesheet, and the
    // centre column is the part of it left bare for that purpose.
    setup();
    expect(bar().querySelector("[data-drag-region]")).not.toBeNull();
  });

  it("carries the attribute tauri's drag handling listens for on the header and its structural columns, but not on the interactive slots", () => {
    // The stylesheet's app-region rule is honoured by WebView2 and ignored by
    // macOS's WKWebView — under an overlay titlebar there, only elements with
    // `data-tauri-drag-region` start a native drag. The header and its two
    // bare columns (the leading row and the centre drag region) carry it; the
    // slots holding what the caller passed must not, or a click there would
    // start a window move instead of reaching the control.
    setup();
    const header = bar();
    const leadingSlot = header.querySelector('[data-slot="leading"]') as HTMLElement;
    const actionsSlot = header.querySelector('[data-slot="actions"]') as HTMLElement;
    const leadingColumn = leadingSlot.parentElement as HTMLElement;
    const titleColumn = header.querySelector("[data-drag-region]") as HTMLElement;

    expect(header.hasAttribute("data-tauri-drag-region")).toBe(true);
    expect(leadingColumn.hasAttribute("data-tauri-drag-region")).toBe(true);
    expect(titleColumn.hasAttribute("data-tauri-drag-region")).toBe(true);

    expect(leadingSlot.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(actionsSlot.hasAttribute("data-tauri-drag-region")).toBe(false);
  });

  it("opts the control slots back out of it", () => {
    // jsdom keeps a property it does not recognise on the CSSOM object without
    // writing it into the style attribute, so this reads the object. In the
    // webview it is the difference between a button that can be clicked and a
    // button that moves the window.
    setup();
    for (const slot of bar().querySelectorAll("[data-slot]")) {
      expect(noDragStyle(slot)?.WebkitAppRegion).toBe("no-drag");
    }
  });

  it("leaves the drag region itself draggable", () => {
    setup();
    expect(noDragStyle(bar().querySelector("[data-drag-region]"))?.WebkitAppRegion).toBeUndefined();
  });
});

/**
 * The traffic lights are a picture of the window controls, not the controls.
 *
 * The window is the shell's, not the design system's: on macOS the real close,
 * minimise and zoom belong to the OS, and this kit has no way to reach them.
 * Drawing them as buttons would put three keyboard-reachable, screen-reader
 * announced controls on the page that do nothing at all when pressed, which is
 * worse than the decoration they replace. So they are decoration, explicitly.
 */
describe("Titlebar's window controls", () => {
  it("draws none by default", () => {
    setup();
    expect(lights()).toHaveLength(0);
  });

  it("draws none when the caller says there are none", () => {
    setup({ controls: "none" });
    expect(lights()).toHaveLength(0);
  });

  it("draws three macOS lights when asked for them", () => {
    setup({ controls: "macos" });
    expect(lights()).toHaveLength(3);
  });

  it("keeps them out of the accessibility tree and off the tab order", () => {
    setup({ controls: "macos" });
    const group = bar().querySelector("[data-window-controls]") as HTMLElement;
    expect(group.getAttribute("aria-hidden")).toBe("true");
    expect(within(group).queryAllByRole("button")).toHaveLength(0);
    expect(group.querySelectorAll("button, [tabindex]")).toHaveLength(0);
  });

  it("colours them from tokens, so they follow the theme", () => {
    // The mock wrote #ff5f57, #febc2e and #28c840 straight into the markup —
    // three colours that do not move when the theme does, and which the kit's
    // own lint forbids.
    setup({ controls: "macos" });
    const backgrounds = Array.from(lights(), (light) => (light as HTMLElement).style.background);
    expect(backgrounds).toEqual([toneColor("sev"), toneColor("warn"), toneColor("ok")]);
  });

  it("rules the lights off from the leading slot", () => {
    setup({ controls: "macos" });
    expect(bar().querySelector("[data-chrome-rule]")).not.toBeNull();
  });

  it("draws no rule when there is nothing on the other side of it", () => {
    const { container } = render(<Titlebar controls="macos" title="srelens" />);
    expect(container.querySelector("[data-chrome-rule]")).toBeNull();
    expect(container.querySelectorAll("[data-light]")).toHaveLength(3);
  });
});
