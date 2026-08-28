import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * The event bus is core's `on`, not `@tauri-apps/api/event` — this package
 * depends on `@srelens/core` and `@srelens/ui-kit` and nothing else. Captured
 * per channel so a test can emit exactly what the backend emits.
 */
const bus = vi.hoisted(() => {
  const handlers = new Map<string, (payload: unknown) => void>();
  const offs = new Map<string, () => void>();
  return { handlers, offs };
});

const core = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  respondToConfirm: vi.fn(async () => {}),
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
  on: vi.fn((channel: string, handler: (payload: unknown) => void) => {
    const off = vi.fn();
    return { channel, handler, off };
  }),
  vaultStatus: vi.fn(),
}));

vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  isTauri: core.isTauri,
  respondToConfirm: core.respondToConfirm,
  notify: core.notify,
  vaultStatus: core.vaultStatus,
  on: (channel: string, handler: (payload: unknown) => void) => {
    bus.handlers.set(channel, handler);
    const off = vi.fn();
    bus.offs.set(channel, off);
    core.on(channel, handler);
    return off;
  },
}));

import { AgentConsent } from "./AgentConsent";
import { lockWorkspace, resetLock, __setKnownVaultMode } from "./LockGate";

const REQUEST = "mcp://confirm-request";
const RESOLVED = "mcp://confirm-resolved";

function emit(channel: string, payload: unknown): void {
  const handler = bus.handlers.get(channel);
  if (!handler) throw new Error(`nothing subscribed to ${channel}`);
  act(() => handler(payload));
}

const ask = (id: string, tool: string, args: Record<string, unknown> = {}) =>
  emit(REQUEST, { id, tool, args });

beforeEach(() => {
  vi.clearAllMocks();
  bus.handlers.clear();
  bus.offs.clear();
  core.isTauri.mockReturnValue(true);
  core.respondToConfirm.mockResolvedValue(undefined);
  // Nothing here renders the gate itself; the store is what this component
  // reads, and a raise in one test must not still be up in the next.
  resetLock();
  // And an OPEN vault, said out loud rather than left to the store's default.
  // A fresh store has read no vault at all, and "no read has landed" counts as
  // covered where a vault exists — so without this every test in this file
  // would be exercising the refusal path and none of them would be about the
  // prompt. This is the state `LockGate`'s launch read establishes on the way
  // in; the covered cases below take it away deliberately.
  __setKnownVaultMode("unlocked");
});

