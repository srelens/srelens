import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  inspectExtension: vi.fn(),
}));
import {
  inspectExtension,
  type ExtensionInspection,
  type ExtensionProcess,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionInspector } from "./ExtensionInspector";

const MiB = 1024 * 1024;
const plugin: InstalledExtension = {
  manifest: {
    id: "org.test.gitops",
    name: "GitOps",
    version: "1.2.0",
    srelensApiVersion: "^0.5",
    kind: "declarative",
    permissions: ["k8s.listCustomResource", "k8s.listEvents"],
    capabilities: [
      { name: "applications", title: "Applications", target: "k8s.listCustomResource", arguments: {}, inputs: [] },
      { name: "events", title: "Events", target: "k8s.listEvents", arguments: {}, inputs: [] },
    ],
    actions: [
      { name: "sync", title: "Sync", target: "k8s.annotate", resource: "applications", arguments: {} },
    ],
    contributions: {
      pages: [{ id: "apps", title: "Applications", capability: "applications" }],
      detailTabs: [],
      detailLinks: [],
      tableColumns: [
        { id: "a", title: "A", forKinds: ["apps/Deployment"], source: { jsonPath: ".a" }, format: "text" },
        { id: "b", title: "B", forKinds: ["apps/Deployment"], source: { jsonPath: ".b" }, format: "text" },
      ],
      dashboardCards: [],
    },
  },
  enabled: true,
  revision: 3,
  grants: ["k8s.listCustomResource", "k8s.listEvents"],
  settings: {},
  source: "catalog",
  installedAt: 1_700_000_000,
  signatureProof: { manifest: "{}", signature: [1] },
  history: [],
};

const declarative = (more: Partial<ExtensionInspection> = {}): ExtensionInspection => ({
  id: "org.test.gitops",
  runtime: "declarative",
  process: null,
  streams: { open: [], watches: [], opened: 0, messages: 0, bytes: 0, rateLimited: 0, refused: 0, windowEnded: 0, maxOpen: 8 },
  recentErrors: [],
  log: { lines: 0, capacity: 1000, dropped: 0 },
  ...more,
});
const running: ExtensionProcess = {
  state: "running",
  reason: null,
  message: null,
  actions: ["disable"],
  apiVersion: "0.5.0",
  pid: 4242,
  startedAt: new Date(2026, 8, 27, 9, 30, 0).getTime(),
  restart: null,
  launches: 2,
  unexpectedExits: 1,
  memory: { bytes: 50 * MiB, limitBytes: 256 * MiB, enforcement: "kernel" },
  cpus: 2,
  rpc: {
    answered: 120,
    failed: 3,
    timedOut: 1,
    refused: 0,
    inFlight: 2,
    latency: { samples: 120, p50Ms: 3, p95Ms: 18.5, maxMs: 40 },
  },
  streams: { open: 1, opened: 7, limit: 16 },
};
const sidecar = (process: Partial<ExtensionProcess> = {}, more: Partial<ExtensionInspection> = {}) =>
  declarative({ runtime: "sidecar", process: { ...running, ...process }, ...more });

const region = (name: string) => screen.getByRole("region", { name });
/** The value beside a term in a section's facts. */
const fact = (section: string, term: string) => within(region(section)).getByText(term).nextElementSibling?.textContent;

const change = vi.fn();
const onViewLogs = vi.fn();
async function open(app: InstalledExtension = plugin) {
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(<ExtensionInspector plugin={app} busy={false} change={change} onViewLogs={onViewLogs} />);
  });
  return view;
}
const wait = (ms: number) => act(async () => {
  await vi.advanceTimersByTimeAsync(ms);
});

beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
  change.mockResolvedValue(true);
});
afterEach(() => {
  vi.useRealTimers();
});

it("names the app's identity, grants, contributions and registered capabilities from its manifest", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(declarative());
  await open();
  expect(inspectExtension).toHaveBeenCalledWith("org.test.gitops");
  expect(fact("Identity", "ID")).toBe("org.test.gitops");
  expect(fact("Identity", "Version")).toBe("1.2.0");
  expect(fact("Identity", "Extension API version")).toBe("^0.5");
  expect(fact("Identity", "Revision")).toBe("3");
  expect(fact("Identity", "Source and signature")).toBe("Signed by srelens · from the Catalog");
  expect(fact("Identity", "Installed")).toContain("2023");
  const grants = within(region("Grants")).getByRole("list", { name: "Granted capabilities" });
  expect(within(grants).getByText("k8s.listEvents").closest("li")!.textContent).toContain("Read-only");
  const contributions = region("Active contributions");
  expect(contributions.textContent).toContain("Active on every cluster.");
  expect(within(contributions).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "1 page",
    "2 table columns",
    "1 action",
  ]);
  const rows = within(region("Registered capabilities")).getAllByRole("row").slice(1);
  expect(rows.map((row) => within(row).getAllByRole("cell").map((cell) => cell.textContent))).toEqual([
    ["applications", "Applications", "k8s.listCustomResource"],
    ["events", "Events", "k8s.listEvents"],
  ]);
});

