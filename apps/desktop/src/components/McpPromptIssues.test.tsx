import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const promptIssues = vi.fn();
vi.mock("../lib/mcpSecurity", () => ({ promptIssues: () => promptIssues() }));

import { McpPromptIssues } from "./McpPromptIssues";

describe("McpPromptIssues", () => {
  beforeEach(() => promptIssues.mockReset());
  // A persistent `mockRejectedValue` (as opposed to `Once`) leaves the mock
  // rejecting after its test's body returns; a later call into it during
  // test-file teardown then surfaces as an unhandled rejection misattributed
  // to whichever test last ran. Reset after each test too so it never
  // outlives the test that configured it.
  afterEach(() => promptIssues.mockReset());

  it("names the file and the reason it was skipped", async () => {
    promptIssues.mockResolvedValue([
      { file: "mine.md", problem: "body uses undeclared argument(s): pod" },
    ]);
    render(<McpPromptIssues />);
    expect(await screen.findByText(/mine\.md/)).toBeTruthy();
    expect(screen.getByText(/undeclared argument/)).toBeTruthy();
  });

  /// The common case is no issues at all — that must render nothing rather than
  /// an empty panel implying something is wrong.
  ///
  /// Both negative tests below wait for the fetch to have SETTLED before
  /// asserting emptiness. Asserting straight after render would pass trivially:
  /// the component returns null on its first paint regardless, so the assertion
  /// would hold even if the component were completely broken.
  it("renders nothing when every file loaded", async () => {
    promptIssues.mockResolvedValue([]);
    const { container } = render(<McpPromptIssues />);
    await waitFor(() => expect(promptIssues).toHaveBeenCalled());
    await act(async () => {});
    expect(container.textContent).toBe("");
  });

  /// Guards the guard: with a non-empty result the same wait-then-assert shape
  /// must NOT see empty output. Without this, a bug that made the component
  /// always render null would leave both tests above green.
  it("does render once issues arrive, so the empty assertions mean something", async () => {
    promptIssues.mockResolvedValue([{ file: "a.md", problem: "bad" }]);
    const { container } = render(<McpPromptIssues />);
    await waitFor(() => expect(promptIssues).toHaveBeenCalled());
    await act(async () => {});
    expect(container.textContent).not.toBe("");
  });

  /// Editing a prompt file takes effect without restarting srelens, so a
  /// panel that only fetches once on mount would keep showing a fixed file's
  /// stale error forever. The parent bumps `nonce` (driven by the Refresh
  /// button next to `McpAuditList`) to force a re-read.
  it("re-reads when its nonce prop changes", async () => {
    promptIssues.mockResolvedValue([
      { file: "mine.md", problem: "body uses undeclared argument(s): pod" },
    ]);
    const { rerender } = render(<McpPromptIssues nonce={0} />);
    expect(await screen.findByText(/mine\.md/)).toBeTruthy();
    expect(promptIssues).toHaveBeenCalledTimes(1);

    promptIssues.mockResolvedValue([]);
    rerender(<McpPromptIssues nonce={1} />);
    await waitFor(() => expect(promptIssues).toHaveBeenCalledTimes(2));
  });
});
