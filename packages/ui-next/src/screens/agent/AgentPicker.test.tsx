import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AgentInfo, AgentKind } from "@srelens/core";
import { AgentPicker } from "./AgentPicker";

const agent = (kind: AgentKind, label: string): AgentInfo => ({
  kind,
  label,
  available: true,
  gated: false,
  path: `/${kind}`,
  version: "1",
  installUrl: "",
});

const AGENTS = [agent("srelens", "srelens"), agent("claude", "Claude Code"), agent("codex", "Codex")];

function selectedLabel(kind: AgentKind): string {
  return AGENTS.find((a) => a.kind === kind)?.label ?? "Agent";
}

/** Opens the popover and returns the option rows inside it. */
async function open(selectedKind: AgentKind = "claude", onSelect = vi.fn()) {
  const user = userEvent.setup();
  render(<AgentPicker agents={AGENTS} selectedKind={selectedKind} onSelect={onSelect} />);
  // The trigger is named by what the chip says — `Popover`'s `label` names the
  // PANEL, deliberately, so the two are not read out twice.
  await user.click(screen.getByRole("button", { name: new RegExp(selectedLabel(selectedKind)) }));
  return { user, onSelect, options: await screen.findAllByRole("option") };
}

describe("the agent picker", () => {
  it("offers every agent it was given", async () => {
    const { options } = await open();
    expect(options.map((o) => o.textContent)).toEqual(["srelens", "Claude Code", "Codex"]);
  });

  it("chooses one and closes", async () => {
    const { user, onSelect, options } = await open();
    await user.click(options[2]);
    expect(onSelect).toHaveBeenCalledWith("codex");
    await vi.waitFor(() => {
      expect(screen.queryByRole("option")).toBeNull();
    });
  });

  /**
   * Reported as "use same ones used in the project": this list hand-rolled its
   * own rows — `hover:bg-sunk`, no mark — so it read as a plain white list that
   * did not belong to the app and never said which agent was answering.
   *
   * jsdom computes no layout, so what is pinned is the CLASS that carries the
   * design's row, the same one `WorkspaceSwitcher` and the kit's `PickerRow`
   * wear.
   */
  it("wears the design's own popover row", async () => {
    const { options } = await open();
    for (const option of options) {
      expect(option.className).toContain("ns-row");
    }
  });

  it("marks the agent that is answering, and only that one", async () => {
    const { options } = await open("claude");
    // `data-on` is what `.ns-row` styles; `aria-selected` is what a screen
    // reader is told. Both, or the mark is visible to one reader only.
    expect(options.map((o) => o.getAttribute("data-on"))).toEqual(["false", "true", "false"]);
    expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
  });

  it("keeps the unmarked rows' marks in place, so the labels stay in a column", async () => {
    const { options } = await open("claude");
    // The check is transparent when off, never absent: removing it would shove
    // every other label sideways. One `svg` per row, whichever is chosen.
    for (const option of options) {
      expect(option.querySelectorAll("svg").length).toBe(1);
    }
    // `getAttribute`, not `.className`: on an SVG element that property is an
    // `SVGAnimatedString`, and `toContain` against it passes vacuously.
    expect(options[1].querySelector("svg")?.getAttribute("class")).toContain("opacity-100");
    expect(options[0].querySelector("svg")?.getAttribute("class")).toContain("opacity-0");
  });

  it("does not floor the panel at the kit's 240px", async () => {
    // Written for a panel holding a namespace filter; four short agent names
    // in it left most of the panel empty ("still width is too much").
    await open();
    const panel = screen.getByRole("dialog", { name: "Choose agent" });
    expect(panel.className).toContain("min-w-[7.5rem]");
  });

  it("says why it cannot be used while a question is in flight", () => {
    render(
      <AgentPicker agents={AGENTS} selectedKind="claude" onSelect={vi.fn()} disabled />
    );
    // Still names the agent that is answering — the reader should see which one
    // it is, and see that it is not theirs to change yet.
    expect(screen.getByTitle("Stop the question in flight before switching agent").textContent).toBe(
      "Claude Code",
    );
    // No trigger at all: the chip is a plain span while a turn is in flight.
    expect(screen.queryByRole("button")).toBeNull();
  });
});
