import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  extensionLogs: vi.fn(),
}));
import { extensionLogs, type ExtensionLogLine, type ExtensionLogRead, type InstalledExtension } from "@srelens/core";
import { ExtensionLogs } from "./ExtensionLogs";

const plugin: InstalledExtension = {
  manifest: {
    id: "org.test.sidecar",
    name: "Sidecar app",
    version: "1.2.0",
    srelensApiVersion: "^0.5",
    kind: "declarative",
    permissions: [],
    capabilities: [],
    contributions: { pages: [], detailTabs: [], detailLinks: [] },
  },
  enabled: true,
  revision: 3,
  grants: [],
  settings: {},
  source: "catalog",
  installedAt: 1_700_000_000,
  history: [],
};

/** A local wall-clock time, so the expected clock reads the same in every time zone. */
const at = (hours: number, minutes: number, seconds: number, ms: number) =>
  new Date(2026, 8, 27, hours, minutes, seconds, ms).getTime();
const line = (seq: number, level: ExtensionLogLine["level"], text: string, source: ExtensionLogLine["source"] = "sidecar"): ExtensionLogLine => ({
  seq,
  at: at(14, 3, 5, seq),
  level,
  source,
  text,
});
const read = (lines: ExtensionLogLine[], more: Partial<ExtensionLogRead> = {}): ExtensionLogRead => ({
  runtime: "sidecar",
  lines,
  capacity: 2000,
  dropped: 0,
  ...more,
});

const log = () => screen.getByRole("log", { name: "Sidecar app log" });
const rows = () => Array.from(log().querySelectorAll(".extension-app-log-line")).map((row) => row.textContent);

async function open() {
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(<ExtensionLogs plugin={plugin} />);
  });
  return view;
}
const wait = (ms: number) => act(async () => {
  await vi.advanceTimersByTimeAsync(ms);
});

beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

it("says a declarative app has no process to write its log, and that the log stays in memory", async () => {
  vi.mocked(extensionLogs).mockResolvedValue(read([], { runtime: "declarative" }));
  await open();
  expect(extensionLogs).toHaveBeenCalledWith("org.test.sidecar", { minLevel: "trace" });
  expect(screen.getByText("This app has no process, so nothing writes to its log.")).toBeTruthy();
  expect(
    screen.getByText(
      "Kept in srelens's memory only: never written to disk, and never sent to an agent or anywhere else. The last 2,000 lines are kept.",
    ),
  ).toBeTruthy();
  expect(screen.queryByText("Nothing logged yet.")).toBeNull();
});

it("says nothing is logged yet for a sidecar that has written nothing", async () => {
  vi.mocked(extensionLogs).mockResolvedValue(read([]));
  await open();
  expect(screen.getByText("Nothing logged yet.")).toBeTruthy();
  expect(screen.queryByText(/has no process/)).toBeNull();
});

it("shows each line's local time to the millisecond, its level and writer as words, and its text", async () => {
  vi.mocked(extensionLogs).mockResolvedValue(
    read([line(7, "warn", "cache miss for org/app"), line(8, "error", "sidecar handshake refused", "host")]),
  );
  await open();
  expect(rows()).toEqual([
    "14:03:05.007 warn app cache miss for org/app",
    "14:03:05.008 error srelens sidecar handshake refused",
  ]);
  const first = log().querySelector(".extension-app-log-line")!;
  expect(within(first as HTMLElement).getByText("warn").getAttribute("data-level")).toBe("warn");
});

it("draws a line's control and direction characters as escapes, never as text that reorders the row", async () => {
  vi.mocked(extensionLogs).mockResolvedValue(read([line(1, "info", "user \u202Enimda\u202C logged in\u0007")]));
  await open();
  expect(rows()[0]).toContain(String.raw`user \u202enimda\u202c logged in\u0007`);
  expect(log().textContent).not.toContain("\u202E");
});

