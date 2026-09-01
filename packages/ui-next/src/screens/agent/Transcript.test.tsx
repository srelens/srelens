import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Transcript } from "./Transcript";
import type { Turn } from "../../lib/agentRun";

const turn = (over: Partial<Turn> = {}): Turn => ({
  id: 1,
  role: "agent",
  text: "",
  calls: [],
  at: 0,
  ...over,
});

// `gates` is a required prop (Ruling A/D: gates live on `AgentRun`, not on
// `Turn`, so every render below supplies its own — empty unless a test is
// specifically about one).
describe("a run's transcript", () => {
  // I7: `ui-kit`'s `ConsoleDock` already wraps its children in one
  // `role="log"` live region — this component nesting a second one inside it
  // announces inconsistently and often twice. `live` defaults true, which is
  // right for `/agent`, where this is the screen's only region.
  it("declares its own live region by default", () => {
    render(<Transcript turns={[turn()]} gates={[]} />);
    const region = screen.getByRole("log");
    expect(region.getAttribute("aria-live")).toBe("polite");
  });

  it("declares no live region of its own when told it is nested inside one", () => {
    render(<Transcript turns={[turn()]} gates={[]} live={false} />);
    expect(screen.queryByRole("log")).toBeNull();
  });

  it("draws a tool call's duration only once it has one", () => {
    render(
      <Transcript
        turns={[turn({ calls: [{ id: "t", tool: "k8s.listPods", args: {}, status: null }] })]}
        gates={[]}
      />,
    );
    expect(screen.queryByText(/ms/)).toBeNull();
  });

  it("draws the duration srelens measured when it has one", () => {
    render(
      <Transcript
        turns={[turn({ calls: [{ id: "t", tool: "k8s.listPods", args: {}, status: "ok", ms: 41 }] })]}
        gates={[]}
      />,
    );
    expect(screen.getByText(/41\s*ms/)).toBeTruthy();
  });

  it("renders an answer as elements, never as raw markdown syntax", () => {
    render(<Transcript turns={[turn({ text: "**pool** exhausted" })]} gates={[]} />);
    expect(screen.queryByText(/\*\*/)).toBeNull();
    expect(screen.getByText("pool")).toBeTruthy();
  });

  // C2 (review round 1): one fixture exercising every `MdBlock` kind, each
  // asserted by its own distinctive text so deleting (or `null`-ing) any one
  // branch of `Block` fails only that assertion, not the whole suite.
  it("renders every markdown block kind the parser can produce", () => {
    const md = [
      "# Heading text",
      "",
      "Paragraph text",
      "",
      "- Bullet text",
      "",
      "1. Ordered text",
      "",
      "| A | B |",
      "| - | - |",
      "| Cell1 | Cell2 |",
      "",
      "```",
      "code text",
      "```",
    ].join("\n");
    render(<Transcript turns={[turn({ text: md })]} gates={[]} />);
    expect(screen.getByText("Heading text")).toBeTruthy();
    expect(screen.getByText("Paragraph text")).toBeTruthy();
    expect(screen.getByText("Bullet text").closest("ul")).toBeTruthy();
    expect(screen.getByText("Ordered text").closest("ol")).toBeTruthy();
    expect(screen.getByText("Cell1").closest("table")).toBeTruthy();
    expect(screen.getByText("code text").closest("pre")).toBeTruthy();
  });

  // Ruling K (review round 2): classic tiers a heading's class by level
  // (`AssistantMarkdown.tsx:62-70`); a flattened single class, or the branch
  // deleted outright so headings fall to the paragraph path, must both fail
  // this — asserting text presence alone let a deleted `heading` branch pass
  // every other assertion in the suite.
  it("gives a heading's level its own weight, not one style for every level", () => {
    const md = ["# H1 text", "", "### H3 text"].join("\n");
    render(<Transcript turns={[turn({ text: md })]} gates={[]} />);
    const h1 = screen.getByText("H1 text").closest("p");
    const h3 = screen.getByText("H3 text").closest("p");
    expect(h1).toBeTruthy();
    expect(h3).toBeTruthy();
    expect(h1!.className).not.toBe(h3!.className);
    expect(h1!.className).toContain("text-base");
    expect(h3!.className).toContain("text-sm");
  });

  it("omits the thoughts row entirely for an agent that streamed none", () => {
    render(<Transcript turns={[turn({ text: "done" })]} gates={[]} />);
    expect(screen.queryByText(/thought/i)).toBeNull();
  });

  // C1 (review round 1): the prior version of this test asserted only the
  // ABSENCE of Approve/Deny, which a reviewer showed stays green even with
  // gate rendering disabled outright. It must also assert decision 1's
  // record — the capability name and the outcome badge — actually renders.
  it("shows a gate as a record, with no second set of answer buttons", () => {
    render(
      <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "pending" }]} />,
    );
    expect(screen.getByText("k8s.scale")).toBeTruthy();
    expect(screen.getByText("Pending")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /approve/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /deny/i })).toBeNull();
  });

  // Ruling I: `GateRecord.at` is optional — stamped by `AgentConsent` at
  // resolution (Task 9), never fabricated here.
  it("shows the outcome word alone until a resolution time is stamped", () => {
    render(
      <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "approved" }]} />,
    );
    expect(screen.getByText("Applied")).toBeTruthy();
  });

  // P2 (#392 review): the fourth outcome. A gate the BACKEND says stopped
  // waiting, where srelens is not the one who answered it.
  it("says a gate is no longer waiting, without claiming how it ended", () => {
    const at = new Date(2024, 0, 1, 14, 6).getTime();
    render(
      <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "settled", at }]} />,
    );
    // The word and the time are two elements now — a badge and a clock —
    // because the card printed the word twice when they were one string.
    expect(screen.getByText("No longer waiting")).toBeTruthy();
    expect(screen.getByText("14:06:00")).toBeTruthy();
    // `mcp://confirm-resolved` carries an id and nothing else, so any of these
    // would be srelens reporting a fact it was never told.
    expect(screen.queryByText(/timed out|expired|denied|applied/i)).toBeNull();
  });

  it("adds the resolution time once one is stamped", () => {
    const at = new Date(2024, 0, 1, 14, 6).getTime();
    render(
      <Transcript
        turns={[turn()]}
        gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "approved", at }]}
      />,
    );
    expect(screen.getByText("Applied")).toBeTruthy();
    expect(screen.getByText("14:06:00")).toBeTruthy();
  });

  describe("the gate card, from §A.1's mock", () => {
    it("shows the arguments in full, as JSON, because they are what is being approved", () => {
      render(
        <Transcript
          turns={[turn()]}
          gates={[{ id: "g", tool: "k8s.scale", args: { namespace: "checkout", replicas: 40 }, outcome: "pending" }]}
        />,
      );
      // Not `summarizeArgs`'s one-liner: a gate is the one place a reader is
      // asked to say yes, and a truncated row is not enough to say it to.
      const block = screen.getByText(/"replicas": 40/);
      expect(block.textContent).toContain('"namespace": "checkout"');
    });

    it("marks a destructive capability, from the registry rather than the name", () => {
      // `destructive` is a real field on the capability catalog, so this is a
      // fact and not a guess about what the tool sounds like.
      render(
        <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.deletePod", args: {}, outcome: "pending" }]} />,
      );
      expect(screen.getByText("destructive")).toBeTruthy();
    });

    it("does not mark one that is not", () => {
      render(
        <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "pending" }]} />,
      );
      expect(screen.queryByText("destructive")).toBeNull();
    });

    it("still offers no way to answer, and claims nothing about what the call will do", () => {
      render(
        <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.deletePod", args: {}, outcome: "pending" }]} />,
      );
      // §A.1 draws `Review and run` / `Deny` / `Ask first` here. They are NOT
      // here: `AgentConsent` is the only thing that answers, and a second set
      // rebuilds exactly the stale-prompt bug that decision 1 removed.
      expect(screen.queryByRole("button", { name: /review and run|deny|ask first/i })).toBeNull();
      // And no effect paragraph: `ConfirmRequest` is `{ id, tool, args }`
      // (#388), so any sentence about what the call would do is invented.
      expect(screen.queryByText(/restores|recreates|expected full recovery/i)).toBeNull();
    });
  });
});