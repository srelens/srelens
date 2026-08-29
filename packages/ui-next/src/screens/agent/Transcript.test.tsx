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

  it("adds the resolution time once one is stamped", () => {
    const at = new Date(2024, 0, 1, 14, 6).getTime();
    render(
      <Transcript
        turns={[turn()]}
        gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "approved", at }]}
      />,
    );
    expect(screen.getByText("Applied 14:06")).toBeTruthy();
  });
});
