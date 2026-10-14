import { describe, it, expect, vi, type Mock } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { HelmOpPane } from "./HelmOpPane";
import { startHelmOp } from "@srelens/core";

vi.mock("@srelens/core/lib/helm", () => ({
  startHelmOp: vi.fn(async (_ctx, _args, onData, onExit) => {
    onData("Release \"web\" has been upgraded.");
    onExit(null);
    return { close: () => {} };
  }),
}));

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText` throws if not found) instead of `toBeInTheDocument`.
describe("HelmOpPane", () => {
  it("streams output lines and shows completion", async () => {
    render(
      <HelmOpPane
        session={{ id: 1, kind: "helm", context: "ctx", namespace: "apps", helm: { args: ["upgrade", "web", "c"], title: "Upgrade web" } }}
      />,
    );
    await waitFor(() => expect(screen.getByText(/has been upgraded/)).toBeTruthy());
    expect(screen.getByText(/Completed/i)).toBeTruthy();
  });

  it("closes a handle that resolves after unmount", async () => {
    const close = vi.fn();
    let resolve!: (h: { close: () => void }) => void;
    (startHelmOp as unknown as Mock).mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );

    const { unmount } = render(
      <HelmOpPane
        session={{ id: 2, kind: "helm", context: "ctx", namespace: "apps", helm: { args: ["upgrade", "web", "c"], title: "t" } }}
      />,
    );
    unmount();
    resolve({ close });
    await Promise.resolve();

    expect(close).toHaveBeenCalled();
  });

  // #734 review: closing a handle once helm has exited is how a page tells the
  // desktop host it showed the outcome, so the host does not report it again
  // when the window goes. It kills nothing: the process is already gone.
  it("closes its handle once helm has exited, while still on screen", async () => {
    const close = vi.fn();
    let exit!: (err: string | null) => void;
    (startHelmOp as unknown as Mock).mockImplementationOnce(
      async (_ctx: string, _args: string[], _onData: unknown, onExit: (err: string | null) => void) => {
        exit = onExit;
        return { close };
      },
    );
    render(
      <HelmOpPane
        session={{ id: 3, kind: "helm", context: "ctx", namespace: "apps", helm: { args: ["upgrade", "web", "c"], title: "t" } }}
      />,
    );
    await waitFor(() => expect(exit).toBeDefined());
    await Promise.resolve();
    expect(close).not.toHaveBeenCalled();

    exit("helm exited with code 1");

    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });

  it("closes a handle that arrives after helm has already exited", async () => {
    const close = vi.fn();
    (startHelmOp as unknown as Mock).mockImplementationOnce(
      async (_ctx: string, _args: string[], _onData: unknown, onExit: (err: string | null) => void) => {
        onExit(null);
        return { close };
      },
    );
    render(
      <HelmOpPane
        session={{ id: 4, kind: "helm", context: "ctx", namespace: "apps", helm: { args: ["upgrade", "web", "c"], title: "t" } }}
      />,
    );

    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });
});
