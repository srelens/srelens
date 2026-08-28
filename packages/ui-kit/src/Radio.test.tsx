import { describe, it, expect, vi, beforeAll, afterAll } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Radio } from "./Radio";

/**
 * Arrow-key movement inside a radio group is the browser's, and user-event
 * emulates it by looking the group up with `CSS.escape`. jsdom 24 ships no
 * `window.CSS` at all, so the emulation throws before it reaches the component.
 * The gap is the environment's, and the behaviour is the whole reason this is a
 * native input, so fill it rather than leave it uncovered.
 */
const nativeCSS = (globalThis as { CSS?: unknown }).CSS;
beforeAll(() => {
  (globalThis as { CSS?: unknown }).CSS ??= {
    escape: (value: string) => value.replace(/[^\w-]/g, (char) => `\\${char}`),
  };
});
afterAll(() => {
  (globalThis as { CSS?: unknown }).CSS = nativeCSS;
});

describe("Radio", () => {
  it("renders the label and the hint", () => {
    render(<Radio checked name="interval" label="Every 30 seconds" hint="Refreshes in the background" onChange={() => {}} />);
    expect(screen.getByRole("radio", { name: "Every 30 seconds" })).toBeDefined();
    expect(screen.getByText("Refreshes in the background")).toBeDefined();
  });

  it("is named by its label alone, not by the label and the hint", () => {
    // The mock wrapped both in one `<label>`, which folds the explanation into
    // the option's name. The hint is a description. (#320)
    render(<Radio checked name="interval" label="Every 30 seconds" hint="Refreshes in the background" onChange={() => {}} />);
    const radio = screen.getByRole("radio", { name: "Every 30 seconds" });
    const describedBy = radio.getAttribute("aria-describedby");
    expect(describedBy).not.toBeNull();
    expect(document.getElementById(describedBy!)?.textContent).toBe("Refreshes in the background");
  });

  it("omits the hint cleanly when there is none", () => {
    const { container } = render(<Radio checked={false} name="interval" label="Manual" onChange={() => {}} />);
    expect(container.querySelector(".text-muted")).toBeNull();
    expect(screen.getByRole("radio", { name: "Manual" }).getAttribute("aria-describedby")).toBeNull();
  });

  it("omits the hint when the slot resolved to false", () => {
    // Not `hint != null`: `hint={verbose && "..."}` hands over `false`, which
    // renders nothing but still buys a wrapper and its line of space.
    const { container } = render(<Radio checked={false} name="interval" label="Manual" hint={false} onChange={() => {}} />);
    expect(screen.getByRole("radio", { name: "Manual" })).toBeDefined();
    expect(container.querySelector(".text-muted")).toBeNull();
  });

  it("reports the change on click", async () => {
    const onChange = vi.fn();
    render(<Radio checked={false} name="interval" label="Manual" onChange={onChange} />);
    await userEvent.click(screen.getByRole("radio", { name: "Manual" }));
    expect(onChange).toHaveBeenCalled();
  });

  it("is selected by clicking its label text", async () => {
    const onChange = vi.fn();
    render(<Radio checked={false} name="interval" label="Manual" onChange={onChange} />);
    await userEvent.click(screen.getByText("Manual"));
    expect(onChange).toHaveBeenCalled();
  });

  it("is selected from the keyboard", async () => {
    const onChange = vi.fn();
    render(<Radio checked={false} name="interval" label="Manual" onChange={onChange} />);
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Manual" }));
    await userEvent.keyboard(" ");
    expect(onChange).toHaveBeenCalled();
  });

  it("makes a whole group one tab stop, landing on the chosen option", async () => {
    // The reason for a native input. Nothing here arranges this: the browser
    // does it for every input sharing a `name`. (#320)
    render(
      <>
        <Radio checked={false} name="interval" label="Manual" onChange={() => {}} />
        <Radio checked name="interval" label="Every 30 seconds" onChange={() => {}} />
        <button type="button">After</button>
      </>,
    );
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Every 30 seconds" }));
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "After" }));
  });

  it("moves between the options of a group with the arrow keys", async () => {
    const onEvery = vi.fn();
    render(
      <>
        <Radio checked name="interval" label="Manual" onChange={() => {}} />
        <Radio checked={false} name="interval" label="Every 30 seconds" onChange={onEvery} />
      </>,
    );
    screen.getByRole("radio", { name: "Manual" }).focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(onEvery).toHaveBeenCalled();
  });

  it("blocks the change when disabled", async () => {
    const onChange = vi.fn();
    render(<Radio checked={false} name="interval" label="Manual" onChange={onChange} disabled />);
    const radio = screen.getByRole("radio", { name: "Manual" }) as HTMLInputElement;
    expect(radio.disabled).toBe(true);
    await userEvent.click(radio);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("puts every option of a group under one name", () => {
    render(<Radio checked={false} name="interval" label="Manual" onChange={() => {}} />);
    expect(screen.getByRole("radio", { name: "Manual" }).getAttribute("name")).toBe("interval");
  });

  it("forwards className onto the row", () => {
    const { container } = render(<Radio checked={false} name="interval" label="Manual" className="extra" onChange={() => {}} />);
    expect(container.querySelector(".extra")).not.toBeNull();
  });
});
