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

  it("omits the thoughts row entirely for an agent that streamed none", () => {
    render(<Transcript turns={[turn({ text: "done" })]} gates={[]} />);
    expect(screen.queryByText(/thought/i)).toBeNull();
  });

  it("shows a gate as a record, with no second set of answer buttons", () => {
    render(
      <Transcript turns={[turn()]} gates={[{ id: "g", tool: "k8s.scale", args: {}, outcome: "pending" }]} />,
    );
    expect(screen.queryByRole("button", { name: /approve/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /deny/i })).toBeNull();
  });
});
