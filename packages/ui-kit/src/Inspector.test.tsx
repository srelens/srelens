import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Inspector } from "./Inspector";

const TABS = [
  { id: "details", label: "Details" },
  { id: "containers", label: "Containers" },
  { id: "events", label: "Events" },
];

function setup(props: Partial<Parameters<typeof Inspector>[0]> = {}) {
  const onTabChange = vi.fn();
  const onClose = vi.fn();
  const view = render(
    <Inspector
      name="checkout-api"
      subtitle="Deployment · checkout"
      tabs={TABS}
      activeTab="details"
      onTabChange={onTabChange}
      onClose={onClose}
      {...props}
    >
      {"children" in props ? props.children : <p>pane body</p>}
    </Inspector>,
  );
  return { ...view, onTabChange, onClose };
}

describe("Inspector", () => {
  it("names the subject with a heading", () => {
    // The peek is a section of the page with a title, and the title is the
    // resource. A styled span drops it out of the outline.
    setup();
    expect(screen.getByRole("heading", { level: 2, name: "checkout-api" })).toBeDefined();
  });

  it("is a region named by that heading", () => {
    // So it can nest wherever the caller docks it without inventing a second
    // complementary landmark beside Drawer's.
    setup();
    expect(screen.getByRole("region", { name: "checkout-api" })).toBeDefined();
  });

  it("renders the subtitle line", () => {
    setup();
    expect(screen.getByText("Deployment · checkout")).toBeDefined();
  });

  it("omits the subtitle line when the slot resolved to false", () => {
    const { container } = setup({ subtitle: false });
    expect(container.querySelector("header p")).toBeNull();
  });

  it("shows the status as words, not only as a colour", () => {
    setup({ status: "Degraded", statusKind: "danger" });
    expect(screen.getByText("Degraded")).toBeDefined();
  });

  it("colours the status word when the state is bad", () => {
    // Frame A's red `Degraded` is the most distinctive thing in the mock's
    // header, and it is the whole reason `tinted` exists. Fixed here rather
    // than asked of the caller, for the same reason the segmented strip is:
    // Inspector IS the frame the design draws. And it can only ever colour
    // danger and warning, since `tinted` is a no-op for every other kind.
    const { container } = setup({ status: "Degraded", statusKind: "danger" });
    const pill = container.querySelector(".status") as HTMLElement;
    expect(pill.style.color).toBe("var(--sev)");
    expect(pill.getAttribute("data-bad")).toBe("true");
  });

  it("leaves a healthy status word plain", () => {
    // Frame B: `Running` beside an ok dot is ordinary ink, not green.
    const { container } = setup({ status: "Running", statusKind: "success" });
    const pill = container.querySelector(".status") as HTMLElement;
    expect(pill.style.color).toBe("");
    expect(pill.getAttribute("data-bad")).toBeNull();
  });

  it("draws one status dot, not a pill nested in a pill", () => {
    // `status` is a ReactNode, so a screen that could not reach the colouring
    // rule would have had to pass its own <StatusPill tinted> — which nests
    // two pills and draws two dots. Reaching it through the prop is the point.
    const { container } = setup({ status: "Degraded", statusKind: "danger" });
    expect(container.querySelectorAll(".status").length).toBe(1);
    expect(container.querySelectorAll(".status .dot").length).toBe(1);
  });

  it("does not key a fact by a label two of them can share", () => {
    // "Ready" is a plausible label twice over on one subject — pod readiness
    // and container readiness. Keyed by label, React warns and the two rows
    // become one another's reconciliation target.
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const { container } = setup({
      facts: [
        { label: "Ready", value: "9/12 ready" },
        { label: "Ready", value: "3/4 containers" },
      ],
    });
    expect(errors.mock.calls.flat().join(" ")).not.toContain("same key");
    expect(Array.from(container.querySelectorAll("dd")).map((e) => e.textContent)).toEqual([
      "9/12 ready",
      "3/4 containers",
    ]);
    errors.mockRestore();
  });

  it("reads the facts as bare figures", () => {
    // The mock's header line is `Degraded  9/12 ready  84d` — the figures on
    // their own, with the word that names them folded into the value where it
    // is wanted at all. The kit argued for labelled pairs and was overruled by
    // the user, whose design this is. (#331)
    const { container } = setup({
      facts: [
        { label: "Ready", value: "9/12 ready" },
        { label: "Age", value: "84d" },
      ],
    });
    const visible = Array.from(container.querySelectorAll("dd")).map((e) => e.textContent);
    expect(visible).toEqual(["9/12 ready", "84d"]);
    expect(Array.from(container.querySelectorAll("dt")).every((e) => e.className.includes("sr-only"))).toBe(true);
  });

  it("keeps the label for anyone who cannot see which column the figure came from", () => {
    // Bare on screen, paired in the markup: a `dt` beside its `dd` is what
    // makes "84d" mean an age to a screen reader. Losing that was the reason
    // the kit resisted the mock, and it is the part that does not have to go.
    setup({ facts: [{ label: "Age", value: "84d" }] });
    const age = screen.getByText("Age");
    expect(age.tagName).toBe("DT");
    expect(age.nextElementSibling?.tagName).toBe("DD");
    expect(age.nextElementSibling?.textContent).toBe("84d");
  });

  it("keeps a toned fact readable without its colour", () => {
    // Tone is emphasis. The label and the figure carry the meaning, so a
    // reader who never sees the red still learns there are 7 restarts.
    setup({ facts: [{ label: "Restarts", value: "7 restarts", tone: "sev" as const }] });
    expect(screen.getByText("7 restarts").textContent).toBe("7 restarts");
    expect(screen.getByText("Restarts")).toBeDefined();
  });

  it("makes the subject's name the largest thing on the pane", () => {
    // ~20px in the mock, against the 12px the rest of the header runs at. The
    // peek is read name-first, and at 14px it was the same size as the tabs.
    setup();
    expect(screen.getByRole("heading", { level: 2, name: "checkout-api" }).className).toContain("text-[1.25rem]");
  });

  it("gives a fact normal ink and leaves muting to a tone", () => {
    // The design's line is `Degraded  9/12 ready  84d`: the ratio in ordinary
    // ink, only the age quiet. Rendered muted throughout — which is what the
    // header's `.path` would have done — there is no way for the screen to
    // draw the difference at all.
    const { container } = setup({
      facts: [
        { label: "Ready", value: "9/12 ready" },
        { label: "Age", value: "84d", tone: "muted" as const },
      ],
    });
    const [ready, age] = Array.from(container.querySelectorAll<HTMLElement>("dd"));
    expect(ready.className).toContain("fact");
    expect(ready.className).not.toContain("path");
    expect(ready.style.color).toBe("");
    expect(age.style.color).toBe("var(--ink-muted)");
  });

  it("names no colour of its own for a fact's tone", () => {
    const { container } = setup({ facts: [{ label: "Restarts", value: "7", tone: "sev" as const }] });
    const styled = container.querySelector<HTMLElement>("dd[style]");
    expect(styled?.style.color).toContain("var(--sev)");
  });

  it("gives the flagged marker a name instead of leaving it a red dot", () => {
    setup({ flagged: true });
    expect(screen.getByText("Needs attention")).toBeDefined();
  });

  it("shows no flag marker when the subject is not flagged", () => {
    setup();
    expect(screen.queryByText("Needs attention")).toBeNull();
  });

/**
 * The flag dot and the status word are two channels on one fact, and they used
 * to be able to disagree: the dot was hard-coded to the severity tone whatever
 * `statusKind` said, so an amber subject got a red dot beside an amber word.
 *
 * Not an exotic pairing. core's `k8sStatus.ts` defines
 * `UNSETTLED = { health: "warning", flagged: true }` and returns it for any
 * warning-health workload as well as for a cordoned-yet-Ready Node, so a
 * mid-rollout Deployment — the very thing the user's frame A shows — is the
 * ordinary path through it, not the edge.
 */
describe("Inspector's flag dot", () => {
  function flagDot(container: HTMLElement) {
    return container.querySelector<HTMLElement>('[data-slot="inspector-flag"]');
  }

  it("takes the tone the status carries, not always the severity one", () => {
    const { container } = setup({ flagged: true, status: "Progressing", statusKind: "warning" });
    expect(flagDot(container)?.style.background).toBe("var(--warn)");
  });

  it("is the severity tone when the status is a danger", () => {
    const { container } = setup({ flagged: true, status: "Degraded", statusKind: "danger" });
    expect(flagDot(container)?.style.background).toBe("var(--sev)");
  });

  it("never disagrees with the dot inside the status pill", () => {
    // The whole point: one fact, two channels, one colour. Read off both
    // rather than asserted twice, so the two cannot drift apart.
    for (const kind of ["success", "warning", "danger", "info", "neutral"] as const) {
      const { container, unmount } = setup({ flagged: true, status: "x", statusKind: kind });
      const pillDot = container.querySelector<HTMLElement>(".status .dot");
      expect(flagDot(container)?.style.background, `${kind} should match the pill`).toBe(
        pillDot?.style.background,
      );
      unmount();
    }
  });

  it("keeps meaning severity when there is no status to echo", () => {
    // A flag with no `statusKind` has nothing to take its colour from, and a
    // muted "needs attention" dot is a worse answer than the red it replaced.
    const { container } = setup({ flagged: true });
    expect(flagDot(container)?.style.background).toBe("var(--sev)");
  });

  it("names no colour of its own for the dot", () => {
    const { container } = setup({ flagged: true, statusKind: "warning" });
    expect(flagDot(container)?.style.background).toContain("var(--");
  });

  it("still says one thing that is true at every severity", () => {
    // The dot is aria-hidden, so this text is the only channel a screen
    // reader gets — and it now stands in for amber as well as red. The
    // severity itself is announced by the status word beside it.
    const { container } = setup({ flagged: true, status: "Progressing", statusKind: "warning" });
    expect(screen.getByText("Needs attention")).toBeDefined();
    expect(container.textContent).toContain("Progressing");
    for (const word of ["critical", "failed", "error", "red", "danger"]) {
      expect(
        "Needs attention".toLowerCase(),
        `the default label should not claim ${word} of an amber subject`,
      ).not.toContain(word);
    }
  });

  it("leaves the dot out entirely when the subject is not flagged, however bad", () => {
    // `flagged` decides whether there is a dot; `statusKind` only decides its
    // colour. A healthy-looking pane must not sprout one from a danger word.
    const { container } = setup({ status: "Degraded", statusKind: "danger" });
    expect(flagDot(container)).toBeNull();
  });
});

  it("renders the caller's header actions", () => {
    setup({ actions: <button type="button">Open tab</button> });
    expect(screen.getByRole("button", { name: "Open tab" })).toBeDefined();
  });

  it("closes from the header button", async () => {
    const { onClose } = setup();
    await userEvent.click(screen.getByRole("button", { name: "Close inspector" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("omits the close button when the caller owns closing", () => {
    // Docked inside a Drawer there is already one, and two are a fault.
    setup({ onClose: undefined });
    expect(screen.queryByRole("button", { name: "Close inspector" })).toBeNull();
  });

  it("gives every button it owns an explicit type", () => {
    // A bare button inside a form submits it. The kit's Button deliberately
    // does not default `type`, so each component sets its own.
    const { container } = setup();
    const buttons = [...container.querySelectorAll("button")];
    expect(buttons.length).toBeGreaterThan(0);
    expect(buttons.every((b) => b.getAttribute("type") === "button")).toBe(true);
  });

  it("switches panes through the kit's tab strip", async () => {
    const { onTabChange } = setup();
    await userEvent.click(screen.getByRole("tab", { name: "Containers" }));
    expect(onTabChange).toHaveBeenCalledWith("containers");
  });

  it("carries the tab strip's keyboard contract rather than a second one", async () => {
    // Delegated to Tabs; asserted here so a rewrite into plain buttons is
    // caught rather than silently losing the arrow keys.
    const { onTabChange } = setup();
    screen.getByRole("tab", { name: "Details" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onTabChange).toHaveBeenLastCalledWith("containers");
  });

  it("draws its panes as the mock's segmented control", () => {
    // The peek is the frame the mock draws, so the variant is fixed here
    // rather than asked of every caller: five panes as a rounded outlined
    // container with the active one a raised pill.
    const { container } = setup();
    expect(container.querySelector(".seg")).not.toBeNull();
    expect(container.querySelector(".tabstrip")).toBeNull();
  });

  it("names the tab strip when the caller says what the panes are", () => {
    setup({ tabsLabel: "Resource views" });
    expect(screen.getByRole("tablist", { name: "Resource views" })).toBeDefined();
  });

  it("renders no tab strip at all when there are no panes", () => {
    setup({ tabs: [] });
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByText("pane body")).toBeDefined();
  });

  it("renders the body as the panel for the active tab", () => {
    setup();
    const panel = screen.getByRole("tabpanel", { name: "Details" });
    expect(panel.textContent).toContain("pane body");
  });

  it("lets the keyboard reach the scrolling body", () => {
    setup();
    expect(screen.getByRole("tabpanel", { name: "Details" }).getAttribute("tabindex")).toBe("0");
  });

  it("says so when the active pane has nothing in it", () => {
    setup({ children: null });
    expect(screen.getByText("Nothing to show")).toBeDefined();
  });

  it("takes the caller's wording for the empty pane", () => {
    setup({ children: [], emptyLabel: "No events in the last hour" });
    expect(screen.getByText("No events in the last hour")).toBeDefined();
  });

  it("treats a slot that resolved to false as empty", () => {
    setup({ children: false });
    expect(screen.getByText("Nothing to show")).toBeDefined();
  });

  it("renders the footer when there is one", () => {
    setup({ footer: <button type="button">Ask</button> });
    expect(screen.getByRole("button", { name: "Ask" })).toBeDefined();
  });

  it("omits the footer band entirely when there is nothing in it", () => {
    // A ruled empty strip is a visible artefact, not a no-op.
    const { container } = setup({ footer: false });
    expect(container.querySelector("footer")).toBeNull();
  });

  it("forwards className onto the pane", () => {
    const { container } = setup({ className: "extra" });
    expect(container.querySelector(".pane.extra")).not.toBeNull();
  });
});

/**
 * Escape backs out of the peek. Handled from inside the panel rather than on
 * the window: a component that listens globally fights every other thing that
 * does, which is the problem Drawer keeps a stack to solve.
 */
describe("Inspector keyboard behaviour", () => {
  it("closes on Escape from inside the panel", async () => {
    const { onClose } = setup();
    screen.getByRole("button", { name: "Close inspector" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("leaves Escape alone in an editable field", async () => {
    // A filter box inside a pane owns its own Escape — clearing the field
    // should not also close the panel around it.
    const { onClose } = setup({ children: <input aria-label="Filter" /> });
    screen.getByRole("textbox", { name: "Filter" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("marks Escape handled so an outer panel does not also close", async () => {
    const onOuterKeyDown = vi.fn();
    const onClose = vi.fn();
    render(
      <div onKeyDown={onOuterKeyDown}>
        <Inspector name="checkout-api" tabs={TABS} activeTab="details" onClose={onClose}>
          body
        </Inspector>
      </div>,
    );
    screen.getByRole("button", { name: "Close inspector" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onOuterKeyDown.mock.calls[0][0].defaultPrevented).toBe(true);
  });

  it("does not swallow Escape when it has no way to close", async () => {
    const onOuterKeyDown = vi.fn();
    render(
      <div onKeyDown={onOuterKeyDown}>
        <Inspector name="checkout-api" tabs={TABS} activeTab="details">
          body
        </Inspector>
      </div>,
    );
    screen.getByRole("tab", { name: "Details" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(onOuterKeyDown.mock.calls[0][0].defaultPrevented).toBe(false);
  });
});

/**
 * The kit's doc comments are the record of why each component looks as it
 * does, and the header's shape was argued out in one of them before the user
 * supplied the mock. A comment left arguing against the code beneath it is
 * worse than no comment: the next reader takes it for the current decision.
 */
describe("the header's figures", () => {
  it("style themselves from the components layer, not from utilities", () => {
    // `.fact` sets a colour, and the header puts Tailwind utilities on the
    // elements around it. Landing in the utilities layer would put it in a
    // race with those instead of underneath them, and the loser would be
    // whichever the bundler happened to emit second.
    const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
    const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));
    expect(components).toContain("\n  .fact {");
  });
});

describe("Inspector's own record", () => {
  const source = readFileSync(join(__dirname, "Inspector.tsx"), "utf8");

  it("records that the bare figures were the user's call, not the kit's", () => {
    expect(source).toMatch(/overrul/i);
  });

  it("no longer argues that a bare figure is unreadable", () => {
    expect(source).not.toContain("is unreadable to anyone who");
  });

  it("does not call two different things 'the mock'", () => {
    // The early paragraphs mean the design-system HTML this component was
    // ported from; the header paragraph means the user's screenshots. One
    // word for both makes the record unreadable to the next person.
    expect(source).not.toMatch(/\bthe mock\b/);
  });
});