it("says a declarative app has no process, no streams, no watches and no errors, and where its failures show", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(declarative());
  await open();
  expect(region("Process").textContent).toContain(
    "No process. This is a declarative app: srelens runs nothing of its own for it, so there is no process, memory or request traffic to show.",
  );
  const streams = region("Streams and watches");
  expect(within(streams).getByText("No open streams.")).toBeTruthy();
  expect(within(streams).getByText("No watches.")).toBeTruthy();
  expect(fact("Streams and watches", "Refused at the cap of 8 open streams")).toBe("0");
  expect(region("Recent errors").textContent).toContain(
    "No errors recorded since srelens started. Its pages and cards show their own failures where they happen.",
  );
});

it("lists the app's open streams and its watches apart, each with its view and traffic", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(
    declarative({
      streams: {
        open: [{ stream: "extstream:1", view: "org.test.gitops/page:apps#1", revision: 3, source: "read", messages: 12, bytes: 2048 }],
        watches: [{ stream: "extstream:2", view: "org.test.gitops/page:apps#2", revision: 3, source: "watch", messages: 1, bytes: 300 }],
        opened: 9,
        messages: 52,
        bytes: 4096,
        rateLimited: 1,
        refused: 2,
        windowEnded: 4,
        maxOpen: 8,
      },
    }),
  );
  await open();
  const streams = region("Streams and watches");
  const open_ = within(streams).getByRole("list", { name: "Open streams" });
  expect(within(open_).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "org.test.gitops/page:apps#1 read · revision 3 · 12 messages · 2.0 KiB",
  ]);
  const watches = within(streams).getByRole("list", { name: "Watches" });
  expect(within(watches).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "org.test.gitops/page:apps#2 watch · revision 3 · 1 message · 300 B",
  ]);
  expect(within(streams).queryByText("No open streams.")).toBeNull();
  expect(within(streams).queryByText("No watches.")).toBeNull();
  expect(fact("Streams and watches", "Opened since srelens started")).toBe("9");
  expect(fact("Streams and watches", "Received")).toBe("52 messages, 4.0 KiB");
  expect(fact("Streams and watches", "Stopped for exceeding the message rate")).toBe("1");
  expect(fact("Streams and watches", "Refused at the cap of 8 open streams")).toBe("2");
  expect(fact("Streams and watches", "Ended with their window")).toBe("4");
});

it("shows a running sidecar's process, memory against its limit, requests and latency", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(sidecar());
  await open();
  expect(fact("Process", "State")).toBe("Running");
  expect(fact("Process", "Sidecar API version")).toBe("0.5.0");
  expect(fact("Process", "PID")).toBe("4242");
  expect(fact("Process", "Running since")).toContain("09:30:00");
  expect(fact("Process", "Launches")).toBe("2");
  expect(fact("Process", "Unexpected exits")).toBe("1");
  expect(fact("Process", "Memory")).toBe("50 MiB of 256 MiB");
  expect(fact("Process", "Memory limit")).toBe("Enforced by the kernel");
  expect(fact("Process", "CPUs")).toBe("2");
  expect(fact("Process", "Requests")).toBe("120 answered · 3 failed · 1 timed out · 0 refused · 2 in flight");
  expect(fact("Process", "Latency")).toBe("p50 3 ms · p95 18.5 ms · max 40 ms, over 120 requests");
  expect(fact("Process", "Sidecar streams")).toBe("1 open of 16 · 7 opened");
  expect(screen.queryByRole("alert")).toBeNull();
});

