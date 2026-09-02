import { describe, it, expect, vi, afterEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { CopyCommand } from "./CopyCommand";

const COMMAND = "kubectl --context prod-eu get widgets.example.com -A -o wide";

/** jsdom ships no clipboard at all, so there is nothing to spy on. */
function stubClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

const copyButton = () => screen.getByRole("button", { name: /^Cop/ });

afterEach(() => {
  vi.useRealTimers();
});

describe("CopyCommand", () => {
  it("shows the command in the code face", () => {
    const { container } = render(<CopyCommand command={COMMAND} />);
    const code = container.querySelector("code");

    expect(code?.textContent).toBe(COMMAND);
    expect(code?.className).toContain("code");
  });

  it("copies the command and says so, then offers to do it again", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    stubClipboard(writeText);
    vi.useFakeTimers();

    render(<CopyCommand command={COMMAND} />);
    // `fireEvent` rather than `userEvent`: the latter installs a clipboard stub
    // of its own and drives its pointer sequence off timers, and this test has
    // replaced one and frozen the other.
    await act(async () => {
      fireEvent.click(copyButton());
    });

    expect(writeText).toHaveBeenCalledWith(COMMAND);
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();

    // A label that never comes back is a control the reader thinks is spent.
    await act(async () => {
      vi.advanceTimersByTime(1400);
    });
    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined();
  });

  it("marks the confirmation with an ok-toned check, and only once copied", async () => {
    stubClipboard(vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined));

    const { container } = render(<CopyCommand command={COMMAND} />);
    expect(container.querySelector(".copy-command-check")).toBeNull();

    await act(async () => {
      fireEvent.click(copyButton());
    });
    expect(container.querySelector(".copy-command-check")).not.toBeNull();
  });

  it("leaves the command readable on a machine with no clipboard", async () => {
    // A non-secure origin has no `navigator.clipboard`. The command is still
    // the thing the reader came for: it stays on screen and selectable, and
    // the button does not claim to have copied anything.
    stubClipboard(vi.fn<(text: string) => Promise<void>>().mockRejectedValue(new Error("insecure")));

    const { container } = render(<CopyCommand command={COMMAND} />);
    await act(async () => {
      fireEvent.click(copyButton());
    });

    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined();
    expect(container.querySelector("code")?.textContent).toBe(COMMAND);
  });

  it("puts no second copy of the command in a title", () => {
    // The disclosure hole `PairList` and `KV` both had removed: a `title` is an
    // unredacted duplicate sitting in the DOM, and it is never what truncation
    // needs anyway.
    const { container } = render(<CopyCommand command={COMMAND} />);
    expect(container.querySelectorAll("[title]").length).toBe(0);
  });

  it("does not announce the command as an equivalent to anything", () => {
    // `KubectlPreview`'s "Equivalent kubectl:" belongs beside an action the app
    // is about to perform. Here the command IS the content.
    const { container } = render(<CopyCommand command={COMMAND} />);
    expect(container.textContent).not.toContain("Equivalent");
  });
});

describe("the copy command's stylesheet", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("styles it in the components layer, so a utility still wins", () => {
    expect(components).toContain("  .copy-command {");
    expect(components).toContain("  .copy-command-text {");
    expect(components).toContain("  .copy-command-check {");
  });

  it("wraps the command rather than truncating it", () => {
    // A command you cannot finish reading is not one you can retype, and this
    // lands in a 264px rail. `KubectlPreview` reached the same finding for the
    // same reason; the alternative is a `title`, which is the hole above.
    const rule = components.slice(components.indexOf("  .copy-command-text {"));
    expect(rule.slice(0, rule.indexOf("}"))).toContain("overflow-wrap: anywhere");
    expect(rule.slice(0, rule.indexOf("}"))).not.toContain("text-overflow");
  });

  it("takes the confirmation's colour from the ok token", () => {
    const rule = components.slice(components.indexOf("  .copy-command-check {"));
    expect(rule.slice(0, rule.indexOf("}"))).toContain("var(--ok)");
  });
});
