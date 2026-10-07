import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CopyNameButton } from "./CopyNameButton";

const NAME = "kps-kube-prometheus-stack-operator-66976c8d6-9j6td";

/** Stand a clipboard up, or take it away — jsdom has none of its own. */
function withClipboard(clipboard: { writeText: (text: string) => Promise<void> } | undefined) {
  Object.defineProperty(navigator, "clipboard", { value: clipboard, configurable: true });
}

let writeText = vi.fn(async (_text: string): Promise<void> => {});

beforeEach(() => {
  writeText = vi.fn(async (_text: string): Promise<void> => {});
});

afterEach(() => {
  withClipboard(undefined);
});

/** Click it the way a reader does. `userEvent.setup()` installs a clipboard of
 *  its own, so the one under test is put in place after it. */
async function press() {
  const user = userEvent.setup();
  withClipboard({ writeText });
  await user.click(screen.getByRole("button", { name: `Copy name ${NAME}` }));
}

describe("CopyNameButton", () => {
  it("is named for the thing it copies, not just Copy", () => {
    render(<CopyNameButton name={NAME} />);
    expect(screen.getByRole("button", { name: `Copy name ${NAME}` })).toBeDefined();
  });

  it("puts the bare name on the clipboard — no kind, no namespace, nothing around it", async () => {
    render(<CopyNameButton name={NAME} />);
    await press();

    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith(NAME);
  });

  it("says so once it has copied, to a screen reader and on the button", async () => {
    render(<CopyNameButton name={NAME} />);
    await press();

    expect(await screen.findByRole("status")).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Copied to clipboard");
    const button = screen.getByRole("button", { name: `Copy name ${NAME}` });
    expect(button.title).toBe("Copied");
    // What keeps the hover-revealed control on screen after the pointer left.
    expect(button.className).toContain("copy-ok");
  });

  it("says it failed when the clipboard refuses, rather than showing a check", async () => {
    writeText = vi.fn(async (_text: string): Promise<void> => {
      throw new Error("denied");
    });
    render(<CopyNameButton name={NAME} />);
    await press();

    expect((await screen.findByRole("status")).textContent).toBe("Could not copy to clipboard");
    expect(screen.getByRole("button", { name: `Copy name ${NAME}` }).title).toBe("Copy failed");
  });

  it("says it failed when there is no clipboard at all — web mode on a plain-http origin", async () => {
    render(<CopyNameButton name={NAME} />);
    const user = userEvent.setup();
    withClipboard(undefined);
    await user.click(screen.getByRole("button", { name: `Copy name ${NAME}` }));

    // Not "Copied" over an empty clipboard, which is what an optional-chained
    // write would have reported.
    expect((await screen.findByRole("status")).textContent).toBe("Could not copy to clipboard");
  });

  it("copies the name it is showing now, after the pane moves to another subject", async () => {
    const view = render(<CopyNameButton name={NAME} />);
    view.rerender(<CopyNameButton name="web-0" />);
    const user = userEvent.setup();
    withClipboard({ writeText });
    await user.click(screen.getByRole("button", { name: "Copy name web-0" }));
    expect(writeText).toHaveBeenCalledWith("web-0");
  });
});