it("says where memory is not measured, who enforces the limit, and when nothing was answered", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(
    sidecar({
      memory: { bytes: null, limitBytes: 256 * MiB, enforcement: "host" },
      rpc: { ...running.rpc, answered: 0, latency: { samples: 0, p50Ms: null, p95Ms: null, maxMs: null } },
    }),
  );
  const view = await open();
  expect(fact("Process", "Memory")).toBe("Not measured on this OS; the limit is 256 MiB");
  expect(fact("Process", "Memory limit")).toBe("Enforced by srelens, by sampling");
  expect(fact("Process", "Latency")).toBe("No requests answered yet");
  view.unmount();
  vi.mocked(inspectExtension).mockResolvedValue(sidecar({ memory: { bytes: 3 * MiB, limitBytes: 256 * MiB, enforcement: "missing" } }));
  await open();
  expect(fact("Process", "Memory limit")).toBe("Not enforced");
});

it("says which restart attempt of how many is next, in how long, and why", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(
    sidecar({
      state: "restarting",
      reason: "It exited with status 101.",
      pid: null,
      startedAt: null,
      apiVersion: null,
      restart: { attempt: 2, of: 5, delayMs: 4000 },
    }),
  );
  await open();
  expect(fact("Process", "State")).toBe("Restarting");
  expect(fact("Process", "Reason")).toBe("It exited with status 101.");
  expect(fact("Process", "Restart")).toBe("Attempt 2 of 5 in 4 s");
  expect(fact("Process", "PID")).toBe("None");
  expect(fact("Process", "Running since")).toBe("Not running");
  expect(fact("Process", "Sidecar API version")).toBe("Not negotiated");
});

it("shows a crashed sidecar's headline and reason with the supervisor's actions, and offers no restart", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(
    sidecar({
      state: "disabled",
      message: "Extension process exited unexpectedly",
      reason: "It exited with status 101 three times in a minute.",
      actions: ["restart", "viewLogs", "disable"],
      pid: null,
    }),
  );
  await open();
  const alert = screen.getByRole("alert");
  expect(within(alert).getByText("Extension process exited unexpectedly")).toBeTruthy();
  expect(within(alert).getByText("It exited with status 101 three times in a minute.")).toBeTruthy();
  expect(fact("Process", "State")).toBe("Disabled");
  expect(screen.queryByRole("button", { name: /restart/i })).toBeNull();
  fireEvent.click(within(alert).getByRole("button", { name: "View logs" }));
  expect(onViewLogs).toHaveBeenCalledTimes(1);
  fireEvent.click(within(alert).getByRole("button", { name: "Disable" }));
  expect(change).toHaveBeenCalledWith({ action: "enable", id: "org.test.gitops", enabled: false });
});

it("offers only the actions the supervisor lists", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(
    sidecar({ state: "disabled", message: "Extension process exited unexpectedly", reason: "Out of memory.", actions: ["viewLogs"] }),
  );
  await open();
  const alert = screen.getByRole("alert");
  expect(within(alert).getByRole("button", { name: "View logs" })).toBeTruthy();
  expect(within(alert).queryByRole("button", { name: "Disable" })).toBeNull();
});

it("shows recent errors with their local time, who wrote them and their text", async () => {
  const at = new Date(2026, 8, 27, 14, 3, 5, 42).getTime();
  vi.mocked(inspectExtension).mockResolvedValue(
    sidecar({}, {
      recentErrors: [
        { seq: 3, at, level: "error", source: "sidecar", text: "panic: index out of range" },
        { seq: 9, at: at + 1, level: "error", source: "host", text: "The request timed out after 30 s." },
      ],
    }),
  );
  await open();
  const errors = within(region("Recent errors")).getByRole("log", { name: "Recent errors" });
  expect(Array.from(errors.children).map((row) => row.textContent)).toEqual([
    "14:03:05.042 app panic: index out of range",
    "14:03:05.043 srelens The request timed out after 30 s.",
  ]);
  expect(region("Recent errors").textContent).not.toContain("No errors recorded");
});

it("says a sidecar has recorded no errors without pointing at pages", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(sidecar());
  await open();
  expect(region("Recent errors").textContent).toBe("Recent errorsNo errors recorded since srelens started.");
});

it("says the app is inactive, and why, when it is off, blocked or quarantined", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(declarative());
  const cases: Array<[Partial<InstalledExtension>, string]> = [
    [{ enabled: false }, "Inactive: it is not enabled."],
    [{ enabled: false, policyBlocked: "unsigned apps may not modify clusters" }, "Inactive: unsigned apps may not modify clusters."],
    [
      { enabled: false, quarantined: "signature does not match", policyBlocked: "unsigned apps may not modify clusters" },
      "Inactive: signature does not match. Remove it or reinstall it from the Catalog.",
    ],
  ];
  for (const [more, said] of cases) {
    const view = await open({ ...plugin, ...more });
    expect(region("Active contributions").textContent).toContain(said);
    expect(region("Active contributions").textContent).not.toContain("Active on");
    view.unmount();
  }
});

