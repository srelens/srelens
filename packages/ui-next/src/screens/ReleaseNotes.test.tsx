import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UpdateMeta } from "@srelens/core";

const checkForUpdate = vi.fn();
const installUpdate = vi.fn();
const appVersion = vi.fn();
const loadUpdateChannel = vi.fn();
const isTauri = vi.fn();

// Everything this screen does is a call into core; the screen is what is under
// test, so core is a set of doubles.
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  checkForUpdate: (...args: unknown[]) => checkForUpdate(...args),
  installUpdate: (...args: unknown[]) => installUpdate(...args),
  appVersion: () => appVersion(),
  loadUpdateChannel: () => loadUpdateChannel(),
  isTauri: () => isTauri(),
}));

import { ReleaseNotes } from "./ReleaseNotes";

const update = (over: Partial<UpdateMeta> = {}): UpdateMeta => ({
  version: "0.8.0",
  currentVersion: "0.7.2",
  notes: "### Fixed\n- a crash on start",
  external: false,
  elevates: false,
  ...over,
});

beforeEach(() => {
  checkForUpdate.mockReset().mockResolvedValue(null);
  installUpdate.mockReset().mockResolvedValue(undefined);
  appVersion.mockReset().mockResolvedValue("0.7.2");
  loadUpdateChannel.mockReset().mockReturnValue("stable");
  isTauri.mockReset().mockReturnValue(true);
});

