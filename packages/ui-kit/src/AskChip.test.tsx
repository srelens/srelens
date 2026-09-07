import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AskChip } from "./AskChip";

/**
 * The chip is one button, and nearly everything worth testing about it comes
 * from where it stands: inside a clickable row, inside a form, inside a list of
 * fifty identical-looking copies of itself. (#320)
 */
describe("AskChip", () => {
  it("shows its label", () => {
    render(<AskChip question="Why is this pod restarting?" onAsk={() => {}} />);
    expect(screen.getByRole("button", { name: /Ask/ })).toBeDefined();
  });

  it("hands the question over when clicked", async () => {
    const onAsk = vi.fn();
    render(<AskChip question="Why is this pod restarting?" onAsk={onAsk} />);
    await userEvent.click(screen.getByRole("button", { name: /Ask/ }));
    expect(onAsk).toHaveBeenCalledWith("Why is this pod restarting?");
  });

  it("is a button, not a submit button", () => {
    // A bare <button> inside a form submits it. These sit on rows of tables
    // that stand inside forms.
    render(<AskChip question="Why?" onAsk={() => {}} />);
    expect(screen.getByRole("button", { name: /Ask/ }).getAttribute("type")).toBe("button");
  });

  it("does not submit the form it is standing in", async () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <AskChip question="Why?" onAsk={() => {}} />
      </form>,
    );
    await userEvent.click(screen.getByRole("button", { name: /Ask/ }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("names itself by its question, not by the word on it", () => {
    // Every row on screen has one of these, and "Ask, Ask, Ask, Ask" tells a
    // screen-reader user which row they are on: none of them.
    render(<AskChip question="Why is this pod restarting?" onAsk={() => {}} />);
    expect(
      screen.getByRole("button", { name: "Ask: Why is this pod restarting?" }),
    ).toBeDefined();
  });

  it("keeps the visible word short", () => {
    render(<AskChip question="Why is this pod restarting?" onAsk={() => {}} />);
    expect(screen.getByRole("button", { name: /Ask/ }).textContent).toBe("Ask");
  });

  it("takes a different word for a different kind of question", () => {
    render(<AskChip question="What changed?" label="Explain" onAsk={() => {}} />);
    const chip = screen.getByRole("button", { name: "Explain: What changed?" });
    expect(chip.textContent).toBe("Explain");
  });

  it("keeps the click off the row it is sitting on", async () => {
    // The row opens a detail pane. Asking about the row is not selecting it.
    const onRowClick = vi.fn();
    const onAsk = vi.fn();
    render(
      <div onClick={onRowClick}>
        <AskChip question="Why?" onAsk={onAsk} />
      </div>,
    );
    await userEvent.click(screen.getByRole("button", { name: /Ask/ }));
    expect(onAsk).toHaveBeenCalled();
    expect(onRowClick).not.toHaveBeenCalled();
  });

  it("keeps the keyboard activation off the row too", async () => {
    // A list that moves its selection on keydown would otherwise act on the
    // same Enter that asked the question.
    const onRowKeyDown = vi.fn();
    const onAsk = vi.fn();
    render(
      <div onKeyDown={onRowKeyDown}>
        <AskChip question="Why?" onAsk={onAsk} />
      </div>,
    );
    await userEvent.tab();
    await userEvent.keyboard("{Enter}");
    expect(onAsk).toHaveBeenCalledWith("Why?");
    expect(onRowKeyDown).not.toHaveBeenCalled();
  });

  it("shows itself when it is focused", async () => {
    // `.row-ask` is invisible until its row is hovered, and a keyboard user
    // cannot hover. A focusable control nobody can see is a trap.
    render(<AskChip question="Why?" onAsk={() => {}} />);
    await userEvent.tab();
    const chip = screen.getByRole("button", { name: /Ask/ });
    expect(chip).toBe(document.activeElement);
    expect(chip.className).toContain("focus-visible:opacity-100");
  });

  it("renders nothing when there is no question", () => {
    // A chip with nothing to ask is a control that does nothing, revealed on
    // hover, taking a tab stop.
    const { container } = render(<AskChip question="" onAsk={() => {}} />);
    expect(container.querySelector("button")).toBeNull();
  });

  it("renders nothing for a question that is only whitespace", () => {
    const { container } = render(<AskChip question="   " onAsk={() => {}} />);
    expect(container.querySelector("button")).toBeNull();
  });

  it("can be disabled while an earlier question is still running", async () => {
    const onAsk = vi.fn();
    render(<AskChip question="Why?" onAsk={onAsk} disabled />);
    const chip = screen.getByRole("button", { name: /Ask/ }) as HTMLButtonElement;
    expect(chip.disabled).toBe(true);
    await userEvent.click(chip);
    expect(onAsk).not.toHaveBeenCalled();
  });

  it("wears the design's chip class", () => {
    const { container } = render(<AskChip question="Why?" onAsk={() => {}} />);
    expect(container.querySelector(".row-ask")).not.toBeNull();
  });

  it("forwards className onto the chip", () => {
    const { container } = render(<AskChip question="Why?" onAsk={() => {}} className="extra" />);
    expect(container.querySelector(".row-ask.extra")).not.toBeNull();
  });
});