it("says which clusters the contributions are active on when the app is limited", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(declarative());
  const view = await open({ ...plugin, contexts: ["/kube/a#a", "/kube/b#b"] });
  expect(region("Active contributions").textContent).toContain("Active on the 2 clusters chosen in Overview.");
  view.unmount();
  await open({ ...plugin, contexts: ["/kube/a#a"] });
  expect(region("Active contributions").textContent).toContain("Active on the cluster chosen in Overview.");
});

it("says so when the manifest contributes nothing", async () => {
  vi.mocked(inspectExtension).mockResolvedValue(declarative());
  await open({
    ...plugin,
    manifest: { ...plugin.manifest, actions: undefined, contributions: { pages: [], detailTabs: [], detailLinks: [] } },
  });
  expect(region("Active contributions").textContent).toContain("This app contributes nothing.");
});

it("says it is loading before the first answer", async () => {
  vi.mocked(inspectExtension).mockImplementation(() => new Promise(() => {}));
  await open();
  expect(screen.getByRole("status").textContent).toBe("Loading…");
  expect(screen.queryByText(/No process/)).toBeNull();
  // The manifest's own facts need no read.
  expect(fact("Identity", "ID")).toBe("org.test.gitops");
});

it("reads again every five seconds while open, and stops when closed", async () => {
  vi.mocked(inspectExtension)
    .mockResolvedValueOnce(sidecar({ launches: 1 }))
    .mockResolvedValue(sidecar({ launches: 2 }));
  const view = await open();
  expect(fact("Process", "Launches")).toBe("1");
  await wait(4999);
  expect(inspectExtension).toHaveBeenCalledTimes(1);
  await wait(1);
  expect(inspectExtension).toHaveBeenCalledTimes(2);
  expect(fact("Process", "Launches")).toBe("2");
  view.unmount();
  await wait(20_000);
  expect(inspectExtension).toHaveBeenCalledTimes(2);
});

it("keeps the last read, labelled as such, when a later poll fails, and through a Retry", async () => {
  const crashed = sidecar({
    state: "disabled",
    reason: "It exited with status 101.",
    message: "Extension process exited unexpectedly",
    actions: ["restart", "viewLogs", "disable"],
  });
  vi.mocked(inspectExtension)
    .mockResolvedValueOnce(crashed)
    .mockRejectedValueOnce(new Error("bridge timed out"))
    .mockReturnValueOnce(new Promise(() => {}));
  await open();
  await wait(5000);
  const notice = screen.getAllByRole("alert").find((alert) => alert.textContent?.includes("Could not inspect this app"));
  expect(notice?.textContent).toContain("bridge timed out");
  // The crash, and what a person can do about it, stay on screen.
  expect(screen.getByText("Extension process exited unexpectedly")).toBeTruthy();
  expect(screen.getByRole("button", { name: "View logs" })).toBeTruthy();
  expect(screen.getByText(/^Showing the last read, from \d\d:\d\d:\d\d\. It may have changed since\.$/)).toBeTruthy();
  await wait(20_000);
  expect(inspectExtension).toHaveBeenCalledTimes(2);
  fireEvent.click(within(notice!).getByRole("button", { name: "Retry" }));
  await wait(0);
  expect(inspectExtension).toHaveBeenCalledTimes(3);
  expect(screen.getByText("Extension process exited unexpectedly")).toBeTruthy();
});

it("says a failed read failed, never that the app has no process or streams, and reads again on Retry", async () => {
  vi.mocked(inspectExtension)
    .mockRejectedValueOnce(new Error("bridge timed out"))
    .mockResolvedValue(declarative());
  await open();
  const alert = screen.getByRole("alert");
  expect(alert.textContent).toContain("Could not inspect this app");
  expect(alert.textContent).toContain("bridge timed out");
  expect(screen.queryByText(/No process/)).toBeNull();
  expect(screen.queryByText("No open streams.")).toBeNull();
  expect(screen.queryByText(/No errors recorded/)).toBeNull();
  await wait(10_000);
  expect(inspectExtension).toHaveBeenCalledTimes(1);
  fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  await wait(0);
  expect(inspectExtension).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByText("No open streams.")).toBeTruthy();
});
