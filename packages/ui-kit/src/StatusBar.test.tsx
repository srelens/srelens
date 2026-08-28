import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { StatusBar, type StatusSegment } from "./StatusBar";
import { toneColor } from "./tone";

function Cable({ className }: { className?: string }) {
  return (
    <svg data-testid="cable" className={className} viewBox="0 0 24 24">
      <path d="M4 4h16" />
    </svg>
  );
}

const SEGMENTS: StatusSegment[] = [
  { id: "cluster", label: "prod-us-east", dot: true, tone: "ok", onSelect: () => {} },
  { id: "version", label: "eks 1.29" },
  { id: "offline", label: "3 offline", tone: "warn", onSelect: () => {} },
];

const END: StatusSegment[] = [
  { id: "forwards", label: "4 forwards", icon: Cable, onSelect: () => {} },
  { id: "index", label: "indexing 1 284", busy: true },
];

function setup(props: Partial<Parameters<typeof StatusBar>[0]> = {}) {
  return render(<StatusBar segments={SEGMENTS} end={END} {...props} />);
}

const bar = () => screen.getByRole("group", { name: "Status" });
const seg = (name: string) => screen.getByText(name).closest(".status-seg") as HTMLElement;

/**
 * The strip along the bottom of the window: a row of small readouts, some of
 * which are also the way in to what they are reporting on.
 *
 * The mock's version reached into six stores for its figures and hard-coded the
 * rest. None of that can come along — the kit is not allowed to know what a
 * cluster or a port-forward is — so every readout arrives as a segment and the
 * component is left owning what a segment looks like, which of them can be
 * pressed, and how a segment says what it means. That last one is the whole
 * point of the port: the mock leaned on colour for it. (#320)
 */
