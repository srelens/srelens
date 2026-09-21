import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach } from "vitest";

const respondToConfirm = vi.fn();
const handlers: Record<string, (e: { payload: unknown }) => void> = {};

vi.mock("@srelens/core/lib/mcpSecurity", () => ({
  respondToConfirm: (...a: unknown[]) => respondToConfirm(...a),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));
const emit = (payload: unknown) => handlers["mcp://confirm-request"]({ payload });
const emitResolved = (id: string) => handlers["mcp://confirm-resolved"]({ payload: { id } });
const { notify } = vi.hoisted(() => ({
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("@srelens/core/lib/notify", () => ({ notify }));

import { McpConfirmDialog } from "./McpConfirmDialog";

// jsdom is a plain browser, i.e. web mode: `isTauri()` looks for
// `window.__TAURI_INTERNALS__` and finds nothing. The consent flow below is
// desktop-only, so the suite runs it under a Tauri marker and clears the
// marker for the web cases at the end.
const TAURI_MARKER = "__TAURI_INTERNALS__";
function enterTauri() {
  (window as unknown as Record<string, unknown>)[TAURI_MARKER] = {};
}
function leaveTauri() {
  delete (window as unknown as Record<string, unknown>)[TAURI_MARKER];
}

describe("McpConfirmDialog", () => {
  beforeEach(() => {
    enterTauri();
    for (const k of Object.keys(handlers)) delete handlers[k];
    respondToConfirm.mockReset();
    notify.success.mockReset();
    notify.error.mockReset();
    notify.info.mockReset();
  });

  it("does not subscribe to Tauri events on the web (#512)", () => {
    // On the web, AppGate renders classic App after sign-in, and App mounts
    // this dialog unconditionally. `listen()` from @tauri-apps/api reaches
    // for `window.__TAURI_INTERNALS__.transformCallback` and throws in a
    // browser -- two unguarded calls, two uncaught rejections per page load.
    leaveTauri();
    const { container } = render(<McpConfirmDialog />);
    expect(container.textContent).toBe("");
    expect(Object.keys(handlers)).toEqual([]);
  });

  it("renders nothing until a request arrives", () => {
    const { container } = render(<McpConfirmDialog />);
    expect(container.textContent).toBe("");
  });

  it("shows the tool and arguments, and approves", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "r1", tool: "k8s_deletePod", args: { name: "web-1", namespace: "prod" } });
    await screen.findByText(/k8s_deletePod/);
    expect(screen.getByText(/web-1/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(respondToConfirm).toHaveBeenCalledWith("r1", true));
  });

  it("denies on the deny button", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "r2", tool: "k8s_scale", args: {} });
    await screen.findByText(/k8s_scale/);
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    await waitFor(() => expect(respondToConfirm).toHaveBeenCalledWith("r2", false));
  });

  it("queues a second request rather than dropping it", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "a", tool: "toolA", args: {} });
    emit({ id: "b", tool: "toolB", args: {} });
    await screen.findByText(/toolA/);
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    await screen.findByText(/toolB/);
  });

  it("surfaces an error instead of silently swallowing a failed response", async () => {
    respondToConfirm.mockRejectedValue(new Error("already timed out"));
    render(<McpConfirmDialog />);
    emit({ id: "r3", tool: "k8s_deletePod", args: {} });
    await screen.findByText(/k8s_deletePod/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(notify.error).toHaveBeenCalled());
    // The user must not be left believing the call was actioned: the
    // request is still dropped from the queue (nothing left to retry), but
    // the failure is surfaced rather than silent.
    expect(screen.queryByText(/k8s_deletePod/)).toBeNull();
  });

  it("drops a request resolved elsewhere (inline card or timeout) instead of lingering", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "a", tool: "toolA", args: {} });
    emit({ id: "b", tool: "toolB", args: {} });
    await screen.findByText(/toolA/);
    // The head request gets answered on the assistant's inline card — the
    // backend broadcasts the resolution and the modal must move on without
    // a click here (and without answering anything itself).
    emitResolved("a");
    await screen.findByText(/toolB/);
    emitResolved("b");
    await waitFor(() => expect(screen.queryByText(/toolB/)).toBeNull());
    expect(respondToConfirm).not.toHaveBeenCalled();
  });

  /**
   * #548. The prompt was the tool id and a JSON blob — the same question for
   * an Argo CD status refresh and a node drain. The backend now renders its
   * own sentence for the call, from a template compiled into it, and sends the
   * impact level beside it.
   */
  it("leads with the host's sentence and names the impact", async () => {
    render(<McpConfirmDialog />);
    emit({
      id: "p1",
      tool: "k8s.drainNode",
      args: { context: "prod", name: "node-7" },
      prompt: "Drain node-7 in cluster prod?",
      impact: "high",
    });
    expect(await screen.findByText("Drain node-7 in cluster prod?")).toBeTruthy();
    expect(screen.getByText(/high impact/i)).toBeTruthy();
    // And the call itself still travels: the sentence says what it does, the
    // payload says exactly which call it is.
    expect(screen.getByText(/k8s.drainNode/)).toBeTruthy();
  });

  /**
   * The fallback is as much the point as the sentence. A capability with no
   * template, or one whose template names a field this call has no value for,
   * arrives with no prompt at all — and the dialog shows what it always
   * showed, rather than a sentence with a hole in it over an Approve button.
   */
  it("falls back to the tool and its arguments when there is no host sentence", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "p2", tool: "toolbox.installHelm", args: { version: "3.16" }, impact: "medium" });
    const tool = await screen.findByText(/toolbox.installHelm/);
    expect(tool).toBeTruthy();
    // The level is still named — it is the half of the metadata that does not
    // depend on a template, and it is what distinguishes this prompt from the
    // one the dialog drew before #548.
    expect(screen.getByText(/medium impact/i)).toBeTruthy();
    expect(screen.getByText(/3.16/)).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/undefined|null/);
  });
});