it("polls every two seconds for the lines after the last one it has, and appends them", async () => {
  vi.mocked(extensionLogs)
    .mockResolvedValueOnce(read([line(1, "info", "starting"), line(2, "info", "listening")]))
    .mockResolvedValueOnce(read([line(3, "warn", "slow reply")]))
    .mockResolvedValueOnce(read([]))
    .mockResolvedValue(read([]));
  await open();
  expect(rows()).toHaveLength(2);
  await wait(1999);
  expect(extensionLogs).toHaveBeenCalledTimes(1);
  await wait(1);
  expect(extensionLogs).toHaveBeenLastCalledWith("org.test.sidecar", { after: 2, minLevel: "trace" });
  expect(rows()).toEqual([
    "14:03:05.001 info app starting",
    "14:03:05.002 info app listening",
    "14:03:05.003 warn app slow reply",
  ]);
  await wait(2000);
  expect(extensionLogs).toHaveBeenLastCalledWith("org.test.sidecar", { after: 3, minLevel: "trace" });
  // An empty answer keeps its place rather than asking for everything again.
  await wait(2000);
  expect(extensionLogs).toHaveBeenLastCalledWith("org.test.sidecar", { after: 3, minLevel: "trace" });
  expect(rows()).toHaveLength(3);
});

it("keeps the newest thousand lines", async () => {
  const first = Array.from({ length: 1000 }, (_, index) => line(index + 1, "info", `line ${index + 1}`));
  vi.mocked(extensionLogs)
    .mockResolvedValueOnce(read(first))
    .mockResolvedValueOnce(read([line(1001, "info", "line 1001"), line(1002, "info", "line 1002")]))
    .mockResolvedValue(read([]));
  await open();
  await wait(2000);
  const all = rows();
  expect(all).toHaveLength(1000);
  expect(all[0]).toContain("line 3");
  expect(all[999]).toContain("line 1002");
});

