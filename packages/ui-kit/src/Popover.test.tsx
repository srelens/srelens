import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { FormEvent, ReactNode } from "react";
import { Popover } from "./Popover";
import { PortalScopeProvider, usePortalHost } from "./portal";

// jsdom has no ResizeObserver, and Radix's popper watches the trigger and the
// content with one. The kit's shared setup does not stub it, and that setup is
// not this file's to edit, so the stub lives here. Inert: jsdom does no layout,
// so there is never a resize to report. (ColumnPicker.test.tsx carries the
// same stub.)
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function setup({
  children = <p>Only failing pods</p>,
  ...props
}: Partial<Parameters<typeof Popover>[0]> = {}) {
  return render(
    <Popover label="Filters" trigger="Open filters" {...props}>
      {children}
    </Popover>,
  );
}

const trigger = () => screen.getByRole("button", { name: "Open filters" });

async function open(props: Partial<Parameters<typeof Popover>[0]> = {}) {
  const view = setup(props);
  await userEvent.click(trigger());
  await screen.findByRole("dialog");
  return view;
}

const panel = () => screen.getByRole("dialog");

/**
 * What this component owns: the API it presents, its wiring to Radix's Popover,
 * and the one structural rule that makes the design's `.popover` survive being
 * placed by a library rather than by hand.
 *
 * Deliberately absent: collision flipping, the shift when a panel meets a
 * viewport edge, outside-click detection, focus movement into the panel and
 * back. The mock hand-wrote all of those and this component exists to stop
 * doing that; asserting them through our component would only pin the version
 * of Radix we happen to have, and jsdom does no layout so the maths could not
 * be checked here anyway. Escape is the exception, as in ColumnPicker: it is
 * the one dismissal path a keyboard user has no alternative to. (#320)
 */
