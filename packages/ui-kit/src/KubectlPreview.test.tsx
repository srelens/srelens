import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { KubectlPreview } from "./KubectlPreview";

/** The classic component's tests, carried over. (#318) */
describe("KubectlPreview", () => {
  it("labels and renders the command", () => {
    render(<KubectlPreview command="kubectl get pods web-0 --context prod" />);
    expect(screen.getByText("Equivalent kubectl:")).toBeDefined();
    expect(screen.getByText("kubectl get pods web-0 --context prod")).toBeDefined();
  });

  it("renders a note instead of a command when there's no clean equivalent", () => {
    render(<KubectlPreview note="No single-line kubectl equivalent." />);
    expect(screen.getByText("No single-line kubectl equivalent.")).toBeDefined();
    expect(screen.queryByText("Equivalent kubectl:")).toBeNull();
  });

  it("omits the copy button when no onCopy handler is given", () => {
    render(<KubectlPreview command="kubectl get pods web-0 --context prod" />);
    expect(screen.queryByRole("button", { name: "Copy kubectl command" })).toBeNull();
  });

  it("fires onCopy when the copy button is clicked", () => {
    const onCopy = vi.fn();
    render(<KubectlPreview command="kubectl get pods web-0 --context prod" onCopy={onCopy} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy kubectl command" }));
    expect(onCopy).toHaveBeenCalledTimes(1);
  });

  it("names the copy control for a pointer as well as a screen reader", () => {
    render(<KubectlPreview command="kubectl delete pod web-0" onCopy={() => {}} />);
    const button = screen.getByRole("button", { name: "Copy kubectl command" });
    expect(button.getAttribute("title")).toBe("Copy kubectl command");
  });

  it("hides the copy glyph from assistive tech", () => {
    // The button already carries the name; an unlabelled graphic announced
    // beside it would be read twice.
    const { container } = render(
      <KubectlPreview command="kubectl delete pod web-0" onCopy={() => {}} />,
    );
    const glyph = container.querySelector("svg");
    expect(glyph).not.toBeNull();
    expect(glyph?.getAttribute("aria-hidden")).toBe("true");
  });

  it("gives the copy control an explicit type", () => {
    // This preview lives inside confirm dialogs, which are forms; a bare
    // button submits the one it is standing in, confirming the action the
    // user was only reading about. (#318)
    render(<KubectlPreview command="kubectl delete pod web-0" onCopy={() => {}} />);
    expect(screen.getByRole("button", { name: "Copy kubectl command" }).getAttribute("type")).toBe(
      "button",
    );
  });

  it("renders the command in the design's monospace class", () => {
    const { container } = render(<KubectlPreview command="kubectl get pods" />);
    expect(container.querySelector("code.code")?.textContent).toBe("kubectl get pods");
  });

  it("renders nothing at all when there is neither a command nor a note", () => {
    // An empty paragraph still takes its top margin, so the dialog gets a gap
    // where a preview was declined. (#318)
    const { container } = render(<KubectlPreview />);
    expect(container.innerHTML).toBe("");
  });

  it("renders nothing when the note resolved to false", () => {
    // `note={hasNote && text}` is the ordinary way to make the slot
    // conditional, and it hands over `false`. (#318)
    const { container } = render(<KubectlPreview note={false as unknown as string} />);
    expect(container.innerHTML).toBe("");
  });

  it("forwards className onto the preview", () => {
    const { container } = render(<KubectlPreview command="kubectl get pods" className="extra" />);
    expect(container.querySelector(".extra")).not.toBeNull();
  });

  it("forwards className onto the note form too", () => {
    const { container } = render(<KubectlPreview note="No equivalent." className="extra" />);
    expect(container.querySelector(".extra")?.textContent).toBe("No equivalent.");
  });
});
