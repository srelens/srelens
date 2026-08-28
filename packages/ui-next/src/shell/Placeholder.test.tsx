import type { SubmitEvent } from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Placeholder } from "./Placeholder";

describe("Placeholder", () => {
  it("is a titled screen for the route, not a blank pane", () => {
    // The parent spec: a route with no ported screen must still be a routed,
    // titled, reachable screen, because users find it on their first session.
    render(<Placeholder route="/k/pods" clusterName="prod" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByRole("heading", { level: 1, name: "Pods" })).toBeDefined();
  });

  it("still titles a restored route whose name will not decode", () => {
    // The render path for the whole class of bug: `parseTab` accepts any string
    // as a route, so a corrupted or legacy `/resources/%zz` comes back out of
    // storage on every launch, `screenFor` has no screen for it, and this is
    // what renders it. `describe` throwing a `URIError` here is the window
    // failing to boot, not one bad tab.
    render(<Placeholder route="/resources/%zz" clusterName="prod" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByRole("heading", { level: 1, name: "%zz" })).toBeDefined();
  });

  it("still titles a restored /edit route whose subject will not decode", () => {
    render(<Placeholder route="/edit/%zz" clusterName="prod" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByRole("heading", { level: 1, name: "Edit %zz" })).toBeDefined();
  });

  it("says this screen is not in the new design yet", () => {
    render(<Placeholder route="/helm" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByText(/not in the new design yet/i)).toBeDefined();
  });

  it("offers to open the same place in the classic design", () => {
    const onOpenInClassic = vi.fn();
    render(<Placeholder route="/helm" ported={[]} onOpenInClassic={onOpenInClassic} />);
    fireEvent.click(screen.getByRole("button", { name: /open in classic/i }));
    // The tab's cluster rides along when the tab has one, so classic can
    // reopen at that cluster; a cluster-less placeholder passes nothing.
    expect(onOpenInClassic).toHaveBeenCalledWith("/helm", undefined);
  });

  it("lists which screens are ported, when any are", () => {
    render(<Placeholder route="/helm" ported={["Application log", "Release notes"]} onOpenInClassic={() => {}} />);
    expect(screen.getByText(/Application log/)).toBeDefined();
    expect(screen.getByText(/Release notes/)).toBeDefined();
  });

  it("says none are ported yet rather than showing an empty list", () => {
    render(<Placeholder route="/helm" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.getByText(/no screens are in the new design yet/i)).toBeDefined();
    expect(document.querySelector("ul")).toBeNull();
  });

  it("offers a way into the component gallery when one is given", () => {
    // #327: the gallery's only affordance used to live on the old root page,
    // and went with it. The Placeholder is where it belongs now — see the
    // component's doc comment.
    const onOpenGallery = vi.fn();
    render(
      <Placeholder route="/helm" ported={[]} onOpenInClassic={() => {}} onOpenGallery={onOpenGallery} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /component gallery/i }));
    expect(onOpenGallery).toHaveBeenCalledTimes(1);
  });

  it("does not offer the gallery when there is nowhere to send them", () => {
    // The kit's own gallery renders Placeholders; a button to the gallery
    // from inside the gallery would be a loop.
    render(<Placeholder route="/helm" ported={[]} onOpenInClassic={() => {}} />);
    expect(screen.queryByRole("button", { name: /component gallery/i })).toBeNull();
  });

  it("does not submit a form it is standing in", () => {
    const onSubmit = vi.fn((e: SubmitEvent<HTMLFormElement>) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Placeholder route="/helm" ported={[]} onOpenInClassic={() => {}} onOpenGallery={() => {}} />
      </form>,
    );
    fireEvent.click(screen.getByRole("button", { name: /open in classic/i }));
    fireEvent.click(screen.getByRole("button", { name: /component gallery/i }));
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
