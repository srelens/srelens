import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
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

  it("does not carry a confirmation over to the next subject's name (PR #835 review)", async () => {
    // The peek stays mounted from row to row. "Copied" beside `web-2` while
    // the clipboard holds the previous name is a confirmation of something
    // that did not happen.
    const view = render(<CopyNameButton name={NAME} />);
    await press();
    expect((await screen.findByRole("status")).textContent).toBe("Copied to clipboard");

    view.rerender(<CopyNameButton name="web-2" />);

    const next = screen.getByRole("button", { name: "Copy name web-2" });
    expect(next.title).not.toBe("Copied");
    expect(next.className).not.toContain("copy-ok");
    expect(screen.queryByRole("status")).toBeNull();
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith(NAME);
  });

  it("does not confirm on the next subject a copy that was still in flight when the pane moved on", async () => {
    let land!: () => void;
    writeText = vi.fn(
      (_text: string): Promise<void> =>
        new Promise((resolve) => {
          land = resolve;
        }),
    );
    const view = render(<CopyNameButton name={NAME} />);
    await press();

    view.rerender(<CopyNameButton name="web-2" />);
    await act(async () => land());

    const next = screen.getByRole("button", { name: "Copy name web-2" });
    expect(next.title).not.toBe("Copied");
    expect(next.className).not.toContain("copy-ok");
    expect(screen.queryByRole("status")).toBeNull();
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