describe("Popover", () => {
  it("keeps the panel shut until the trigger is used", () => {
    setup();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("Only failing pods")).toBeNull();
  });

  it("opens from its trigger", async () => {
    await open();
    expect(screen.getByText("Only failing pods")).toBeDefined();
  });

  it("reports its open state on the trigger", async () => {
    setup();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(trigger());
    await waitFor(() => expect(trigger().getAttribute("aria-expanded")).toBe("true"));
  });

  it("names the panel", async () => {
    // Without a name the panel announces itself as an unlabelled dialog: a
    // screen-reader user is told something opened and not what.
    await open();
    expect(screen.getByRole("dialog", { name: "Filters" })).toBeDefined();
  });

  it("renders the panel in a portal, out of the tree it was declared in", async () => {
    // Not decoration: `.popover` sits inside toolbars and panes that clip, and
    // an anchored panel that renders in place is cut off by the first ancestor
    // with `overflow: hidden`.
    const { container } = await open();
    expect(container.contains(panel())).toBe(false);
  });

  it("closes on Escape", async () => {
    await open();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

/**
 * The mock handed `children` a `close` and `trigger` an `open`. Only one of
 * those is worth carrying: a panel with an Apply or a Clear in it has to be
 * able to dismiss itself, and nothing else can tell it how.
 */
describe("Popover's children", () => {
  it("hands a render prop a working close", async () => {
    render(
      <Popover label="Filters" trigger="Open filters">
        {(close) => (
          <button type="button" onClick={close}>
            Apply
          </button>
        )}
      </Popover>,
    );
    await userEvent.click(trigger());
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("takes a plain node just as happily", async () => {
    // Most panels are content, not a form. Forcing every caller through a
    // render prop, as the mock did, buys them a lambda they never call.
    await open({ children: <p>Only failing pods</p> });
    expect(screen.getByText("Only failing pods")).toBeDefined();
  });

  it("stays open while the panel is used", async () => {
    await open({
      children: (
        <label>
          <input type="checkbox" /> Only failing pods
        </label>
      ),
    });
    await userEvent.click(screen.getByRole("checkbox"));
    expect(screen.queryByRole("dialog")).not.toBeNull();
  });
});

describe("Popover's trigger", () => {
  it("takes its accessible name from what it shows", async () => {
    // The mock put `aria-label={label}` on the trigger as well as the panel,
    // which renames a trigger that already says something — "Namespace:
    // default" read out as "Choose namespace", and unsayable by a speech-input
    // user. The label names the panel; the trigger names itself. (#320)
    setup({ trigger: "Namespace: default", label: "Choose namespace" });
    expect(screen.getByRole("button", { name: "Namespace: default" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Choose namespace" })).toBeNull();
  });

  it("does not submit the form it is standing in", async () => {
    // These stand in toolbars, and a toolbar sits inside a form often enough
    // that a bare <button> submitting on open is a live bug. The kit's Button
    // deliberately does not default the type, so this component sets it.
    const onSubmit = vi.fn((event: FormEvent) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Popover label="Filters" trigger="Open filters">
          <p>Only failing pods</p>
        </Popover>
      </form>,
    );
    expect(trigger().getAttribute("type")).toBe("button");
    await userEvent.click(trigger());
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps a box of its own for the panel to be anchored to", async () => {
    // The mock's trigger wore `display: contents`, which removes the element's
    // box entirely — it measured a separate wrapper div instead. Radix anchors
    // on the trigger itself, so a boxless trigger measures as a zero-sized rect
    // at the origin and the panel is placed against nothing. Structural,
    // because jsdom does no layout. (#320)
    setup();
    expect(trigger().className).not.toContain("contents");
    expect(trigger().className).toContain("inline-flex");
  });
});

describe("Popover's panel", () => {
  it("wears the design's own popover surface, and scrolls", async () => {
    await open();
    expect(panel().className).toContain("popover");
    expect(panel().className).toContain("scroll");
  });

  it("takes a className without losing the surface", async () => {
    await open({ className: "w-[268px]" });
    expect(panel().className).toContain("popover");
    expect(panel().className).toContain("w-[268px]");
  });

  it("leaves the panel in flow so the popper can measure it", async () => {
    // The trap this component exists to close. `.popover` is written for a
    // panel that places itself: `position: fixed`. Radix already fixes and
    // translates a wrapper around the content, and a fixed child leaves that
    // wrapper zero-sized — which is the box the collision logic measures, so
    // the panel flips and shifts against nothing. Relative rather than static,
    // so the stylesheet's `z-index: 45` still applies. Structural, because
    // jsdom does no layout and nothing else would catch a regression. (#320)
    await open();
    expect(panel().style.position).toBe("relative");
  });

  it("is capped at the room the popper found for it", async () => {
    // The mock capped its own height against the viewport, and dropping that
    // on the way to Radix would be a regression: a long panel would run off
    // the screen instead of scrolling. The measurement is the library's now,
    // read back through the variable it publishes.
    await open();
    expect(panel().style.maxHeight).toBe("var(--radix-popover-content-available-height)");
  });
});

/** A tab-sized surface that owns the layers opened inside it, as `TabSurface` does. */
function Surface({ children }: { children: ReactNode }) {
  const { ref, scope } = usePortalHost();
  return (
    <div data-testid="surface">
      <PortalScopeProvider scope={scope}>
        <div data-testid="content">{children}</div>
        <div data-testid="host" ref={ref} />
      </PortalScopeProvider>
    </div>
  );
}

/**
 * The node the layer was portalled into.
 *
 * Radix's `Portal` renders a div of its own as a direct child of the container
 * and the popper adds another inside that, so the container is never the
 * content's parent. Matching the outermost portalled element and taking *its*
 * parent names the container exactly, where "somewhere in the document" would
 * also pass for a container that was wrong but still attached.
 */
function mountedIn(node: Element): Element | null {
  return node.closest("body > *, [data-testid='host'] > *")?.parentElement ?? null;
}

async function openInSurface() {
  const view = render(
    <Surface>
      <Popover label="Filters" trigger="Open filters">
        <p>Only failing pods</p>
      </Popover>
    </Surface>,
  );
  await userEvent.click(trigger());
  await screen.findByRole("dialog");
  return view;
}

/**
 * A panel opened in one tab belongs to that tab.
 *
 * The window is a strip of tabs over one screen each, all of them mounted at
 * once with the inactive ones hidden by the `hidden` attribute — which a portal
 * to `document.body` escapes. So a filter panel opened in one tab stayed on
 * screen over whatever tab the reader moved to. Its anchor went with the tab
 * and it did not. (#357)
 *
 * Nothing else about it changes, deliberately. Radix's Popover is already
 * non-modal, already leaves the window outside it live, and already dismisses
 * on an outside interaction — which is the right answer for a panel, and is why
 * this gets the container and none of the rest of the dialog's treatment.
 */
describe("Popover inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", async () => {
    await openInSurface();
    expect(mountedIn(panel())).toBe(screen.getByTestId("host"));
  });

  it("mounts into the document body when there is no surface", async () => {
    // The fallback the gallery, the frozen classic app and most of this kit's
    // own tests rely on, and it must stay exactly as it was.
    await open();
    expect(mountedIn(panel())).toBe(document.body);
  });

  it("leaves the window outside the tab live, as it always did", async () => {
    // A panel is not a dialog: it takes nothing away from the window around it,
    // so there is nothing here to scope. Pinned so that giving it the container
    // is not mistaken for a licence to give it the rest.
    const chrome = document.createElement("div");
    chrome.innerHTML = "<button>the tab strip</button>";
    document.body.appendChild(chrome);
    try {
      await openInSurface();
      expect(chrome.getAttribute("aria-hidden")).toBeNull();
      expect(document.body.style.pointerEvents).toBe("");
    } finally {
      chrome.remove();
    }
  });

  it("still closes on Escape", async () => {
    await openInSurface();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("still closes on an interaction outside it", async () => {
    const elsewhere = document.createElement("button");
    elsewhere.textContent = "the tab strip";
    document.body.appendChild(elsewhere);
    try {
      await openInSurface();
      await userEvent.click(elsewhere);
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    } finally {
      elsewhere.remove();
    }
  });
});
