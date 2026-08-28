import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { RawError } from "./RawError";

const STRUCT =
  'ApiError: Unauthorized (Status { reason: Some("Unauthorized"), code: Some(401) })';

describe("RawError", () => {
  it("keeps the original message out of sight until it is asked for", () => {
    render(<RawError text={STRUCT} />);
    const details = screen.getByText("Original error").closest("details") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    // In the DOM but not rendered: `details` collapses its own content, which
    // is the whole point — nothing is thrown away and nothing is shouted.
    expect(screen.getByText(STRUCT)).toBeDefined();
  });

  it("is a disclosure the keyboard can reach, not a hover affordance", () => {
    render(<RawError text={STRUCT} />);
    // `summary` is focusable and toggles on Enter/Space by the UA's own
    // behaviour; what this pins is that the text hangs off one, rather than
    // off a hover-only surface a keyboard user never opens.
    expect(screen.getByText("Original error").tagName).toBe("SUMMARY");
  });

  it("never writes the message into an attribute", () => {
    // The rule `PairList` and `KV` settled: a second, unredacted copy of a
    // string must not sit in the markup where nothing on screen says it is
    // there. This component exists BECAUSE `title` is not available. (#331)
    const { container } = render(<RawError text={STRUCT} />);
    for (const node of Array.from(container.querySelectorAll("*"))) {
      for (const attribute of Array.from(node.attributes)) {
        expect(
          attribute.value,
          `${node.tagName}.${attribute.name} carries the message`,
        ).not.toContain(STRUCT);
      }
    }
  });

  it("offers no way to put the message back into an attribute", () => {
    // Guards the guard, the way KV.test.tsx does: an opt-in is a flag someone
    // eventually passes.
    const source = readFileSync(join(__dirname, "RawError.tsx"), "utf8");
    expect(source.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "")).not.toContain("title=");
  });

  it("renders nothing at all when there is no original to show", () => {
    // A bare disclosure that opens onto an empty box is worse than no word:
    // the reader spends a click learning the app has nothing.
    const { container } = render(<RawError text="" />);
    expect(container.innerHTML).toBe("");
  });

  it("takes the caller's word for the disclosure", () => {
    render(<RawError text={STRUCT} label="What the cluster said" />);
    expect(screen.getByText("What the cluster said")).toBeDefined();
  });
});
