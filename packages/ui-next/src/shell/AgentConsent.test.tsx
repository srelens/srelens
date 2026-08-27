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
import { lockWorkspace, resetLock } from "./LockGate";

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

  it("surfaces a failed response instead of letting the reader believe it landed", async () => {
    core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
    render(<AgentConsent />);
    ask("r3", "k8s_deleteResource");
    await screen.findByText(/k8s_deleteResource/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(core.notify.error).toHaveBeenCalled());
    // Still dropped — there is nothing left here to retry.
    await waitFor(() => expect(screen.queryByText(/k8s_deleteResource/)).toBeNull());
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
      });
      ask("r8", "k8s_evictPod");
      expect(await screen.findByText(/k8s_evictPod/)).toBeTruthy();
      expect(core.respondToConfirm).not.toHaveBeenCalledWith("r8", false);
    });

    /**
     * A refusal that cannot be delivered — the call already timed out — has
     * nothing to tell the reader: they did not ask the question and cannot act
     * on the answer. It must not throw either, which an unhandled rejection
     * out of an effect would.
     */
    it("says nothing to the reader about a refusal it made on their behalf", async () => {
      core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
      render(<AgentConsent />);
      act(() => lockWorkspace());
      ask("r9", "k8s_drainNode");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r9", false));
      expect(core.notify.error).not.toHaveBeenCalled();
    });
  });
});
