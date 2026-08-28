import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Mark } from "./Mark";
import type { IconComponent } from "./IconButton";

/** A stand-in for whatever glyph the app hands over; see NavIcon. */
const Glyph: IconComponent = ({ size, className, "aria-hidden": hidden }) => (
  <svg data-testid="glyph" width={size} height={size} className={className} aria-hidden={hidden} />
);

const mark = (container: HTMLElement) =>
  container.querySelector('[data-slot="chip-mark"]') as HTMLElement;
const badge = (container: HTMLElement) =>
  container.querySelector('[data-slot="chip-badge"]') as HTMLElement | null;

/**
 * The mock's version took a resolved cluster override straight out of the app's
 * hotbar store and looked its symbol up in a lucide map. What is left after
 * both of those go is a mark: a name, a short text, a colour, and at most one
 * of a glyph or an image. These cover that, plus the four states the mock had
 * no answer for — nothing to draw, an image that fails, a chip with no name,
 * and a badge with no text to put in it. (#320)
 */
describe("Mark", () => {
  it("draws the short text as the mark, uppercased", () => {
    render(<Mark name="prod-eu-west" short="pew" />);
    expect(screen.getByText("PEW")).toBeDefined();
  });

  it("derives initials from the name when no short text is given", () => {
    render(<Mark name="prod eu west" />);
    expect(screen.getByText("PE")).toBeDefined();
  });

  it("derives initials from a single-word name by taking its first letters", () => {
    render(<Mark name="staging" />);
    expect(screen.getByText("ST")).toBeDefined();
  });

  it("splits a name on the separators a context name actually uses", () => {
    render(<Mark name="gke_acme-prod" />);
    expect(screen.getByText("GA")).toBeDefined();
  });

  it("caps the short text at three characters", () => {
    // Longer than three and the mark's text spills out of a 26px square.
    render(<Mark name="whatever" short="abcdef" />);
    expect(screen.getByText("ABC")).toBeDefined();
  });

  it("names the mark for assistive technology", () => {
    render(<Mark name="prod-eu-west" short="pew" />);
    expect(screen.getByRole("img", { name: "prod-eu-west" })).toBeDefined();
  });

  it("goes silent when the surrounding control already says the name", () => {
    const { container } = render(<Mark name="prod-eu-west" decorative />);
    expect(screen.queryByRole("img")).toBeNull();
    expect(container.firstElementChild?.getAttribute("aria-hidden")).toBe("true");
  });

  it("does not claim to be an image when it has nothing to say", () => {
    // A blank name with no short text leaves an unnamed role="img", which is
    // announced as "image" and tells the listener nothing at all.
    const { container } = render(<Mark name="   " />);
    expect(screen.queryByRole("img")).toBeNull();
    expect(container.firstElementChild?.getAttribute("aria-hidden")).toBe("true");
  });

  it("falls back to the short text for a name when the name is blank", () => {
    render(<Mark name="" short="pew" />);
    expect(screen.getByRole("img", { name: "PEW" })).toBeDefined();
  });

  it("draws a glyph instead of the initials, hidden from assistive technology", () => {
    // The initials still ride under it as the badge — that is what the badge is
    // for — so the claim is about the mark itself, not the whole chip.
    const { container } = render(<Mark name="prod" icon={Glyph} />);
    expect(screen.getByTestId("glyph").getAttribute("aria-hidden")).toBe("true");
    expect(mark(container).textContent).toBe("");
  });

  it("draws an image ahead of a glyph, with an empty alt", () => {
    const { container } = render(<Mark name="prod" icon={Glyph} imageSrc="data:image/png;base64,AA" />);
    const img = container.querySelector("img") as HTMLImageElement;
    expect(img.getAttribute("alt")).toBe("");
    expect(screen.queryByTestId("glyph")).toBeNull();
  });

  it("ignores an empty image source rather than rendering a broken image", () => {
    const { container } = render(<Mark name="prod" imageSrc="" />);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText("PR")).toBeDefined();
  });

  it("falls back to the mark underneath when the image fails to load", () => {
    // A stored data URL can be truncated, and a remote one can 404; the mock
    // drew the browser's broken-image glyph inside the rail and kept it there.
    const { container } = render(<Mark name="prod" icon={Glyph} imageSrc="/gone.png" />);
    fireEvent.error(container.querySelector("img") as HTMLImageElement);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByTestId("glyph")).toBeDefined();
  });

  it("draws a later valid image after an earlier one failed", () => {
    // The failure belongs to the source that failed, not to the component: the
    // editor keeps three marks mounted across every edit, so a corrupt image
    // picked once would otherwise strand the preview on the fallback until the
    // dialog closed, however many good images followed.
    const { container, rerender } = render(<Mark name="prod" icon={Glyph} imageSrc="/gone.png" />);
    fireEvent.error(container.querySelector("img") as HTMLImageElement);
    expect(container.querySelector("img")).toBeNull();

    rerender(<Mark name="prod" icon={Glyph} imageSrc="/good.png" />);
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/good.png");
  });

  it("keeps falling back for the source that failed, across an unrelated edit", () => {
    // The other half of the pair above: re-rendering must not un-fail an image
    // that is still the one that could not load.
    const { container, rerender } = render(<Mark name="prod" icon={Glyph} imageSrc="/gone.png" />);
    fireEvent.error(container.querySelector("img") as HTMLImageElement);

    rerender(<Mark name="prod" short="xy" icon={Glyph} imageSrc="/gone.png" />);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByTestId("glyph")).toBeDefined();
  });

  it("rides the short text along the bottom of a glyph mark", () => {
    const { container } = render(<Mark name="prod-eu-west" short="pew" icon={Glyph} />);
    expect(badge(container)?.textContent).toBe("PEW");
  });

  it("omits the badge on a text mark, which is already the short text", () => {
    const { container } = render(<Mark name="prod-eu-west" short="pew" />);
    expect(mark(container).textContent).toBe("PEW");
    expect(badge(container)).toBeNull();
  });

  it("omits the badge when the caller turns it off", () => {
    const { container } = render(
      <Mark name="prod-eu-west" short="pew" icon={Glyph} withBadge={false} />,
    );
    expect(mark(container)).not.toBeNull();
    expect(badge(container)).toBeNull();
  });

  it("omits the badge when there is no text to put in it", () => {
    // An empty badge is a floating rounded smudge under the mark.
    const { container } = render(<Mark name="   " icon={Glyph} />);
    expect(mark(container)).not.toBeNull();
    expect(badge(container)).toBeNull();
  });

  it("sizes the box from the size, not from a class", () => {
    const { container, rerender } = render(<Mark name="prod" size="sm" />);
    expect((container.firstElementChild as HTMLElement).style.width).toBe("26px");
    rerender(<Mark name="prod" size="lg" />);
    expect((container.firstElementChild as HTMLElement).style.height).toBe("38px");
  });

  it("fills the mark with the caller's colour and takes a token by default", () => {
    const { container, rerender } = render(<Mark name="prod" />);
    expect(mark(container).style.background).toContain("--accent");
    rerender(<Mark name="prod" color="var(--info)" />);
    expect(mark(container).style.background).toBe("var(--info)");
  });

  it("draws the mark's text in a token, never a fixed white", () => {
    // The mock wrote `#fff` and `text-white` on the glyph and the initials.
    const { container } = render(<Mark name="prod" />);
    expect(mark(container).style.color).toMatch(/^var\(--/);
  });

  it("rings the active chip in its own colour", () => {
    const { container } = render(<Mark name="prod" color="var(--info)" active />);
    expect(mark(container).style.boxShadow).toContain("var(--info)");
  });

  it("leaves the selected state to the control that owns it", () => {
    // The ring is presentation. Nothing here is pressable, so a chip that
    // announced itself as selected would be announcing a state it does not own.
    render(<Mark name="prod" active />);
    const chip = screen.getByRole("img", { name: "prod" });
    expect(chip.getAttribute("data-active")).toBe("true");
    expect(chip.getAttribute("aria-pressed")).toBeNull();
    expect(chip.getAttribute("aria-current")).toBeNull();
  });

  it("puts no title anywhere: nothing here is hoverable by keyboard or touch", () => {
    const { container } = render(<Mark name="prod-eu-west" short="pew" icon={Glyph} />);
    expect(badge(container)).not.toBeNull();
    expect(container.querySelector("[title]")).toBeNull();
  });

  it("forwards className onto the chip", () => {
    const { container } = render(<Mark name="prod" className="extra" />);
    expect(container.querySelector(".extra")).not.toBeNull();
    expect(container.querySelector(".extra")).toBe(container.firstElementChild);
  });
});