it("reloads from the start at the chosen minimum level, among the five levels", async () => {
  vi.mocked(extensionLogs)
    .mockResolvedValueOnce(read([line(1, "debug", "tick"), line(2, "error", "boom")]))
    .mockResolvedValueOnce(read([line(2, "error", "boom")]))
    .mockResolvedValue(read([]));
  await open();
  const levels = screen.getByRole("group", { name: "Minimum level" });
  expect(within(levels).getAllByRole("button").map((button) => button.textContent)).toEqual([
    "trace",
    "debug",
    "info",
    "warn",
    "error",
  ]);
  expect(within(levels).getByRole("button", { name: "trace" }).getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(within(levels).getByRole("button", { name: "warn" }));
  await wait(0);
  expect(extensionLogs).toHaveBeenLastCalledWith("org.test.sidecar", { after: 0, minLevel: "warn" });
  expect(within(levels).getByRole("button", { name: "warn" }).getAttribute("aria-pressed")).toBe("true");
  expect(rows()).toEqual(["14:03:05.002 error app boom"]);
  await wait(2000);
  expect(extensionLogs).toHaveBeenLastCalledWith("org.test.sidecar", { after: 2, minLevel: "warn" });
});

it("says no line reaches the chosen level, rather than that nothing was logged", async () => {
  vi.mocked(extensionLogs)
    .mockResolvedValueOnce(read([line(1, "debug", "tick")]))
    .mockResolvedValue(read([]));
  await open();
  fireEvent.click(screen.getByRole("button", { name: "error" }));
  await wait(0);
  expect(screen.getByText("No lines at error or above.")).toBeTruthy();
  expect(screen.queryByText("Nothing logged yet.")).toBeNull();
});

it("ignores an answer for a level that is no longer chosen", async () => {
  let answerTrace!: (answer: ExtensionLogRead) => void;
  vi.mocked(extensionLogs)
    .mockImplementationOnce(() => new Promise((resolve) => (answerTrace = resolve)))
    .mockResolvedValueOnce(read([line(5, "error", "boom")]))
    .mockResolvedValue(read([]));
  await open();
  fireEvent.click(screen.getByRole("button", { name: "error" }));
  await wait(0);
  await act(async () => answerTrace(read([line(4, "debug", "tick")])));
  expect(rows()).toEqual(["14:03:05.005 error app boom"]);
});

it("says how many older lines the host dropped", async () => {
  vi.mocked(extensionLogs).mockResolvedValue(read([line(1204, "info", "ok")], { capacity: 1000, dropped: 203 }));
  const view = await open();
  expect(
    screen.getByText(
      "Kept in srelens's memory only: never written to disk, and never sent to an agent or anywhere else. The last 1,000 lines are kept; 203 older lines were dropped.",
    ),
  ).toBeTruthy();
  view.unmount();
  vi.mocked(extensionLogs).mockResolvedValue(read([line(1002, "info", "ok")], { capacity: 1000, dropped: 1 }));
  await open();
  expect(screen.getByText(/The last 1,000 lines are kept; 1 older line was dropped\.$/)).toBeTruthy();
});

it("says a failed read failed, never that the log is empty, and reads again on Retry", async () => {
  vi.mocked(extensionLogs)
    .mockRejectedValueOnce(new Error("bridge timed out"))
    .mockResolvedValue(read([line(1, "info", "listening")]));
  await open();
  const alert = screen.getByRole("alert");
  expect(alert.textContent).toContain("Could not read this app's log");
  expect(alert.textContent).toContain("bridge timed out");
  expect(screen.queryByText("Nothing logged yet.")).toBeNull();
  expect(screen.queryByText(/has no process/)).toBeNull();
  // No poll while the failure stands: the reader chooses when to try again.
  await wait(4000);
  expect(extensionLogs).toHaveBeenCalledTimes(1);
  fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  await wait(0);
  expect(extensionLogs).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("alert")).toBeNull();
  expect(rows()).toEqual(["14:03:05.001 info app listening"]);
});

it("keeps the lines it has when a later poll fails, and resumes after them on Retry", async () => {
  vi.mocked(extensionLogs)
    .mockResolvedValueOnce(read([line(1, "info", "listening")]))
    .mockRejectedValueOnce(new Error("bridge timed out"))
    .mockResolvedValue(read([line(2, "info", "served")]));
  await open();
  await wait(2000);
  expect(screen.getByRole("alert").textContent).toContain("bridge timed out");
  expect(rows()).toEqual(["14:03:05.001 info app listening"]);
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await wait(0);
  expect(extensionLogs).toHaveBeenLastCalledWith("org.test.sidecar", { after: 1, minLevel: "trace" });
  expect(rows()).toHaveLength(2);
});

it("stops polling when it is closed", async () => {
  vi.mocked(extensionLogs).mockResolvedValue(read([]));
  const view = await open();
  view.unmount();
  await wait(10_000);
  expect(extensionLogs).toHaveBeenCalledTimes(1);
});

it("follows new lines while the reader is at the bottom, and stays put once they scroll up", async () => {
  vi.mocked(extensionLogs)
    .mockResolvedValueOnce(read([line(1, "info", "one")]))
    .mockResolvedValueOnce(read([line(2, "info", "two")]))
    .mockResolvedValueOnce(read([line(3, "info", "three")]))
    .mockResolvedValue(read([]));
  await open();
  const region = log();
  let height = 400;
  Object.defineProperty(region, "scrollHeight", { configurable: true, get: () => height });
  Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 100 });
  // At the bottom: the next lines keep it there.
  height = 500;
  await wait(2000);
  expect(region.scrollTop).toBe(500);
  // Scrolled up to read: new lines do not pull it away.
  region.scrollTop = 120;
  fireEvent.scroll(region);
  height = 600;
  await wait(2000);
  expect(region.scrollTop).toBe(120);
});
