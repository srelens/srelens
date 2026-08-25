import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Only the two capability wrappers and the platform check are replaced.
// `describeError` stays real, so the error assertions below are against core's
// own classification rather than a copy of it, and `plural` stays real so the
// skew sentence is counted by core's arithmetic.
const core = vi.hoisted(() => ({
  toolboxStatus: vi.fn(),
  startToolInstall: vi.fn(),
  isTauri: vi.fn(() => true),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import type { ClusterContext } from "@srelens/core";
// Chat exports a `ToolStatus` too; the toolbox one is the DTO these rows are.
import type { ToolStatus } from "@srelens/core/lib/toolbox";
import { Toolbox } from "./Toolbox";
import { resetContexts, setContexts } from "../lib/clusters";
import { probeCluster, resetProbes } from "../lib/probe";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { resetView } from "../lib/workspace";

const ROUTE = "/toolbox";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
};

/**
 * The three tools `toolbox.status` actually reports (`MANAGED_TOOLS` in
 * `crates/kube/src/toolbox.rs`), one in each of the three states the screen
 * can tell apart: srelens's own copy, somebody else's copy on the PATH, and
 * nothing at all.
 */
const MANAGED: ToolStatus = {
  name: "kubectl",
  installed: true,
  path: "/Users/ada/.srelens/bin/kubectl",
  version: "v1.31.4",
  source: "managed",
  sizeBytes: 54_200_000,
};
const SYSTEM: ToolStatus = {
  name: "helm",
  installed: true,
  path: "/opt/homebrew/bin/helm",
  version: "v3.16.3",
  source: "system",
  // Installed, on the PATH, and its path could not be stat'd — a dangling
  // symlink or a permission error. Distinct from "not installed", and the
  // reason the cell must not read `0 B`.
  sizeBytes: null,
};
const ABSENT: ToolStatus = {
  name: "krew",
  installed: false,
  path: null,
  version: null,
  source: null,
  sizeBytes: null,
};

const TOOLS = [MANAGED, SYSTEM, ABSENT];

beforeEach(() => {
  vi.clearAllMocks();
  core.isTauri.mockReturnValue(true);
  core.toolboxStatus.mockResolvedValue({ data: TOOLS });
  core.startToolInstall.mockResolvedValue({
    data: { tool: "krew", version: "v0.4.4", path: "/Users/ada/.krew/bin/krew" },
  });
  resetContexts();
  setContexts([CTX]);
  store.setState(defaultState([CTX]));
  resetProbes();
  resetView();
});

/** Probe the active cluster so the screen has a server version to compare with. */
const probe = (version: string) =>
  probeCluster(CTX, async () => ({ context: CTX.name, reachable: true, version }));

function open() {
  store.openTab(ROUTE);
  return render(<Toolbox route={ROUTE} />);
}

const headers = () =>
  Array.from(document.querySelectorAll("thead th")).map((th) => th.textContent?.trim() ?? "");
const rowFor = (name: string) => screen.getByText(name).closest("tr") as HTMLElement;
const cells = (row: HTMLElement) =>
  Array.from(row.querySelectorAll("td")).map((td) => td.textContent?.trim() ?? "");
/** The State cell's verdict word and the severity the pill drew it at. */
const verdict = (row: HTMLElement) => {
  const pill = row.querySelector(".status");
  return { word: pill?.textContent?.trim() ?? "", kind: pill?.getAttribute("data-kind") ?? "" };
};
/** The Note cell, addressed by column position so a moved column fails loudly. */
const note = (row: HTMLElement) => cells(row)[headers().indexOf("Note")];

describe("Toolbox — the inventory", () => {
  it("says it is loading while the binaries are being located", async () => {
    let release: (v: { data: ToolStatus[] }) => void = () => {};
    core.toolboxStatus.mockReturnValue(
      new Promise<{ data: ToolStatus[] }>((resolve) => {
        release = resolve;
      }),
    );
    open();

    // Not instant: `toolbox.status` shells out to every tool it knows about.
    expect(screen.getByRole("status", { name: "Locating the toolchain" })).toBeTruthy();
    expect(screen.queryByText("kubectl")).toBeNull();

    release({ data: TOOLS });
    expect(await screen.findByText("kubectl")).toBeTruthy();
  });

  it("heads the pane with where srelens keeps what it installs", async () => {
    open();
    await screen.findByText("kubectl");
    expect(screen.getByText("Managed tools · installed under ~/.srelens/bin")).toBeTruthy();
  });

  it("renders one row per tool, with its version and its state", async () => {
    open();
    await screen.findByText("kubectl");

    expect(cells(rowFor("kubectl"))[headers().indexOf("Version")]).toBe("v1.31.4");
    expect(verdict(rowFor("kubectl"))).toEqual({ word: "Installed", kind: "success" });

    // A tool nobody has installed has no version to show either — an absence,
    // rendered as one.
    expect(cells(rowFor("krew"))[headers().indexOf("Version")]).toBe("—");
    expect(verdict(rowFor("krew"))).toEqual({ word: "Missing", kind: "neutral" });
  });

  it("tells a copy on the PATH apart from the copy srelens installed", async () => {
    open();
    await screen.findByText("helm");

    // The distinction is the point: `helm` IS installed and IS usable, so the
    // row must not read the same as kubectl's, which srelens put there and can
    // replace. Asserted against the managed row so a state table that
    // collapsed the two fails here rather than passing on a word.
    expect(verdict(rowFor("helm"))).toEqual({ word: "Unmanaged", kind: "neutral" });
    expect(verdict(rowFor("helm"))).not.toEqual(verdict(rowFor("kubectl")));
    expect(note(rowFor("helm"))).toContain("on PATH");

    // And srelens's own copy claims nothing about the PATH.
    expect(note(rowFor("kubectl"))).not.toContain("on PATH");
  });

  it("sizes what it can measure, and dashes what it cannot", async () => {
    open();
    await screen.findByText("kubectl");

    expect(headers()).toContain("Size");
    // Measured.
    expect(cells(rowFor("kubectl"))).toContain("54.2 MB");
    // Installed but unreadable, and not installed at all: two different
    // reasons for the same answer, and neither of them is `0 B`. A zero is a
    // measurement, and nobody took one.
    expect(cells(rowFor("helm"))).toContain("\u2014");
    expect(cells(rowFor("krew"))).toContain("\u2014");
    expect(screen.queryByText(/\b0 B\b/)).toBeNull();
  });
});

describe("Toolbox — kubectl against the cluster", () => {
  it("says kubectl matches the server when the minors agree", async () => {
    await probe("v1.31.4");
    open();
    await screen.findByText("kubectl");

    expect(note(rowFor("kubectl"))).toContain("matches prod-eu server version");
  });

  it("names the skew when kubectl runs ahead of the server", async () => {
    await probe("v1.29.8");
    open();
    await screen.findByText("kubectl");

    // v1.31 client, v1.29 server: two minors, and the direction is part of the
    // fact. "matches" must be gone, not merely joined by a second sentence.
    expect(note(rowFor("kubectl"))).toContain("2 minors ahead of prod-eu");
    expect(note(rowFor("kubectl"))).not.toContain("matches");
  });

  it("names the skew, and its direction, when kubectl runs behind the server", async () => {
    core.toolboxStatus.mockResolvedValue({
      data: [{ ...MANAGED, version: "v1.30.1" }, SYSTEM, ABSENT],
    });
    await probe("v1.31.4");
    open();
    await screen.findByText("kubectl");

    expect(note(rowFor("kubectl"))).toContain("1 minor behind prod-eu");
    expect(note(rowFor("kubectl"))).not.toContain("ahead");
  });

  it("claims no skew when nothing has read a server version", async () => {
    // No probe: the screen is app-scoped and can be opened before any cluster
    // answers. A note that guessed "matches" here would be an assertion about
    // a cluster nobody asked.
    open();
    await screen.findByText("kubectl");

    expect(note(rowFor("kubectl"))).toBe("");
  });

  it("compares nothing but kubectl against the server", async () => {
    await probe("v1.31.4");
    open();
    await screen.findByText("helm");

    // helm v3.16 is not "13 minors ahead of prod-eu", and it does not "differ
    // from the prod-eu server version" either: its versions have nothing to do
    // with the API server's, so the cluster is not named on its row at all.
    // Asserted as the WHOLE cell rather than as a pattern — a comparison that
    // ran on every tool would land here as extra text, whatever words it
    // chose, and a pattern only catches the words it thought of.
    expect(note(rowFor("helm"))).toBe("on PATH");
    expect(note(rowFor("krew"))).toBe("");
  });
});

describe("Toolbox — what can be done, and where", () => {
  it("offers an install on the desktop, and runs the real one", async () => {
    open();
    await screen.findByText("krew");

    expect(headers()).toEqual(["Tool", "Version", "State", "Note", "Size", ""]);
    // The missing tool is offered an install; the managed one a replacement.
    expect(screen.getByRole("button", { name: "Install krew" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reinstall kubectl" })).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "Install krew" }));
    expect(core.startToolInstall).toHaveBeenCalledWith("krew", expect.any(Function));
    // And the inventory is read again, so the row stops saying Missing.
    await waitFor(() => expect(core.toolboxStatus).toHaveBeenCalledTimes(2));
  });

  it("reports an install that failed, and leaves the rows standing", async () => {
    core.startToolInstall.mockResolvedValue({ error: "Error: handler error: checksum mismatch" });
    open();
    await screen.findByText("krew");

    await userEvent.click(screen.getByRole("button", { name: "Install krew" }));

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/checksum mismatch/)).toBeTruthy();
    // Classified, not echoed: the two wrapper prefixes never reach the reader.
    expect(alert.textContent).not.toContain("handler error:");
    expect(rowFor("kubectl")).toBeTruthy();
  });

  it("draws no action column in the browser, and says once why", async () => {
    core.isTauri.mockReturnValue(false);
    open();
    await screen.findByText("krew");

    // Every per-row button here is a `WEB_DENIED_CAPABILITIES` entry, so none
    // is drawn — the column is gone, not disabled.
    expect(headers()).toEqual(["Tool", "Version", "State", "Note", "Size"]);
    expect(cells(rowFor("krew"))).toHaveLength(5);
    expect(screen.queryByRole("button", { name: /^(Install|Reinstall)/ })).toBeNull();

    // Said once, for the whole table, rather than per row.
    expect(screen.getAllByText("Tools are managed where srelens runs")).toHaveLength(1);
    // And the inventory itself still reads: `toolbox.status` is allowed on web.
    expect(screen.getByText("v1.31.4")).toBeTruthy();
  });
});

describe("Toolbox — when the inventory cannot be taken", () => {
  it("reports a failed status in words, and retries it", async () => {
    core.toolboxStatus.mockResolvedValueOnce({
      error: "Error: handler error: PATH could not be read",
    });
    open();

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Could not inventory the toolchain")).toBeTruthy();
    // `describeError`'s cleaning, not the raw string: both wrapper prefixes are
    // stripped before anything reaches the reader.
    expect(within(alert).getByText("PATH could not be read")).toBeTruthy();
    expect(alert.textContent).not.toContain("handler error:");
    expect(alert.textContent).not.toContain("Error: Error:");

    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("kubectl")).toBeTruthy();
    expect(core.toolboxStatus).toHaveBeenCalledTimes(2);
  });

  it("says so when the backend reports no tools at all", async () => {
    core.toolboxStatus.mockResolvedValue({ data: [] });
    open();

    expect(await screen.findByText("No tools reported")).toBeTruthy();
  });
});
