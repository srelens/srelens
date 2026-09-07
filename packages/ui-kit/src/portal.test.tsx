import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { useRef, useState, type ReactNode } from "react";
import { PortalScopeProvider, useOpenLayer, usePortalContainer, usePortalHost } from "./portal";

/** Reports what a portalled component would be told to mount into. */
function Probe() {
  const container = usePortalContainer();
  return <span data-testid="probe">{container?.dataset.name ?? "none"}</span>;
}

/** Stands in for a dialog: holds the scope open for as long as it is mounted. */
function Layer({ id = "layer" }: { id?: string }) {
  const { container, scoped, showing } = useOpenLayer();
  const where = `${scoped ? "scoped" : "loose"}:${container?.dataset.name ?? "none"}`;
  return <span data-testid={id}>{`${where}:${showing ? "showing" : "hidden"}`}</span>;
}

/** A surface that hosts its own layers, the way TabSurface does. */
function Host({ name, visible = true, children }: { name: string; visible?: boolean; children: ReactNode }) {
  const { ref, layered, scope } = usePortalHost(visible);
  return (
    <PortalScopeProvider scope={scope}>
      <div data-testid={`content-${name}`} inert={layered}>
        {children}
      </div>
      <div data-name={name} ref={ref} />
    </PortalScopeProvider>
  );
}

/** Opens and closes layers inside one host, from outside it. */
function Stage({ name = "a" }: { name?: string }) {
  const [open, setOpen] = useState(0);
  return (
    <>
      <button type="button" onClick={() => setOpen((n) => n + 1)}>open</button>
      <button type="button" onClick={() => setOpen((n) => Math.max(0, n - 1))}>close</button>
      <Host name={name}>
        {Array.from({ length: open }, (_, i) => <Layer key={i} id={`layer-${i}`} />)}
      </Host>
    </>
  );
}

/** Drives the registration by hand, to reach the cases a component cannot. */
function Rig() {
  const { ref, layered, scope } = usePortalHost();
  const first = useRef<(() => void) | null>(null);
  return (
    <PortalScopeProvider scope={scope}>
      <div data-testid="content-r" inert={layered} />
      <div data-name="r" ref={ref} />
      <button type="button" onClick={() => { first.current = scope.hold(); }}>hold one</button>
      <button type="button" onClick={() => { scope.hold(); }}>hold two</button>
      <button type="button" onClick={() => first.current?.()}>release one</button>
    </PortalScopeProvider>
  );
}

const content = (name = "a") => screen.getByTestId(`content-${name}`);

/**
 * The seam that lets a portalled layer belong to one part of the window rather
 * than to the whole document.
 *
 * Optional on purpose, and that is the load-bearing part: the gallery, the
 * frozen classic app and most tests render a dialog with no surface around it
 * at all, and every one of them has to keep working. No scope means no
 * container, which is exactly what Radix's `container` prop wants in order to
 * fall back to `document.body`. (#357)
 */
describe("a portal scope", () => {
  it("names no container outside a scope, so Radix falls back to document.body", () => {
    render(<Probe />);
    expect(screen.getByTestId("probe").textContent).toBe("none");
  });

  it("names the surface's own node inside one", () => {
    render(<Host name="a"><Probe /></Host>);
    expect(screen.getByTestId("probe").textContent).toBe("a");
  });

  it("tells a layer whether it is scoped, which is not the same question as where to mount", () => {
    // A layer needs both: the container says where to render, and `scoped`
    // says whether there is a surface to be modal *within*. They differ during
    // the render before the host's node exists, and a layer that read only the
    // container could not tell that case from having no surface at all.
    render(<Layer />);
    expect(screen.getByTestId("layer").textContent).toBe("loose:none:showing");
  });

  it("tells a layer when its surface is off screen", () => {
    // A tab that is hidden rather than unmounted keeps its layers mounted with
    // it, and a layer that cannot tell it is off screen will answer key
    // presses meant for the tab the reader is actually looking at.
    render(<Host name="a" visible={false}><Layer /></Host>);
    expect(screen.getByTestId("layer").textContent).toBe("scoped:a:hidden");
  });

  it("keeps each surface's layers to itself", () => {
    render(
      <>
        <Host name="a"><Layer id="in-a" /></Host>
        <Host name="b"><Layer id="in-b" /></Host>
      </>,
    );
    expect(screen.getByTestId("in-a").textContent).toBe("scoped:a:showing");
    expect(screen.getByTestId("in-b").textContent).toBe("scoped:b:showing");
  });
});

/**
 * The count exists so the surface can make its own content unreachable while a
 * layer covers it. The overlay stops the pointer; nothing else stops Tab or an
 * assistive technology's own cursor, and `inert` is the one attribute that
 * means all three.
 */
describe("a portal scope's open-layer count", () => {
  it("starts clear", () => {
    render(<Stage />);
    expect(content().hasAttribute("inert")).toBe(false);
  });

  it("marks the surface while a layer is open", () => {
    render(<Stage />);
    fireEvent.click(screen.getByRole("button", { name: "open" }));
    expect(content().hasAttribute("inert")).toBe(true);
  });

  it("clears the surface when the layer closes", () => {
    render(<Stage />);
    fireEvent.click(screen.getByRole("button", { name: "open" }));
    fireEvent.click(screen.getByRole("button", { name: "close" }));
    expect(content().hasAttribute("inert")).toBe(false);
  });

  it("holds until the last of several layers closes", () => {
    // A dialog that opens a second dialog: releasing on the first close would
    // hand the covered screen back while it is still covered.
    render(<Stage />);
    fireEvent.click(screen.getByRole("button", { name: "open" }));
    fireEvent.click(screen.getByRole("button", { name: "open" }));
    fireEvent.click(screen.getByRole("button", { name: "close" }));
    expect(content().hasAttribute("inert")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "close" }));
    expect(content().hasAttribute("inert")).toBe(false);
  });

  it("ignores a release called twice, so another layer still holds the surface", () => {
    // The count is shared and the release is handed out, so a caller that
    // releases twice — a cleanup run again, an effect double-invoked — would
    // hand the covered screen back while a second dialog was still covering
    // it. jsdom cannot show that as a reachable button; the count is the thing
    // that can be pinned.
    render(<Rig />);
    fireEvent.click(screen.getByRole("button", { name: "hold one" }));
    fireEvent.click(screen.getByRole("button", { name: "hold two" }));
    fireEvent.click(screen.getByRole("button", { name: "release one" }));
    fireEvent.click(screen.getByRole("button", { name: "release one" }));
    expect(screen.getByTestId("content-r").hasAttribute("inert")).toBe(true);
  });

  it("counts only its own surface's layers", () => {
    render(
      <>
        <Stage name="a" />
        <Host name="b"><span /></Host>
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "open" }));
    expect(content("a").hasAttribute("inert")).toBe(true);
    expect(content("b").hasAttribute("inert")).toBe(false);
  });
});