describe("ReleaseNotes", () => {
  it("says which version is installed when there is no update", async () => {
    render(<ReleaseNotes route="/release-notes" />);

    expect(await screen.findByText("srelens is up to date")).toBeDefined();
    expect(screen.getByText("Version 0.7.2")).toBeDefined();
    expect(checkForUpdate).toHaveBeenCalledWith("stable");
  });

  it("renders the available version, its notes and an install button", async () => {
    checkForUpdate.mockResolvedValue(update());
    render(<ReleaseNotes route="/release-notes" />);

    expect(await screen.findByText("Update to 0.8.0")).toBeDefined();
    expect(screen.getByRole("heading", { level: 3, name: "Fixed" })).toBeDefined();
    expect(screen.getByText("a crash on start")).toBeDefined();
    expect(screen.getByRole("button", { name: /install/i })).toBeDefined();
  });

  it("installs on the stored channel and shows the progress it reports", async () => {
    loadUpdateChannel.mockReturnValue("dev");
    checkForUpdate.mockResolvedValue(update());
    let report!: (percent: number | null) => void;
    let finish!: () => void;
    installUpdate.mockImplementation(
      (_channel: unknown, onProgress: (percent: number | null) => void) =>
        new Promise<void>((resolve) => {
          report = onProgress;
          finish = resolve;
        }),
    );
    render(<ReleaseNotes route="/release-notes" />);

    await userEvent.click(await screen.findByRole("button", { name: /install/i }));
    expect(installUpdate).toHaveBeenCalledWith("dev", expect.any(Function));

    await act(async () => report(42));
    expect(screen.getByText("Installing… 42%")).toBeDefined();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("42");

    await act(async () => {
      finish();
    });
    expect(screen.getByText("Restart srelens to finish")).toBeDefined();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("offers no install when a package manager owns the install", async () => {
    checkForUpdate.mockResolvedValue(update({ external: true }));
    render(<ReleaseNotes route="/release-notes" />);

    expect(
      await screen.findByText("Installed by your package manager — update it there"),
    ).toBeDefined();
    expect(screen.queryByRole("button", { name: /install/i })).toBeNull();
  });

  it("warns before the install asks for a password", async () => {
    checkForUpdate.mockResolvedValue(update({ elevates: true }));
    render(<ReleaseNotes route="/release-notes" />);

    expect(await screen.findByText("srelens will ask for your password")).toBeDefined();
    expect(screen.getByRole("button", { name: /install/i })).toBeDefined();
  });

  it("shows the failure and checks again on retry", async () => {
    checkForUpdate.mockRejectedValueOnce(new Error("network is unreachable"));
    render(<ReleaseNotes route="/release-notes" />);

    expect(await screen.findByRole("alert")).toBeDefined();
    expect(screen.getByText("network is unreachable")).toBeDefined();

    checkForUpdate.mockResolvedValue(update());
    await userEvent.click(screen.getByRole("button", { name: /retry/i }));

    expect(await screen.findByText("Update to 0.8.0")).toBeDefined();
    expect(checkForUpdate).toHaveBeenCalledTimes(2);
  });

  it("reports an install that failed", async () => {
    checkForUpdate.mockResolvedValue(update());
    installUpdate.mockRejectedValue(new Error("no space left on device"));
    render(<ReleaseNotes route="/release-notes" />);

    await userEvent.click(await screen.findByRole("button", { name: /install/i }));

    expect(await screen.findByText(/no space left on device/)).toBeDefined();
    expect(screen.getByRole("button", { name: /install/i }).hasAttribute("disabled")).toBe(false);
  });

  /**
   * The install's own failure went to the reader as `install.message` — the
   * caught value's `.message`, or `String(cause)` — straight onto the screen.
   * Every other screen in this area routes the same class of value through
   * `lib/errorCopy`, which classifies it with `describeError` and folds the
   * original behind a disclosure; the OBJECT has to reach it for that to be
   * possible at all.
   */
  it("classifies a failed install rather than printing what the updater said", async () => {
    checkForUpdate.mockResolvedValue(update());
    installUpdate.mockRejectedValue(
      new Error("error sending request: tcp connect error: connection refused"),
    );
    render(<ReleaseNotes route="/release-notes" />);

    await userEvent.click(await screen.findByRole("button", { name: /install/i }));

    // The sentence a reader can act on, not hyper's Display.
    const alert = await screen.findByText(/could not be made/i);
    expect(alert).toBeTruthy();
    const raw = alert.closest("[data-tone]")?.querySelector('[data-slot="raw"]');
    expect(raw?.textContent).toContain("tcp connect error");
    // Still announced: this arrives while the reader is looking elsewhere.
    expect(alert.closest('[role="alert"]')).toBeTruthy();
  });

  it("strips the wrapper the updater's own rejection arrives with", async () => {
    // A rejected Tauri command rejects with a STRING, not an `Error`, and
    // `CapabilityError`'s Display puts `handler error:` in front of it. Read off
    // `.message`/`String(cause)` that prefix went to the reader verbatim; it is
    // not news to anyone, and `describeError` is the one place that knows so.
    checkForUpdate.mockResolvedValue(update());
    installUpdate.mockRejectedValue("handler error: no space left on device");
    render(<ReleaseNotes route="/release-notes" />);

    await userEvent.click(await screen.findByRole("button", { name: /install/i }));

    expect(await screen.findByText("no space left on device")).toBeDefined();
    expect(screen.queryByText(/handler error/)).toBeNull();
  });

  it("classifies a failed check the same way", async () => {
    checkForUpdate.mockRejectedValueOnce(
      new Error("ApiError: Unauthorized (Status { metadata: Some(ListMeta { .. }) })"),
    );
    render(<ReleaseNotes route="/release-notes" />);

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/rejected your credentials/i)).toBeTruthy();
    expect(alert.querySelector('[data-slot="raw"]')?.textContent).toContain("ApiError");
    // The screen's own title survives the classification.
    expect(within(alert).getByText("Could not check for updates")).toBeTruthy();
  });

  it("describes updater timeouts as HTTP failures, not cluster failures", async () => {
    checkForUpdate.mockRejectedValueOnce(new Error("request timed out"));
    render(<ReleaseNotes route="/release-notes" />);

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/network connection/i)).toBeTruthy();
    expect(alert.textContent).not.toMatch(/Kubernetes|cluster|kubeconfig/);
  });

  it("does not blame kubeconfig when the update server's TLS check fails", async () => {
    checkForUpdate.mockResolvedValue(update());
    installUpdate.mockRejectedValue(new Error("x509: certificate signed by unknown authority"));
    render(<ReleaseNotes route="/release-notes" />);

    await userEvent.click(await screen.findByRole("button", { name: /install/i }));
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/server's TLS certificate/i)).toBeTruthy();
    expect(alert.textContent).not.toMatch(/cluster|kubeconfig|certificate-authority/);
  });

  it("leaves updates to the server in web mode", async () => {
    isTauri.mockReturnValue(false);
    render(<ReleaseNotes route="/release-notes" />);

    expect(await screen.findByText("Updates are managed by the server")).toBeDefined();
    expect(checkForUpdate).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /install/i })).toBeNull();
  });
});
