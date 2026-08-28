import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { StatusRow } from "./StatusRow";

/**
 * The overview's `NOT READY` list, which is deliberately not a table: no
 * header, no sort, no selection. New with the cluster overview. (#331)
 */
describe("StatusRow", () => {
  it("renders the status word, the name and the trailing facts", () => {
    render(
      <StatusRow status="Degraded" kind="danger" flagged name="checkout-api" facts={["checkout", "9/12"]} />,
    );
    expect(screen.getByText("Degraded")).toBeDefined();
    expect(screen.getByText("checkout-api")).toBeDefined();
    expect(screen.getByText("checkout")).toBeDefined();
    expect(screen.getByText("9/12")).toBeDefined();
  });

  it("keeps the trailing facts in the order it was given them", () => {
    // Namespace then ratio, not ratio then namespace: the caller's order is
    // the design's column order, and a component that sorts or reverses it
    // silently relabels every row.
    const { container } = render(
      <StatusRow status="Pending" kind="warning" flagged name="payments-dlq-drain" facts={["payments", "0/1"]} />,
    );
    const facts = Array.from(container.querySelectorAll(".status-row-fact")).map((el) => el.textContent);
    expect(facts).toEqual(["payments", "0/1"]);
  });

  it("draws no facts box when there are none", () => {
    // An empty wrapper still takes its share of the row's gap, so the row
    // would sit indented against its neighbours for no reason.
    const { container } = render(<StatusRow status="Unknown" kind="neutral" flagged={false} name="orphan" />);
    expect(container.querySelector(".status-row-facts")).toBeNull();
  });

  it("is one activation target whose accessible name covers the status and the name", () => {
    // The rule this row exists to keep: colour is never the only signal. A
    // bare dot beside a separately-linked name announces itself as a link
    // called "checkout-api" with the verdict nowhere in the name, and a
    // reader who cannot see the dot never learns the pod is degraded.
    render(
      <StatusRow
        status="Degraded"
        kind="danger"
        flagged
        name="checkout-api"
        facts={["checkout", "9/12"]}
        onActivate={() => {}}
      />,
    );
    const byStatus = screen.getByRole("button", { name: /Degraded/ });
    const byName = screen.getByRole("button", { name: /checkout-api/ });
    expect(byStatus).toBe(byName);
  });

  it("activates on a click", async () => {
    const onActivate = vi.fn();
    render(
      <StatusRow status="CrashLoopBackOff" kind="danger" flagged name="search-indexer-0" onActivate={onActivate} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /search-indexer-0/ }));
    expect(onActivate).toHaveBeenCalledTimes(1);
  });

  it("does not submit a form it happens to stand in", () => {
    render(<StatusRow status="Degraded" kind="danger" flagged name="checkout-api" onActivate={() => {}} />);
    expect(screen.getByRole("button", { name: /checkout-api/ }).getAttribute("type")).toBe("button");
  });

  it("is not a target at all without an activation", () => {
    // A row that looks pressable and does nothing is worse than a plain one.
    const { container } = render(<StatusRow status="Progressing" kind="warning" flagged name="payments-worker" />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(container.querySelector("button")).toBeNull();
    expect(container.querySelector(".status-row")).not.toBeNull();
  });

  it("composes StatusPill rather than drawing a second dot of its own", () => {
    // A second implementation of the dot/word asymmetry is exactly the
    // duplication this project has spent the most time deleting — eight
    // hand-paired label/tone tables so far.
    const { container } = render(<StatusRow status="Degraded" kind="danger" flagged name="checkout-api" />);
    const pill = container.querySelector(".status-row .status");
    expect(pill).not.toBeNull();
    // Counted across the WHOLE row. Scoped to the inside of the pill
    // (`pill.querySelectorAll(".dot")`) this asserts only that StatusPill
    // draws one dot, which is StatusPill's own test's job, and the defect
    // this test is named for -- a dot drawn by THIS row as a sibling of the
    // pill -- is invisible to it. Mutation-checked: adding a second
    // `<span className="dot" />` to `.status-row-verdict` reddens this line.
    expect(container.querySelectorAll(".status-row .dot").length).toBe(1);
    // The kind reaches the pill, so the pill's own tone table is the only one.
    expect(pill?.getAttribute("data-kind")).toBe("danger");
  });

  it("tones the dot whether the state is bad or not", () => {
    // The dot is the tone's channel and is always coloured; only the word is
    // rationed. A healthy row is a coloured dot beside plain grey text.
    const { container } = render(<StatusRow status="Running" kind="success" flagged={false} name="checkout-api" />);
    const dot = container.querySelector(".status .dot");
    expect(dot?.getAttribute("style")).toContain("var(--ok)");
  });

  it("colours and weights the word only when the state is bad", () => {
    const { container } = render(<StatusRow status="Degraded" kind="danger" flagged name="checkout-api" />);
    const pill = container.querySelector(".status");
    expect(pill?.getAttribute("data-bad")).toBe("true");
    expect(pill?.getAttribute("style")).toContain("var(--sev)");
  });

  it("leaves a healthy word plain", () => {
    const { container } = render(<StatusRow status="Running" kind="success" flagged={false} name="checkout-api" />);
    const pill = container.querySelector(".status");
    expect(pill?.getAttribute("data-bad")).toBeNull();
    expect(pill?.getAttribute("style") ?? "").not.toContain("color");
  });

  it("takes the badness as data rather than deriving it from the tone", () => {
    // Core enumerates the (tone, dot) pairs precisely because badness is not
    // a function of the tone: a running Job is amber and NOT flagged, while a
    // Pending pod is amber and IS. A row that tinted every amber word would
    // shout at the first and be right only by accident.
    const { container } = render(<StatusRow status="Running" kind="warning" flagged={false} name="nightly-report" />);
    expect(container.querySelector(".status")?.getAttribute("data-bad")).toBeNull();
  });

  it("takes no tone, so a caller cannot pair a word with a colour by hand", () => {
    // Compile-time, asserted in prose here and by `tsc` in the kit's
    // typecheck: the API names `kind` (the verdict's vocabulary) and never
    // `tone` (the palette's). The row below is the whole surface.
    const props = { status: "Degraded", kind: "danger", flagged: true, name: "checkout-api" } as const;
    render(<StatusRow {...props} />);
    expect(screen.getByText("Degraded")).toBeDefined();
  });
});

describe("the status row's stylesheet", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("styles the row in the components layer, so a utility still wins", () => {
    expect(components).toContain("  .status-row {");
  });

  it("gives the name the slack and lets it shrink", () => {
    // Without `min-width: 0` a flex item refuses to shrink below its content,
    // so one long pod name pushes the trailing facts off the row.
    const rule = components.slice(components.indexOf("  .status-row-name {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("flex: 1");
    expect(body).toContain("min-width: 0");
  });

  it("gives the verdict a minimum so the names line up down the list", () => {
    // A minimum, never a fixed width: `CrashLoopBackOff` is longer than the
    // column and pushes rather than being cut in half.
    const rule = components.slice(components.indexOf("  .status-row-verdict {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("min-width");
    expect(body).not.toMatch(/[^-]width:/);
  });
});