describe("StatusBar", () => {
  it("shows every readout it is given", () => {
    setup();
    for (const label of ["prod-us-east", "eks 1.29", "3 offline", "4 forwards", "indexing 1 284"]) {
      expect(screen.getByText(label)).toBeDefined();
    }
  });

  it("names the strip so it can be found, without announcing itself on every change", () => {
    // A group, not `role="status"`: a live region here would read out every
    // count, every reconnect and every indexing tick as it happened.
    setup();
    expect(bar().className).toContain("statusbar");
  });

  it("takes a name of the caller's choosing", () => {
    setup({ label: "Cluster status" });
    expect(screen.getByRole("group", { name: "Cluster status" })).toBeDefined();
  });

  it("presses only the segments that do something", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: "3 offline" }));
    expect(screen.queryByRole("button", { name: "eks 1.29" })).toBeNull();
    expect(seg("eks 1.29").tagName).toBe("SPAN");
  });

  it("calls back the segment that was pressed", () => {
    const onSelect = vi.fn();
    setup({ segments: [{ id: "lock", label: "lock", onSelect }], end: [] });
    fireEvent.click(screen.getByRole("button", { name: "lock" }));
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it("explains a segment on hover when there is more to say", () => {
    setup({ segments: [{ id: "lock", label: "lock", title: "Lock workspace  ⌘⇧L", onSelect: () => {} }], end: [] });
    expect(screen.getByRole("button", { name: "lock" }).title).toBe("Lock workspace  ⌘⇧L");
  });

  it("pushes the trailing readouts to the far end", () => {
    // Structural, because jsdom does no layout: the two groups are separated by
    // a growing spacer, which is how the design splits the strip.
    setup();
    const spacer = bar().querySelector(".flex-1") as HTMLElement;
    expect(spacer).not.toBeNull();
    expect(spacer.getAttribute("aria-hidden")).toBe("true");
    expect(spacer.compareDocumentPosition(seg("3 offline")) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
    expect(spacer.compareDocumentPosition(seg("4 forwards")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("leaves out the spacer when nothing is pinned to the far end", () => {
    setup({ end: [] });
    expect(bar().querySelector(".flex-1")).toBeNull();
  });
});

/**
 * The bug this branch has now found four times. Both halves of it: a control
 * that submits the form it happens to be standing in, and a control with
 * nothing to call it by.
 */
describe("StatusBar's pressable segments", () => {
  it("are all type=button", () => {
    setup();
    const buttons = within(bar()).getAllByRole("button") as HTMLButtonElement[];
    expect(buttons.length).toBeGreaterThan(0);
    for (const button of buttons) expect(button.type).toBe("button");
  });

  it("all have a name, because the label is text and is required", () => {
    // Structural rather than a convention: a segment cannot be icon-only, so
    // there is no shape of this component that produces a nameless button.
    setup();
    for (const button of within(bar()).getAllByRole("button")) {
      expect(button.textContent?.trim()).not.toBe("");
    }
  });

  it("keeps the icon out of the name", () => {
    setup();
    const button = screen.getByRole("button", { name: "4 forwards" });
    expect(within(button).getByTestId("cable").closest("[aria-hidden='true']")).not.toBeNull();
  });
});

/**
 * The mock said several things in colour and nothing else: `style={{ color:
 * "var(--warn)" }}` on a segment, a dot filled from a value in the data. A
 * colour-blind user sees an ordinary readout, and a screen reader sees one too.
 */
describe("StatusBar never says it in colour alone", () => {
  it("tints from a tone, over words that already carry the meaning", () => {
    setup();
    const offline = seg("3 offline");
    expect(offline.style.color).toBe(toneColor("warn"));
    expect(offline.textContent).toContain("3 offline");
  });

  it("leaves an untoned segment to the stylesheet", () => {
    setup();
    expect(seg("eks 1.29").style.color).toBe("");
  });

  it("treats the dot as a second channel, not a message", () => {
    setup();
    const dot = seg("prod-us-east").querySelector("[data-dot]") as HTMLElement;
    expect(dot).not.toBeNull();
    expect(dot.getAttribute("aria-hidden")).toBe("true");
    expect(dot.style.background).toBe(toneColor("ok"));
  });

  it("gives a dot without a tone the neutral colour rather than none", () => {
    setup({ segments: [{ id: "watch", label: "watching", dot: true }], end: [] });
    const dot = seg("watching").querySelector("[data-dot]") as HTMLElement;
    expect(dot.style.background).toBe(toneColor("muted"));
  });

  it("pulses a dot only when asked", () => {
    setup({
      segments: [
        { id: "live", label: "live", tone: "ok", dot: true, pulse: true },
        { id: "still", label: "dropped", tone: "sev", dot: true },
      ],
      end: [],
    });
    expect(seg("live").querySelector("[data-dot]")?.className).toContain("live-dot");
    expect(seg("dropped").querySelector("[data-dot]")?.className).not.toContain("live-dot");
  });
});

describe("StatusBar's work-in-progress segment", () => {
  it("says it is busy to assistive technology, not only by spinning", () => {
    setup();
    const indexing = seg("indexing 1 284");
    expect(indexing.getAttribute("aria-busy")).toBe("true");
    // The spinner is the visual half. Left out of the accessible name, which
    // otherwise reads "Loading indexing 1 284" — the kit's Spinner names
    // itself, and two names on one readout is one too many.
    const spinner = indexing.querySelector("svg") as SVGElement;
    expect(spinner.getAttribute("aria-hidden")).toBe("true");
  });

  it("is not busy otherwise", () => {
    setup();
    expect(seg("eks 1.29").getAttribute("aria-busy")).toBeNull();
  });
});

describe("StatusBar's quieter detail", () => {
  it("hangs a suffix off the label without letting it become the label", () => {
    setup({ segments: [{ id: "watch", label: "live", detail: "· 2 drops" }], end: [] });
    const watch = seg("live");
    expect(watch.textContent).toBe("live· 2 drops");
    expect(screen.getByText("· 2 drops").className).toContain("opacity-60");
  });

  it("renders no suffix element when the detail is empty", () => {
    // `filled`, not a null check: `detail={drops && \`· ${drops}\`}` is how a
    // caller makes it conditional, and that hands over `false` or `0`.
    setup({ segments: [{ id: "watch", label: "live", detail: "" }], end: [] });
    expect(seg("live").textContent).toBe("live");
  });
});

describe("the strip at a narrow window", () => {
  // MEASURED, not estimated. The worst-case set this strip can carry — a
  // 20-character cluster name, a version, the link word, a dead forward, a
  // plural forward count, live shells, a failed helm operation and a running
  // one, and `Ask` — was rendered in the real app at the window's own 960px
  // minimum (`minWidth` in apps/desktop/src-tauri/tauri.conf.json). The strip's
  // `clientWidth` was 960 and its `scrollWidth` 1036: 76px over. `Ask` ran from
  // 1005px to 1036px, entirely past the right edge, and `elementFromPoint` at
  // its own midpoint did not find it — the way into the console was not merely
  // ugly but unclickable.
  //
  // It could not be scrolled to either: `.statusbar` declared no overflow, so
  // the overflow escaped to the document, where `body { overflow: hidden }`
  // clipped it. So the strip carries its own overflow, the way `.tabstrip`
  // already does for the same reason.
  //
  // Asserted on the stylesheet because that is where the policy lives and
  // jsdom does no layout.
  const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  // Matched to the rule's own closing brace at its own indentation, not to the
  // first `}` in the text: the rule carries a comment that quotes CSS, and a
  // `[^}]*` scan stops inside it and reports the whole declaration missing.
  const body = /\n {2}\.statusbar \{[\s\S]*?\n {2}\}/.exec(css)?.[0] ?? "";

  it("scrolls rather than clipping the segments it cannot fit", () => {
    expect(body, "the .statusbar rule should exist").toBeTruthy();
    expect(body).toContain("overflow-x: auto");
  });

  it("does not spend the strip's 22px height on a scrollbar", () => {
    // Shorter than the tab strip's 33px, so a visible bar would leave less
    // room for the readouts than the bar itself takes.
    expect(body).toContain("scrollbar-width: none");
    expect(css).toContain(".statusbar::-webkit-scrollbar");
  });
});