describe("AgentConsent", () => {
  it("renders nothing until a request arrives", () => {
    const { container } = render(<AgentConsent />);
    expect(container.textContent).toBe("");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the tool and its arguments, and approves", async () => {
    render(<AgentConsent />);
    ask("r1", "k8s_deletePod", { name: "web-1", namespace: "prod" });
    expect(await screen.findByText(/k8s_deletePod/)).toBeTruthy();
    expect(screen.getByText(/web-1/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r1", true));
  });

  it("denies on the Deny button", async () => {
    render(<AgentConsent />);
    ask("r2", "k8s_scale");
    await screen.findByText(/k8s_scale/);
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r2", false));
  });

  it("queues a second request rather than dropping it", async () => {
    render(<AgentConsent />);
    ask("a", "toolA");
    ask("b", "toolB");
    await screen.findByText(/toolA/);
    expect(screen.getByText(/1 more request waiting/i)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    expect(await screen.findByText(/toolB/)).toBeTruthy();
  });

  it("drops a request resolved elsewhere instead of lingering over it", async () => {
    render(<AgentConsent />);
    ask("a", "toolA");
    ask("b", "toolB");
    await screen.findByText(/toolA/);
    emit(RESOLVED, { id: "a" });
    expect(await screen.findByText(/toolB/)).toBeTruthy();
    emit(RESOLVED, { id: "b" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // Answered by nobody here: the backend already settled it.
    expect(core.respondToConfirm).not.toHaveBeenCalled();
  });

  /**
   * A failed answer is the one thing here the reader has to be told, and this
   * dialog is the only surface that can tell them. `notify.error` reaches
   * sonner, whose `<Toaster>` is mounted in classic's `App` — and `main.tsx`
   * mounts that tree or this one and never both, so a toast raised from this
   * package is created and rendered nowhere. (#374 item 2)
   *
   * So the pin is on the DOM. The version of this test that shipped asserted
   * the `notify` mock had been called, which was true for the whole time
   * nothing whatsoever reached the reader — a test that could not fail for the
   * reason its own name gave.
   */
  it("tells the reader, on screen, when their answer did not take effect", async () => {
    core.respondToConfirm.mockRejectedValue(
      new Error("that confirmation is no longer waiting (it timed out or was already answered)"),
    );
    render(<AgentConsent />);
    ask("r3", "k8s_deleteResource");
    await screen.findByText(/k8s_deleteResource/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));

    const said = await screen.findByRole("alert");
    // What srelens knows for certain, in srelens's own words.
    expect(said.textContent).toMatch(/not answered by you/i);
    // And the reason, from the backend rather than invented here.
    expect(said.textContent).toMatch(/no longer waiting/i);
    // Not into the invisible sink instead: a toast host this tree does not
    // have is not a report, and a second copy of the failure would be one
    // problem said twice the moment #374's sweep mounts one.
    expect(core.notify.error).not.toHaveBeenCalled();
  });

  /**
   * The prompt SURVIVES a failed answer. Dropping the request in `finally` —
   * which is what shipped — took the question off screen exactly as though it
   * had been answered, and with the queue entry went the only way to try again.
   */
  it("keeps the prompt up when the answer did not land, so it can be tried again", async () => {
    core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
    render(<AgentConsent />);
    ask("r3", "k8s_deleteResource");
    await screen.findByText(/k8s_deleteResource/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await screen.findByRole("alert");

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText(/k8s_deleteResource/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /approve/i }).hasAttribute("disabled")).toBe(false);

    // And a second press that does land takes the failure down with the prompt.
    core.respondToConfirm.mockResolvedValue(undefined);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * The failure belongs to the request it happened on, and is held BY ID for
   * the same reason removal is: `mcp://confirm-resolved` can take the head out
   * from under this component at any moment — and it always eventually does,
   * since `ResolveOnDrop` broadcasts on every exit from `confirm`. A failure
   * kept as a bare string would then be painted under the NEXT agent's
   * question, telling the reader a call had been refused that never was.
   */
  it("does not carry a failure over onto the next request", async () => {
    core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
    render(<AgentConsent />);
    ask("a", "toolA");
    ask("b", "toolB");
    await screen.findByText(/toolA/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await screen.findByRole("alert");

    // The backend announces the resolution it refused the answer for.
    emit(RESOLVED, { id: "a" });
    expect(await screen.findByText(/toolB/)).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * The window, not one tab. Mounted outside every portal scope, the kit's
   * `ConfirmDialog` is a document-wide modal — `aria-modal`, Radix's own
   * overlay and its focus trap — which is what an app-wide question needs.
   * A tab-scoped one (PR #365) would cover a single tab and leave the rest of
   * the window live, and the reader could switch away from a question the
   * backend is blocking on.
   */
  it("asks the whole window rather than one tab of it", async () => {
    render(<AgentConsent />);
    ask("r4", "k8s_drainNode");
    const card = await screen.findByRole("dialog");
    expect(card.getAttribute("aria-modal")).toBe("true");
    expect(document.querySelector('[data-slot="dialog-overlay"]')?.className).toContain("fixed");
  });

  it("stops listening when it goes away", () => {
    const { unmount } = render(<AgentConsent />);
    const offRequest = bus.offs.get(REQUEST);
    const offResolved = bus.offs.get(RESOLVED);
    unmount();
    expect(offRequest).toHaveBeenCalledTimes(1);
    expect(offResolved).toHaveBeenCalledTimes(1);
  });

  /**
   * Every vault command and `mcp_confirm_respond` alike are Tauri commands, so
   * in a browser there is no gate to answer and nothing that could emit for it.
   */
  it("subscribes to nothing in web mode", () => {
    core.isTauri.mockReturnValue(false);
    render(<AgentConsent />);
    expect(core.on).not.toHaveBeenCalled();
  });

  // ---- While the window is covered ------------------------------------

  /**
   * A prompt over a sealed window would be a live control on a sealed session:
   * the backend raises and focuses the window before it emits, so the person
   * who answers is whoever is at the keyboard — not necessarily the reader who
   * sealed it. Refusing is the safe default, and it is said immediately rather
   * than left to the backend's 60-second timeout.
   */
  describe("while the workspace is covered", () => {
    it("refuses a request rather than putting it to a sealed window", async () => {
      render(<AgentConsent />);
      act(() => lockWorkspace());
      ask("r5", "k8s_drainNode");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r5", false));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("refuses what was already on screen when the cover went up", async () => {
      render(<AgentConsent />);
      ask("r6", "k8s_deletePod");
      await screen.findByRole("dialog");
      await act(async () => {
        lockWorkspace();
      });
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r6", false));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("refuses every queued request, not only the one in front", async () => {
      render(<AgentConsent />);
      ask("a", "toolA");
      ask("b", "toolB");
      await screen.findByRole("dialog");
      await act(async () => {
        lockWorkspace();
      });
      await waitFor(() => {
        expect(core.respondToConfirm).toHaveBeenCalledWith("a", false);
        expect(core.respondToConfirm).toHaveBeenCalledWith("b", false);
      });
    });

    /**
     * The refusal is the cover's, not this component's: an unlocked window puts
     * the next question to the reader as before. Without this the whole surface
     * could be a `respondToConfirm(id, false)` and every test above would still
     * pass on the sealed ones.
     */
    it("puts a later request to the reader once the cover is down", async () => {
      render(<AgentConsent />);
      act(() => lockWorkspace());
      ask("r7", "k8s_scale");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r7", false));
      await act(async () => {
        resetLock();
        // The unlock's own read, which is what actually lowers a cover: a
        // store with no mode in it is still covered, so `resetLock` alone
        // would leave this test asserting the refusal it is the control for.
        __setKnownVaultMode("unlocked");
      });
      ask("r8", "k8s_evictPod");
      expect(await screen.findByText(/k8s_evictPod/)).toBeTruthy();
      expect(core.respondToConfirm).not.toHaveBeenCalledWith("r8", false);
    });

    /**
     * The state the branch had left open, and it is the dangerous one: a vault
     * NOTHING has read.
     *
     * `sealed` and the launch check are both written by a mounted `LockGate`.
     * Before one mounts — a fresh module, a webview reload after `Lock now` —
     * neither is set, and the store used to answer "not covered" about a vault
     * it had never read. This component is mounted outside the boot gate on
     * purpose (a request is emitted once and never replayed), and the MCP
     * server is a backend process that survives a reload: so a confirm-gated
     * call could arrive in exactly that window and be put to whoever was at the
     * keyboard, with an Approve button, over a vault the backend had sealed.
     *
     * Not-yet-known counts as covered, so the answer is a refusal.
     */
    it("refuses over a vault whose state nothing has read yet", async () => {
      // No mode: the store as a fresh module has it, and as a reloaded webview
      // has it. Nothing has been sealed here — that is the point.
      act(() => __setKnownVaultMode(null));
      render(<AgentConsent />);
      ask("r10", "k8s_scale", { name: "api", replicas: 0 });
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r10", false));
      expect(core.respondToConfirm).not.toHaveBeenCalledWith("r10", true);
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByText(/k8s_scale/)).toBeNull();
    });

    /**
     * The browser is the other half of that rule, and it is not asserted here:
     * there is no vault in web mode, so `knownMode` stays `null` for the life
     * of the page — and this component subscribes to nothing there, so no
     * request can be delivered to observe the decision on. The store is where
     * that half is pinned (`LockGate.test.tsx`, "a vault whose state nothing
     * has read yet"), which is also where the `isTauri()` condition lives.
     */

    /**
     * A refusal that cannot be delivered — the call already timed out — has
     * nothing to tell the reader: they did not ask the question and cannot act
     * on the answer. It must not throw either, which an unhandled rejection
     * out of an effect would.
     */
    it("says nothing to the reader about a refusal it made on their behalf", async () => {
      core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
      const { container } = render(<AgentConsent />);
      act(() => lockWorkspace());
      ask("r9", "k8s_drainNode");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r9", false));
      // On screen rather than on a spy — the same reason the failure test
      // above changed. A covered window is the cover's, and this component
      // draws nothing over it: no prompt, and no failure line either.
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(container.textContent).toBe("");
    });
  });
});
