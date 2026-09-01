import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setNotifier } from "@srelens/core";
import { AppLog } from "./AppLog";

// The file, the path and the file manager are all core's, and all three need a
// Tauri backend. Mocked wholesale: this suite is about what the screen does
// with what core returns, and `isTauri` is a function so a test can move the
// app to the browser between renders.
const { core } = vi.hoisted(() => ({
  core: {
    readAppLog: vi.fn<() => Promise<string>>(),
    appLogPath: vi.fn<() => Promise<string>>(),
    revealAppLog: vi.fn<() => Promise<void>>(),
    isTauri: vi.fn(() => true),
  },
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

const PATH = "/Users/ada/Library/Logs/srelens/srelens.log";
const LOG = [
  "[2026-08-21][09:12:03][srelens::cluster][INFO] connected to prod",
  "[2026-08-21][09:12:04][srelens::rbac][ERROR] RBAC denied for Pods",
].join("\n");

beforeEach(() => {
  vi.clearAllMocks();
  // `clearAllMocks` forgets the calls, not the implementations — but these are
  // set per test often enough that spelling the defaults out here keeps one
  // test's rejection from leaking into the next.
  core.isTauri.mockReturnValue(true);
  core.readAppLog.mockResolvedValue(LOG);
  core.appLogPath.mockResolvedValue(PATH);
  core.revealAppLog.mockResolvedValue(undefined);
});

/** The rendered lines, in order, as `level → message`. */
const rendered = (region: HTMLElement) =>
  Array.from(region.querySelectorAll(".logline")).map(
    (el) =>
      `${el.querySelector("[data-slot=level]")?.textContent} ${el.querySelector("[data-slot=message]")?.textContent}`,
  );

describe("AppLog", () => {
  it("loads the log and renders one line per entry, with its level", async () => {
    render(<AppLog route="/logs" />);
    expect(screen.getByRole("status", { name: "Loading the log" })).toBeTruthy();

    const region = await screen.findByRole("log", { name: "Application log" });
    expect(rendered(region)).toEqual([
      "INFO connected to prod",
      "ERROR RBAC denied for Pods",
    ]);
    expect(within(region).getByText("09:12:03")).toBeTruthy();
    expect(within(region).getAllByText("srelens::cluster")).not.toHaveLength(0);
  });

  it("says so when the log has nothing in it", async () => {
    core.readAppLog.mockResolvedValue("");
    render(<AppLog route="/logs" />);
    expect(await screen.findByText("No log entries yet")).toBeTruthy();
  });

  it("reports a failed read, and retries it", async () => {
    core.readAppLog.mockRejectedValueOnce(new Error("permission denied"));
    render(<AppLog route="/logs" />);

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/permission denied/)).toBeTruthy();

    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    const region = await screen.findByRole("log", { name: "Application log" });
    expect(rendered(region)).toHaveLength(2);
    expect(core.readAppLog).toHaveBeenCalledTimes(2);
  });

  it("does not describe a local-file timeout as a Kubernetes timeout", async () => {
    core.readAppLog.mockRejectedValueOnce(new Error("read operation timed out"));
    render(<AppLog route="/logs" />);

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/local operation/i)).toBeTruthy();
    expect(alert.textContent).not.toMatch(/Kubernetes|cluster|kubeconfig/);
  });

  it("does not read anything in the browser, and says where the log lives", async () => {
    core.isTauri.mockReturnValue(false);
    render(<AppLog route="/logs" />);

    expect(
      screen.getByText("The application log is a file on the machine running srelens"),
    ).toBeTruthy();
    // Not merely "not yet" — nothing schedules a read on a later tick either.
    await waitFor(() => expect(core.readAppLog).not.toHaveBeenCalled());
    expect(core.appLogPath).not.toHaveBeenCalled();
  });

  it("reveals the log file in the file manager", async () => {
    render(<AppLog route="/logs" />);
    await screen.findByRole("log", { name: "Application log" });

    await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
    expect(core.revealAppLog).toHaveBeenCalledTimes(1);
  });

  /**
   * `revealAppLog` is `await invokeCommand("reveal_app_log")` with no catch of
   * its own, and the command behind it returns `Result<(), String>`: no file
   * manager, a sandbox denial, or a log directory it cannot resolve all come
   * back as a rejection. Fired as `void revealAppLog()` that was an unhandled
   * rejection and nothing at all on screen.
   *
   * A toast rather than a banner, which is the rule `NewForwardDialog` settled
   * for the same shape of failure: the gesture has no slot on the screen, and
   * nothing the reader was reading has gone wrong.
   */
  it("says so when the reveal is refused, rather than failing silently", async () => {
    core.revealAppLog.mockRejectedValue(new Error("no file manager on this machine"));
    const error = vi.fn();
    const restore = setNotifier({
      success: () => {},
      error,
      info: () => {},
      updateAvailable: () => {},
      clusterSignIn: () => {},
    });
    try {
      render(<AppLog route="/logs" />);
      await screen.findByRole("log", { name: "Application log" });

      await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
      await waitFor(() => expect(error).toHaveBeenCalledTimes(1));
      // Through `describeError`, like every other failure this package reports.
      expect(error.mock.calls[0][0]).toMatch(/reveal/i);
      expect(error.mock.calls[0][1]).toContain("no file manager on this machine");
    } finally {
      restore();
    }
  });

  it("says nothing when the reveal goes through", async () => {
    const error = vi.fn();
    const restore = setNotifier({
      success: () => {},
      error,
      info: () => {},
      updateAvailable: () => {},
      clusterSignIn: () => {},
    });
    try {
      render(<AppLog route="/logs" />);
      await screen.findByRole("log", { name: "Application log" });
      await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
      await waitFor(() => expect(core.revealAppLog).toHaveBeenCalledTimes(1));
      expect(error).not.toHaveBeenCalled();
    } finally {
      restore();
    }
  });

  it("reports a failed read in words, with the original folded away", async () => {
    // The kit's raw `ErrorState` printed whatever Rust sent as the detail. Every
    // other screen in this area routes the same class of value through
    // `lib/errorCopy`, which classifies it and folds the original behind a
    // disclosure — and this screen's failures are exactly the ones a reader
    // reaches it to understand.
    core.readAppLog.mockRejectedValue(
      new Error("ApiError: Unauthorized (Status { metadata: Some(ListMeta { .. }) })"),
    );
    render(<AppLog route="/logs" />);

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/rejected your credentials/i)).toBeTruthy();
    // The struct is inside the disclosure, never the sentence.
    const raw = alert.querySelector('[data-slot="raw"]');
    expect(raw?.textContent).toContain("ApiError");
    expect(within(alert).getByText("Could not read the application log")).toBeTruthy();
  });

  it("copies the log file's path to the clipboard", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    // jsdom ships no clipboard at all, so there is nothing to spy on.
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });

    render(<AppLog route="/logs" />);
    await screen.findByRole("log", { name: "Application log" });

    await userEvent.click(screen.getByRole("button", { name: "Copy path" }));
    expect(writeText).toHaveBeenCalledWith(PATH);
  });

  it("filters by level", async () => {
    render(<AppLog route="/logs" />);
    const region = await screen.findByRole("log", { name: "Application log" });

    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Log level" }), "ERROR");
    expect(rendered(region)).toEqual(["ERROR RBAC denied for Pods"]);
  });

  it("filters by text, and says when nothing matches", async () => {
    render(<AppLog route="/logs" />);
    const region = await screen.findByRole("log", { name: "Application log" });

    const field = screen.getByRole("searchbox", { name: "Filter log lines" });
    await userEvent.type(field, "rbac");
    expect(rendered(region)).toEqual(["ERROR RBAC denied for Pods"]);

    await userEvent.clear(field);
    await userEvent.type(field, "no such line");
    expect(screen.getByText("No lines match")).toBeTruthy();
    expect(rendered(region)).toEqual([]);
  });

  it("says how many lines are shown when the cap truncates real matches", async () => {
    const many = Array.from(
      { length: 5001 },
      (_, i) => `[2026-08-21][09:12:03][srelens::cluster][INFO] entry ${i}`,
    ).join("\n");
    core.readAppLog.mockResolvedValue(many);

    render(<AppLog route="/logs" />);
    const region = await screen.findByRole("log", { name: "Application log" });

    // 5000 shown of 5001 real matches — not "no lines match", the opposite
    // state, and not silent either: a truncated log must not look complete.
    expect(await screen.findByText(/5 000 of 5 001 lines/)).toBeTruthy();
    expect(rendered(region)).toHaveLength(5000);
  });
});
