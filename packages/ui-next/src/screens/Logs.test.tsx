import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * The two things this screen reaches outside itself for: the subject lookup
 * that turns a route into pods and containers, and the stream hook that owns
 * the buffer. Both are mocked wholesale.
 *
 * Mocking the HOOK rather than `startLogStream` underneath it is deliberate.
 * `logStream.test.ts` already pins what the hook does with a burst, a pause
 * and an unmount; what is left for this suite is what the SCREEN does with
 * what the hook returns — which lines it draws, which it filters out, what it
 * asks the hook for, and where the viewport ends up when new ones arrive.
 * Driving that through a real subscription would put a backend transport in
 * the way of every one of those assertions.
 */
const h = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  return {
    listeners,
    version: 0,
    /** What the hook currently reports, mutated by the helpers below. */
    state: {
      lines: [] as { source: string; text: string }[],
      dropped: 0,
      status: "live" as "connecting" | "live" | "reconnecting" | "error",
      error: undefined as { title: string; detail: string; raw: string } | undefined,
      paused: false,
      pending: 0,
      /** Whether the screen holds the view, and the lines it holds it on: see `holdStream`. */
      held: false,
      heldLines: [] as { source: string; text: string }[],
      /** Per-target truth: how many are streaming, how many are down, of how
       *  many altogether. */
      live: 4,
      completed: 0,
      reconnecting: 0,
      total: 4,
      restarts: 0,
    },
    /** Whether the app believes it is inside the Tauri shell. */
    tauri: true,
    /** Every `saveTextFile` the screen has asked for. */
    saved: vi.fn(),
    /** Every one-shot `podLogs` fetch the previous-instance toggle has made. */
    fetched: vi.fn(),
    /** Every `listResource` the bare route's recents have been checked with. */
    listed: vi.fn(),
    /** The field of every `facetCounts` pass the rail has run: one entry per recount. */
    facetPasses: [] as string[],
    /** Every call the screen has made into the hook, in order. */
    seen: [] as {
      context: string;
      namespace: string;
      targets: { pod: string; container?: string; label?: string }[];
      options: {
        sinceSeconds?: number;
        tailLines?: number;
        timestamps?: boolean;
        source?: { key: string; open: (...a: never[]) => Promise<unknown> };
      };
    }[],
    /** The installed apps the log view may take a source from (#569). */
    plugins: [] as unknown[],
    /** Why the inventory could not be read, when a test says it could not. */
    inventoryError: undefined as string | undefined,
    /** Every app view a provider source opened, with its streams. */
    views: [] as { app: string; label: string; close: ReturnType<typeof vi.fn>; opened: { request: unknown; handlers: { onEnd?: (end: unknown) => void } }[] }[],
    resolve: vi.fn(),
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
});

vi.mock("../lib/logSubject", async (orig) => ({
  ...(await orig<typeof import("../lib/logSubject")>()),
  resolveLogSubject: (...a: unknown[]) => h.resolve(...a),
}));

vi.mock("../lib/logStream", async () => {
  const { useSyncExternalStore } = await import("react");
  return {
    useLogStream: (
      context: string,
      namespace: string,
      targets: { pod: string; container?: string; label?: string }[],
      options: { sinceSeconds?: number; tailLines?: number; timestamps?: boolean; source?: never } = {},
    ) => {
      useSyncExternalStore(
        h.subscribe,
        () => h.version,
        () => h.version,
      );
      h.seen.push({ context, namespace, targets, options });
      return {
        lines: h.state.held ? h.state.heldLines : h.state.lines,
        dropped: h.state.dropped,
        status: h.state.status,
        error: h.state.error,
        paused: h.state.paused,
        held: h.state.held,
        hold: holdStream,
        pendingWhilePaused: h.state.pending,
        liveTargets: h.state.live,
        completedTargets: h.state.completed,
        reconnectingTargets: h.state.reconnecting,
        totalTargets: h.state.total,
        togglePause: () => {
          h.state.paused = !h.state.paused;
          h.state.pending = 0;
          notify();
        },
        clear: () => {
          h.state.lines = [];
          notify();
        },
        restartCount: h.state.restarts,
      };
    },
  };
});

/**
 * The two platform calls the Export button lands on. Mocked at the core
 * boundary rather than at `window`, because which of the two runs is a
 * decision the screen makes from `isTauri()` and that decision is the thing
 * under test.
 */
vi.mock("@srelens/core", async (orig) => {
  const real = await orig<typeof import("@srelens/core")>();
  return {
    ...real,
    // Calls through; only records which facet each pass recounted.
    facetCounts: (...a: Parameters<typeof real.facetCounts>) => {
      h.facetPasses.push(a[2]);
      return real.facetCounts(...a);
    },
    isTauri: () => h.tauri,
    saveTextFile: (...a: unknown[]) => h.saved(...a),
    podLogs: (...a: unknown[]) => h.fetched(...a),
    // The bare route's only backend call: one list per kind and namespace, to
    // find out which remembered subjects the cluster still has.
    listResource: (...a: unknown[]) => h.listed(...a),
    // A log provider's source (#569): the app view it opens its stream on, and the
    // kubeconfig listing that says which cluster an app is enabled for.
    listContexts: async () => ({ contexts: [{ name: "prod-eu", key: "prod" }] }),
    openExtensionView: (app: string, label: string) => {
      const opened: { request: unknown; handlers: { onEnd?: (end: unknown) => void } }[] = [];
      const view = { app, label, close: vi.fn(async () => {}), opened };
      h.views.push(view);
      return {
        view: `${app}/${label}`,
        close: view.close,
        open: async (request: unknown, handlers: { onEnd?: (end: unknown) => void }) => {
          opened.push({ request, handlers });
          return { stream: "s-1", cancel: vi.fn(async () => {}) };
        },
      };
    },
  };
});

vi.mock("../extensions/inventoryStore", async (orig) => ({
  ...(await orig<typeof import("../extensions/inventoryStore")>()),
  useExtensions: () =>
    h.inventoryError
      ? { status: "error", error: h.inventoryError, reload: () => {} }
      : { status: "ready", data: { schemaVersion: 1, nextRevision: 9, plugins: h.plugins }, reload: () => {} },
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import type { ClusterContext, LogTarget } from "@srelens/core";
import type { LogSubjectPod, PreviousInstance } from "../lib/logSubject";
import { absoluteTimestamp, describeError } from "@srelens/core";
import { toneColor } from "@srelens/ui-kit";
import { Logs, logsRoute, parseLogsRoute } from "./Logs";
import { ConsoleProvider, useConsole } from "../console";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { loadRecentLogSubjects, recentLogSubjects, rememberLogSubject } from "../lib/logRecents";
import { liveLogStreams } from "../lib/liveLogStreams";
import { defaultState } from "../lib/tabs";
import { TabScope } from "../lib/tabScope";
import * as store from "../lib/tabsStore";

/** Push the version forward and wake every mounted hook. */
function notify() {
  h.version += 1;
  for (const listener of [...h.listeners]) listener();
}

/**
 * The hook's `hold`, as its contract has it (`logStream.test.ts` pins the real one): while held, the screen is
 * handed the lines it had when the hold began, whatever arrives; on release, the lines as they are now.
 */
function holdStream(on: boolean) {
  if (h.state.held === on) return;
  h.state.held = on;
  h.state.heldLines = h.state.lines;
  notify();
}

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod", key: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

/**
 * Three pods, four containers — the shape the header's pod count exists for.
 * `api-7` runs two containers, so a naive `targets.length` would call this
 * four pods.
 */
const TARGETS: LogTarget[] = [
  { pod: "api-7", container: "api", label: "api-7/api" },
  { pod: "api-7", container: "otel-sidecar", label: "api-7/otel-sidecar" },
  { pod: "api-8", container: "api", label: "api-8/api" },
  { pod: "api-9", container: "api", label: "api-9/api" },
];

/**
 * The per-pod facts `resolveLogSubject` hands back with the targets — one
 * entry per pod, on core's severity vocabulary, with the `pod-template-hash`
 * where there is one.
 */
const PODS: LogSubjectPod[] = [
  { name: "api-7", health: "danger", revision: "7d764666f9" },
  { name: "api-8", health: "warning", revision: "7d764666f9" },
  // No revision at all — a pod whose controller stamps no pod-template-hash.
  { name: "api-9", health: "success" },
];

const ROUTE = logsRoute("Deployment", "checkout", "checkout-api");

/** A stream line as the backend sends it with `timestamps: true`. */
const line = (source: string, ts: string, text: string) => ({
  source,
  text: `2026-08-24T${ts}Z ${text}`,
});

/**
 * The `HH:MM:SS.mmm` the time column draws for an instant: the reader's own
 * clock, so the expectation holds in whatever zone the suite runs in. Built
 * from local `Date` getters on purpose — it must not share the code under test.
 */
const localTs = (iso: string) => {
  const at = new Date(iso);
  const two = (n: number) => String(n).padStart(2, "0");
  return `${two(at.getHours())}:${two(at.getMinutes())}:${two(at.getSeconds())}.${String(at.getMilliseconds()).padStart(3, "0")}`;
};

const LINES = [
  line("api-7/api", "14:07:41.208000000", "info starting checkout-api build=4f2a1c"),
  line("api-7/otel-sidecar", "14:07:42.100000000", "warn exporter queue is full"),
  line("api-8/api", "14:07:43.900000000", "error pool timeout waited=30.0s in_use=5"),
  line("api-9/api", "14:07:44.010000000", "GET /healthz 200 1ms"),
];

/** Three JSON lines like the issue's screenshot: a logger's level, msg and fields. */
const JSON_LINES = [
  line("api-7/api", "14:07:41.208000000", '{"level":"info","msg":"Wrote batch","system":"camlog","internal_use":true}'),
  line("api-8/api", "14:07:42.100000000", '{"level":"warn","msg":"Skipping checkpoint","system":"schwarz"}'),
  line("api-9/api", "14:07:43.900000000", '{"level":"error","msg":"Success: Write failed","system":"camlog"}'),
];

/**
 * The corpse: one container of one pod has died and been restarted, and
 * `resolveLogSubject` reports it off the pod object it already fetched.
 */
const TERMINATED: PreviousInstance[] = [
  {
    pod: "api-7",
    container: "api",
    exitCode: 137,
    reason: "OOMKilled",
    finishedAt: "2026-08-24T14:07:42Z",
  },
];

/**
 * What `podLogs(..., { previous: true })` hands back: ONE BLOB, stamped,
 * with the trailing newline a text file ends on.
 *
 * The design's previous-instance buffer, verbatim in its levels: the instance
 * starts, saturates, times out, and is killed.
 */
const PREVIOUS_BLOB = [
  "2026-08-24T14:07:11.004000000Z info starting checkout-api build=4f2a1c pool_size=5",
  "2026-08-24T14:07:12.880000000Z warn pool saturated within 1.8s of accepting traffic",
  "2026-08-24T14:07:41.902000000Z error pool timeout waited=30.0s pool_size=5 in_use=5",
  "2026-08-24T14:07:42.410000000Z fatal liveness deadline exceeded, terminating",
  "",
].join("\n");

beforeEach(() => {
  vi.clearAllMocks();
  h.listeners.clear();
  h.version = 0;
  h.seen = [];
  h.plugins = [];
  h.inventoryError = undefined;
  h.views = [];
  h.state = {
    lines: [],
    dropped: 0,
    status: "live",
    error: undefined,
    paused: false,
    pending: 0,
    held: false,
    heldLines: [],
    live: 4,
    completed: 0,
    reconnecting: 0,
    total: 4,
    restarts: 0,
  };
  h.tauri = true;
  h.saved.mockResolvedValue("/home/u/checkout-api.log");
  h.fetched.mockResolvedValue({ logs: PREVIOUS_BLOB });
  // Everything the recents name is still there unless a test says otherwise.
  h.listed.mockResolvedValue({ items: [{ name: "checkout-api", namespace: "checkout", age: "1d" }] });
  // No corpse by default: a pod on its first run is the common case, and the
  // one the previous-instance toggle has to have an answer for.
  h.resolve.mockResolvedValue({ status: "resolved", targets: TARGETS, pods: PODS, previous: [] });

  resetContexts();
  setContexts([CTX]);
  setKubeconfigFiles(["/home/u/.kube/config"]);
  store.setState(defaultState([CTX]));
  // A Map and no platform, the way every store in `lib/` is tested.
  loadRecentLogSubjects(fakeStorage());
});

/** Storage as a Map: the shape `settingsStorage` is injected as in tests. */
function fakeStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  };
}

let asked: string[] = [];

/** Records what the ask chip put to the console, which has no dock here. */
function AskSpy() {
  const console_ = useConsole();
  console_.registerSubmit((question) => {
    asked.push(question);
  });
  return null;
}

function draw(route = ROUTE) {
  asked = [];
  return render(
    <ConsoleProvider>
      <AskSpy />
      <Logs route={route} />
    </ConsoleProvider>,
  );
}

/** The rendered lines as `ts|source|level|message`, in order. */
/**
 * The drawn rows, read from the log region that is in the document NOW.
 *
 * The node is re-read rather than trusted: `body()` hands back the element as
 * it was, and a resolution still in flight can re-render and replace it. A
 * detached node answers every query with nothing, so the assertion sees an
 * empty body and reads as "the screen drew none of them" — which is how three
 * tests in this file flaked under parallel load while passing alone every
 * time. The argument is kept because it reads naturally at the call sites,
 * and used only while it is still attached.
 */
const rendered = (region?: HTMLElement) => {
  const live = region?.isConnected ? region : screen.queryByRole("log", { name: /logs/i });
  if (!live) return [];
  return Array.from(live.querySelectorAll(".logline")).map((el) =>
    ["ts", "source", "level", "message"]
      .map((slot) => el.querySelector(`[data-slot=${slot}]`)?.textContent ?? "")
      .join("|"),
  );
};

/** Push lines into the buffer the screen is rendering. */
function push(...lines: { source: string; text: string; truncated?: true }[]) {
  act(() => {
    h.state.lines = [...h.state.lines, ...lines];
    notify();
  });
}

/**
 * The log region, once the stream behind it is listening.
 *
 * `findByRole` returns on the commit that mounts the stream, and that commit
 * lands outside `act`: it comes from `resolveLogSubject`'s promise. So its
 * passive effects, the stream hook's `useSyncExternalStore` subscription among
 * them, run when React's scheduler gets round to them, not before the test's
 * next line. A test that pushed lines or a restart through `notify()` in that
 * gap reached no subscriber, and the screen went on drawing what it mounted
 * with: an empty rail, no restart notice. Testing Library's own drain is a
 * `setTimeout(0)` and the scheduler's is a `setImmediate`; which runs first is
 * up to the machine, which is why these failed about one run in six under
 * full-suite load and passed alone. (#361)
 *
 * The barrier is the subscription itself. An empty `act` stood here before,
 * and it usually gave the scheduler enough time: with the scheduler's tasks
 * held back 60ms, 28 of this file's tests failed on every run, the dismiss,
 * Sources-row and stripped-message failures CI had seen among them.
 *
 * **The ONLY barrier in this file, and the only `findByRole("log")` in it.**
 * The helper existed for thirty-five tests and was used by twenty-eight of
 * them; the other thirty-five sites still awaited `findByRole` raw, with no
 * barrier at all. #375 is one of those: "draws a Sources row per pod" pushed
 * lines before the stream had subscribed, and the rail tallied an empty
 * buffer — a failure that reached CI twice, on #373 and again on #380. A raw
 * `findByRole("log")` added back here is that flake returning, so there is
 * exactly one left and it is the line below.
 */
const body = async () => {
  await screen.findByRole("log", { name: /logs/i });
  await waitFor(() => expect(h.listeners.size).toBeGreaterThan(0));
  return screen.getByRole("log", { name: /logs/i });
};

/** The last options the screen asked the stream hook for. */
const lastOptions = () => h.seen[h.seen.length - 1].options;

/**
 * Give `.logline` rows a real height, so the screen's windowing actually
 * engages.
 *
 * `computeLogWindow` needs a measured row height before it will window
 * anything, and jsdom measures nothing — which means every test that does NOT
 * call this runs against the "render everything" bail-out, where the drawn
 * window and the filtered buffer are the same array and nothing can tell them
 * apart. Only `.logline` is given a height, so the measurement path under test
 * is the real one: the screen finds the first row itself.
 */
function measurableRows() {
  const proto = window.HTMLElement.prototype;
  const original = proto.getBoundingClientRect;
  proto.getBoundingClientRect = function (this: HTMLElement) {
    if (this.classList.contains("logline")) return { height: 20 } as DOMRect;
    return original.call(this);
  };
  return () => {
    proto.getBoundingClientRect = original;
  };
}

// jsdom has no scrollIntoView; keyboard selection reveals the row through it.
HTMLElement.prototype.scrollIntoView ??= () => {};

describe("the logs route", () => {
  it("round-trips a subject through the route it mints", () => {
    expect(parseLogsRoute(logsRoute("Deployment", "checkout", "checkout-api"))).toEqual({
      kind: "Deployment",
      namespace: "checkout",
      name: "checkout-api",
    });
  });

  it("survives a name with a slash in it", () => {
    const route = logsRoute("Pod", "kube-system", "weird/name");
    expect(parseLogsRoute(route)).toEqual({ kind: "Pod", namespace: "kube-system", name: "weird/name" });
  });

  it("refuses a bare route and the list route it is not", () => {
    expect(parseLogsRoute("/logs")).toBeNull();
    expect(parseLogsRoute("/logs/checkout")).toBeNull();
    expect(parseLogsRoute("/k/pods/checkout/api-7")).toBeNull();
  });

  it("refuses a segment that cannot be decoded, rather than throwing", () => {
    // `decodeURIComponent` THROWS on a malformed escape, and this parser runs
    // during render on every restored tab — `describe` and `screenFor` both
    // call it, and tab routes are persisted. One corrupted route in storage
    // would take the whole window down on boot. "Unrecognised route" is
    // already a normal outcome of this function; a bad escape is one more of
    // them, not a new path.
    expect(parseLogsRoute("/logs/Pod/checkout/%zz")).toBeNull();
    expect(parseLogsRoute("/logs/%e0%a4%a/checkout/api-7")).toBeNull();
    expect(parseLogsRoute("/logs/Pod/%/api-7")).toBeNull();
    // The last segment decodes fine; the parser must still refuse the route
    // rather than returning a half-decoded subject.
    expect(parseLogsRoute("/logs/Pod/%zz/api-7")).toBeNull();
  });
});

describe("Logs", () => {
  it("names the subject and counts PODS, not containers", async () => {
    draw();
    expect(await screen.findByText("prod-eu / checkout / checkout-api · 3 pods")).toBeTruthy();
  });

  it("says one pod in the singular", async () => {
    h.resolve.mockResolvedValue({
      status: "resolved",
      targets: [{ pod: "api-7", container: "api", label: "" }],
      pods: [{ name: "api-7", health: "success" }],
      previous: [],
    });
    draw(logsRoute("Pod", "checkout", "api-7"));
    expect(await screen.findByText("prod-eu / checkout / api-7 · 1 pod")).toBeTruthy();
  });

  it("draws each line with its time, source, LEVEL WORD and message", async () => {
    const region = await (draw(), body());
    push(...LINES);
    expect(rendered(region)).toEqual([
      // The message is the line as it was written, level word and all. A log
      // viewer that edits the text out of a line breaks the one thing a reader
      // does with it — compare it against what they grepped for elsewhere.
      //
      // And the level column says what the LINE said — `warn`, not the
      // `warning` tone name, and `error`, not `danger`. A column printing the
      // severity vocabulary reports a word that appears nowhere in the log it
      // is describing (and `WARNING` does not fit its 44px gutter either).
      `${localTs("2026-08-24T14:07:41.208Z")}|api-7 · api|info|info starting checkout-api build=4f2a1c`,
      `${localTs("2026-08-24T14:07:42.100Z")}|api-7 · otel-sidecar|warn|warn exporter queue is full`,
      `${localTs("2026-08-24T14:07:43.900Z")}|api-8 · api|error|error pool timeout waited=30.0s in_use=5`,
      `${localTs("2026-08-24T14:07:44.010Z")}|api-9 · api||GET /healthz 200 1ms`,
    ]);
  });

  it("says beside a line the host cut that it was cut, and keeps the line as it came (#747)", async () => {
    const region = await (draw(), body());
    push({ ...line("api-7/api", "14:07:41.208000000", "info the start of it"), truncated: true }, LINES[1]);
    // The note follows the text the container wrote, up to the cut. It is the
    // screen's, drawn beside the line's text rather than in it, so the filter
    // and an export never see it.
    expect(rendered(region)).toEqual([
      `${localTs("2026-08-24T14:07:41.208Z")}|api-7 · api|info|info the start of it [line cut: too long]`,
      `${localTs("2026-08-24T14:07:42.100Z")}|api-7 · otel-sidecar|warn|warn exporter queue is full`,
    ]);
    const notes = Array.from(region.querySelectorAll(".logline")).map(
      (row) => row.querySelector("[data-slot=message] .text-faint")?.textContent ?? null,
    );
    expect(notes).toEqual([" [line cut: too long]", null]);
  });

  it("leaves the level's colour to the kit rather than pairing one itself", async () => {
    // The kit owns level→tone (`LEVEL_TONE`) precisely so the same word is
    // not red on one screen and grey on the next. The screen passes the word
    // and no `tone`, so what comes out is the kit's own mapping — asserted
    // against `toneColor`, not against a hex the screen could have written.
    await (draw(), body());
    push(...LINES);
    // Re-query the region rather than holding the one `body()` returned: a
    // resolution still in flight can re-render and replace that node, and a
    // detached element answers every query with nothing. This failed about
    // one run in a hundred, under full-package load, as an empty array.
    await waitFor(() => {
      const region = screen.getByRole("log", { name: /logs/i });
      const levels = Array.from(region.querySelectorAll<HTMLElement>("[data-slot=level]"));
      expect(levels.map((el) => el.style.color)).toEqual([
        toneColor("info"),
        toneColor("warn"),
        toneColor("sev"),
        // No level word: the kit's fallback, not a tone the screen chose.
        toneColor("muted"),
      ]);
    });
  });

  it("names the source of a single-target stream, which arrives unlabelled", async () => {
    // `resolveLogSubject` labels a line only when more than one target is in
    // scope — one pod, one container, no prefix. The column still has to say
    // which pod, or the design's source gutter is blank for every ordinary
    // single-container workload.
    h.resolve.mockResolvedValue({
      status: "resolved",
      targets: [{ pod: "api-7", container: "api", label: "" }],
      pods: [{ name: "api-7", health: "success" }],
      previous: [],
    });
    const region = await (draw(logsRoute("Pod", "checkout", "api-7")), body());
    push({ source: "", text: "a line with no timestamp on it" });
    expect(rendered(region)).toEqual(["|api-7 · api||a line with no timestamp on it"]);
  });

  it("names a target carrying no container by its pod alone, separator and all", async () => {
    // `LogTarget.container` is optional: a stream can be opened against a pod
    // and let the API pick its only container. There is then no container
    // name to say — and a gutter that draws the separator anyway prints
    // `api-7 · `, which an export turns into `api-7/ | …`. Both read as a
    // truncated fact rather than an absent one.
    h.resolve.mockResolvedValue({
      status: "resolved",
      targets: [{ pod: "api-7", label: "" }],
      pods: [{ name: "api-7", health: "success" }],
      previous: [],
    });
    const region = await (draw(logsRoute("Pod", "checkout", "api-7")), body());
    push({ source: "", text: "a line from a container nobody named" });
    expect(rendered(region)).toEqual(["|api-7||a line from a container nobody named"]);

    // And nothing to filter by, so the select is not offered at all.
    expect(screen.queryByRole("combobox", { name: /container/i })).toBeNull();
  });

  it("filters on the message and on the severity word, either case", async () => {
    const region = await (draw(), body());
    push(...LINES);
    const field = within(screen.getByRole("search", { name: /filter lines/i })).getByRole("searchbox");

    await userEvent.type(field, "POOL");
    expect(rendered(region).map((r) => r.split("|")[3])).toEqual([
      "error pool timeout waited=30.0s in_use=5",
    ]);

    await userEvent.clear(field);
    await userEvent.type(field, "warning");
    expect(rendered(region).map((r) => r.split("|")[3])).toEqual(["warn exporter queue is full"]);
  });

  it("filters by container", async () => {
    const region = await (draw(), body());
    push(...LINES);
    await userEvent.selectOptions(screen.getByRole("combobox", { name: /container/i }), "otel-sidecar");
    expect(rendered(region).map((r) => r.split("|")[3])).toEqual(["warn exporter queue is full"]);
  });

  it("reports completed sources separately from the live count", async () => {
    h.state.live = 3;
    h.state.completed = 1;
    await (draw(), body());
    expect(screen.getByText("Following — 3 of 4 streaming, 1 completed")).toBeDefined();
  });

  it("shows init-container lines and fetches that container's previous logs", async () => {
    const init = { pod: "api-7", container: "migrate", label: "api-7/migrate" };
    h.resolve.mockResolvedValue({
      status: "resolved", targets: [...TARGETS, init], pods: PODS,
      previous: [{ pod: "api-7", container: "migrate", exitCode: 1 }],
    });
    const region = await (draw(), body());
    push(...LINES, { source: init.label, text: "2026-08-24T14:07:11Z error migration failed" });
    await userEvent.selectOptions(screen.getByRole("combobox", { name: /container/i }), "migrate");
    expect(rendered(region).map((r) => r.split("|")[3])).toEqual(["error migration failed"]);
    await userEvent.click(screen.getByRole("button", { name: /previous instance/i }));
    await waitFor(() => expect(h.fetched).toHaveBeenCalledWith(
      expect.any(String), expect.any(String), "api-7", undefined,
      expect.objectContaining({ container: "migrate", previous: true }),
    ));
  });

  it("filters by container when the label names the pod alone", async () => {
    // The ORDINARY workload: a Deployment with three replicas of one
    // container. Every in-scope target shares that container name, so
    // `resolveLogSubject` labels each line with the POD ALONE — naming the
    // container on every row would repeat one word down the whole gutter.
    //
    // That is a decision about the DISPLAY. The container is still a fact
    // about the line, and the select is populated from the targets, which
    // carry it. Recovering the container by splitting the label finds no
    // slash, calls it `""`, and then picking `web` from the dropdown matches
    // no row at all: the screen goes blank on the commonest case there is.
    h.resolve.mockResolvedValue({
      status: "resolved",
      targets: [
        { pod: "web-0", container: "web", label: "web-0" },
        { pod: "web-1", container: "web", label: "web-1" },
      ],
      pods: [
        { name: "web-0", health: "success" },
        { name: "web-1", health: "success" },
      ],
      previous: [],
    });
    const region = await (draw(logsRoute("Deployment", "checkout", "web")), body());
    push(
      line("web-0", "14:07:41.208000000", "info serving on :8080"),
      line("web-1", "14:07:42.100000000", "warn upstream slow"),
    );

    await userEvent.selectOptions(screen.getByRole("combobox", { name: /container/i }), "web");
    expect(rendered(region)).toEqual([
      // Still there — and the gutter still names the pod ALONE, because the
      // fix carries the container as data beside the label rather than
      // putting it back into the string the reader sees.
      `${localTs("2026-08-24T14:07:41.208Z")}|web-0|info|info serving on :8080`,
      `${localTs("2026-08-24T14:07:42.100Z")}|web-1|warn|warn upstream slow`,
    ]);
  });

  it("hides the other container when a pod-only label is in play", async () => {
    // The other half of the same seam: a pod-only label must not become a
    // wildcard that matches EVERY container either. Two pods, one of which
    // also runs a sidecar, so the labels are `pod/container` — and one whose
    // line arrives under a pod-only label because it is the only target of
    // its kind. Picking `api` must drop the sidecar's line.
    h.resolve.mockResolvedValue({
      status: "resolved",
      targets: [
        { pod: "api-7", container: "api", label: "api-7" },
        { pod: "api-8", container: "api", label: "api-8" },
      ],
      pods: [
        { name: "api-7", health: "success" },
        { name: "api-8", health: "success" },
      ],
      previous: [],
    });
    const region = await (draw(), body());
    push(
      line("api-7", "14:07:41.208000000", "info from api-7"),
      // A line whose source matches no target at all — the stream tagged it
      // with something this screen never handed out. It must not be claimed
      // by a container filter that never named it.
      line("ghost-0/sidecar", "14:07:42.100000000", "warn from a stranger"),
    );

    await userEvent.selectOptions(screen.getByRole("combobox", { name: /container/i }), "api");
    expect(rendered(region).map((r) => r.split("|")[3])).toEqual(["info from api-7"]);
  });

  it("falls back to the placeholder on an undecodable route rather than dying", async () => {
    // What a corrupted persisted tab actually does to the window: the parser
    // is called during render, so a throw here is an unmounted app, not a
    // blank pane.
    draw("/logs/Pod/checkout/%zz");
    expect(await screen.findByText("Pick a workload or a pod to follow")).toBeTruthy();
  });

  it("wires the since select to what the stream is asked to tail", async () => {
    draw();
    await body();
    // Opens on the whole log; the narrowing is the reader's to ask for.
    expect(lastOptions().sinceSeconds).toBeUndefined();

    await userEvent.selectOptions(screen.getByRole("combobox", { name: /since/i }), "5m");
    await waitFor(() => expect(lastOptions().sinceSeconds).toBe(300));

    await userEvent.selectOptions(screen.getByRole("combobox", { name: /since/i }), "1h");
    await waitFor(() => expect(lastOptions().sinceSeconds).toBe(3600));

    await userEvent.selectOptions(screen.getByRole("combobox", { name: /since/i }), "all");
    await waitFor(() => expect(lastOptions().sinceSeconds).toBeUndefined());
  });

  it("asks for timestamps, because the design gives the time its own column", async () => {
    draw();
    await body();
    expect(lastOptions().timestamps).toBe(true);
  });

  it("strips a stamp that carries a zone offset, not only a Z", async () => {
    // A container runtime that stamps local time emits
    // `2026-08-25T08:13:41.721258+02:00`. Consuming the date and time but not
    // the offset left `+02:00 ` glued to the front of every message on the
    // screen — and the rail then tallied it as the stream's top term, 1993
    // times, which is to say once per line.
    h.state.lines = [{ source: "", text: "2026-08-25T08:13:41.721258+02:00 pool timeout waited=30.0s" }];
    const region = await (draw(), body());
    expect(within(region).getByText(/^pool timeout waited=30.0s$/)).toBeTruthy();
  });

  it("draws a line's time on the reader's clock, not the stamp's own", async () => {
    // The histogram axis, its caption and the window pill are all local, so a
    // time column left on the stamp's UTC wall time disagrees with them by the
    // reader's offset: a line at 22:00:29Z sat beside an axis reading 00:00:29.
    // An offset stamp is the same instant read the same way, not its own wall
    // time. -03:30 is no zone's August offset, so its written clock cannot
    // coincide with the machine's.
    const region = await (draw(), body());
    push(
      line("api-7/api", "14:07:41.208000000", "info stamped in Z"),
      { source: "api-8/api", text: "2026-08-24T10:37:42.100000000-03:30 info stamped with an offset" },
    );
    expect(rendered(region).map((r) => r.split("|")[0])).toEqual([
      localTs("2026-08-24T14:07:41.208Z"),
      localTs("2026-08-24T14:07:42.100Z"),
    ]);
  });

  it("leaves the time column empty for a line that arrived without a stamp", async () => {
    const region = await (draw(), body());
    push({ source: "api-7/api", text: "info no stamp on this one" });
    expect(rendered(region)).toEqual(["|api-7 · api|info|info no stamp on this one"]);
  });

  it("opens on the whole log, not the last five minutes", async () => {
    // Classic defaults to ALL and this screen replaces it, so a narrower
    // default is a regression a reader meets on their first visit: a workload
    // quiet for six minutes renders "Nothing has been logged yet", which is
    // true, useless, and reads as a broken screen. The tail cap already bounds
    // how much arrives, so the age window buys nothing and costs the logs.
    draw();
    await body();
    expect(lastOptions().sinceSeconds).toBeUndefined();
    expect((screen.getByRole("combobox", { name: /since/i }) as HTMLSelectElement).value).toBe("all");
  });

  it("lets an empty state wrap, though the lines around it must not", async () => {
    // The body is `whitespace-nowrap` so an unwrapped log line keeps its fixed
    // height — the windowing depends on it. EmptyState renders inside that
    // body, where its own `max-w-[42ch]` cannot take effect: the hint ran off
    // under the rail on a real cluster.
    h.state.lines = [];
    draw();
    const empty = await screen.findByText(/nothing has been logged yet/i);
    expect(empty.parentElement?.className ?? "").toContain("whitespace-normal");
  });

  describe("the three states the design leaves out", () => {
    it("says nothing has been logged yet, naming the window that decides it", async () => {
      const region = await (draw(), body());
      expect(within(region).getByText(/nothing has been logged yet/i)).toBeTruthy();
      // No window by default, so no window clause: the screen opens on the
      // whole log and has nothing to blame the silence on.
      expect(within(region).queryByText(/in the last/i)).toBeNull();

      // Narrow it, and the sentence names what is now hiding the lines —
      // which is the whole point of saying it.
      await userEvent.selectOptions(screen.getByRole("combobox", { name: /since/i }), "5m");
      expect(await within(region).findByText(/last 5m/i)).toBeTruthy();
    });

    it("says nothing MATCHES, which is a different sentence", async () => {
      const region = await (draw(), body());
      push(...LINES);
      const field = within(screen.getByRole("search", { name: /filter lines/i })).getByRole("searchbox");
      await userEvent.type(field, "zzz");

      expect(within(region).queryByText(/nothing has been logged yet/i)).toBeNull();
      expect(within(region).getByText(/no lines match/i)).toBeTruthy();
    });

    it("clears the filter from the state that reports it", async () => {
      const region = await (draw(), body());
      push(...LINES);
      const field = within(screen.getByRole("search", { name: /filter lines/i })).getByRole("searchbox");
      await userEvent.type(field, "zzz");
      await userEvent.click(within(region).getByRole("button", { name: /clear the filter/i }));
      expect(rendered(region)).toHaveLength(LINES.length);
    });

    it("reads a stream that could not start as friendly copy, not a raw error", async () => {
      h.state.status = "error";
      h.state.error = describeError("start_log_stream failed: request timeout");
      const region = await (draw(), body());
      const alert = within(region).getByRole("alert");
      expect(within(alert).getByText(/didn't respond in time/i)).toBeTruthy();
      // The original is still reachable, but only folded away behind
      // `RawError` — never as the sentence the reader is handed.
      expect(alert.querySelector("[data-slot=detail]")?.textContent).not.toContain(
        "start_log_stream failed",
      );
      expect(alert.querySelector("details")?.textContent).toContain("start_log_stream failed");
    });
  });

  describe("the subject", () => {
    it("says a workload has no pods rather than opening an empty stream", async () => {
      h.resolve.mockResolvedValue({
        status: "empty",
        detail: "Deployment/checkout-api has no pods to follow.",
      });
      draw();
      expect(await screen.findByText(/has no pods to follow/i)).toBeTruthy();
      expect(h.seen).toHaveLength(0);
    });

    it("reads a failed lookup through describeError", async () => {
      h.resolve.mockResolvedValue({
        status: "error",
        error: describeError("listing pods: request timeout"),
      });
      draw();
      const alert = await screen.findByRole("alert");
      expect(within(alert).getByText(/didn't respond in time/i)).toBeTruthy();
    });
  });

  describe("stick to bottom", () => {
    /**
     * jsdom does no layout, so the viewport's geometry has to be declared. The
     * heights are real numbers the component reads; `scrollTop` gets a backing
     * field so the component's own writes to it are observable — which is the
     * whole property under test.
     */
    function measurable(el: HTMLElement, rows: () => number) {
      let top = 0;
      Object.defineProperty(el, "clientHeight", { configurable: true, get: () => 100 });
      Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => rows() * 20 });
      Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => top,
        set: (v: number) => {
          top = v;
        },
      });
      return {
        get top() {
          return top;
        },
        to(v: number) {
          top = v;
          fireEvent.scroll(el);
        },
      };
    }

    const many = (n: number, from = 0) =>
      Array.from({ length: n }, (_, i) =>
        line("api-7/api", "14:07:41.208000000", `line ${from + i}`),
      );

    it("pins the newest line while the reader is at the bottom", async () => {
      const region = await (draw(), body());
      const view = measurable(region, () => h.state.lines.length);

      push(...many(20));
      expect(view.top).toBe(400);

      push(...many(5, 20));
      expect(view.top).toBe(500);
    });

    it("does NOT yank a reader who has scrolled up", async () => {
      const region = await (draw(), body());
      const view = measurable(region, () => h.state.lines.length);
      push(...many(20));

      view.to(0);
      push(...many(5, 20));
      expect(view.top).toBe(0);

      // Back to the bottom — 25 rows of 20px in a 100px viewport — and
      // following resumes without a control to press.
      view.to(400);
      push(...many(5, 25));
      expect(view.top).toBe(600);
    });
  });

  describe("the window over a long buffer", () => {
    const many = (n: number) =>
      Array.from({ length: n }, (_, i) => line("api-7/api", "14:07:41.208000000", `line ${i}`));

    it("draws only the slice on screen, and reserves the rest", async () => {
      const restore = measurableRows();
      try {
        const region = await (draw(), body());
        Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 100 });
        push(...many(300));

        const drawn = region.querySelectorAll(".logline");
        expect(drawn.length).toBeGreaterThan(0);
        expect(drawn.length).toBeLessThan(300);
        // The spacer that keeps the scrollbar honest about the 300 lines.
        const pads = Array.from(region.querySelectorAll("[aria-hidden=true]"));
        expect(pads.some((p) => (p as HTMLElement).style.height !== "")).toBe(true);
      } finally {
        restore();
      }
    });

    it("draws everything once lines wrap, because a wrapped row has no fixed height", async () => {
      const restore = measurableRows();
      try {
        const region = await (draw(), body());
        Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 100 });
        push(...many(300));
        await userEvent.click(screen.getByRole("button", { name: "Wrap" }));
        expect(region.querySelectorAll(".logline")).toHaveLength(300);
      } finally {
        restore();
      }
    });
  });

  describe("the pause toggle", () => {
    it("reads Pause while following and Follow while paused", async () => {
      draw();
      await body();
      await userEvent.click(screen.getByRole("button", { name: "Pause" }));
      expect(screen.getByRole("button", { name: "Follow" })).toBeTruthy();
    });

    it("says how many lines arrived while the view was held", async () => {
      draw();
      await body();
      await userEvent.click(screen.getByRole("button", { name: "Pause" }));
      act(() => {
        h.state.pending = 12;
        notify();
      });
      expect(screen.getByText(/12 new lines/i)).toBeTruthy();
    });
  });

  describe("what the stream is doing", () => {
    it("says Following while it is live", async () => {
      draw();
      await body();
      expect(signal()).toContain("Following");
    });

    it("says so when the connection is being retried", async () => {
      draw();
      await body();
      act(() => {
        h.state.status = "reconnecting";
        notify();
      });
      expect(signal()).toContain("Reconnecting");
    });
  });

  it("says how much of the buffer the ring has thrown away", async () => {
    const region = await (draw(), body());
    act(() => {
      h.state.lines = LINES;
      h.state.dropped = 1200;
      notify();
    });
    expect(within(region).getByText(/1 200 earlier lines/i)).toBeTruthy();
  });

  it("hands the stream to the console when asked about", async () => {
    draw();
    await body();
    await userEvent.click(screen.getByRole("button", { name: /Summarise this stream/i }));
    await waitFor(() =>
      expect(asked).toEqual(["Summarise the last 500 log lines and group errors by cause"]),
    );
  });

  it("asks which logs rather than showing none, on a bare route", async () => {
    draw("/logs");
    expect(await screen.findByText(/pick a workload or a pod/i)).toBeTruthy();
    expect(h.resolve).not.toHaveBeenCalled();
    // Nothing has ever been followed, so there is no list of ways in and no
    // reason to have asked the cluster anything.
    expect(document.querySelector('[data-slot="recents"]')).toBeNull();
    expect(h.listed).not.toHaveBeenCalled();
  });

  it("remembers the subject it streamed", async () => {
    draw();
    await body();
    await waitFor(() =>
      expect(recentLogSubjects("prod")).toEqual([
        { cluster: "prod", kind: "Deployment", namespace: "checkout", name: "checkout-api" },
      ]),
    );
  });

  it("does not remember a subject it could not open", async () => {
    h.resolve.mockResolvedValue({ status: "error", error: describeError("boom") });
    draw();
    await screen.findByText(/could not open logs/i);
    expect(recentLogSubjects("prod")).toEqual([]);
  });

  it("offers a remembered subject the cluster still has, and opens it", async () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject({ cluster: "prod", kind: "Deployment", namespace: "checkout", name: "checkout-api" }, s);
    draw("/logs");
    const row = await screen.findByRole("button", { name: /Deployment\/checkout-api/ });
    expect(h.listed).toHaveBeenCalledWith("prod-eu", "Deployment", "checkout");
    await userEvent.click(row);
    expect(store.currentWorkspace().tabs.map((t) => t.route)).toContain(
      logsRoute("Deployment", "checkout", "checkout-api"),
    );
  });

  it("offers nothing until the cluster has answered", async () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject({ cluster: "prod", kind: "Deployment", namespace: "checkout", name: "checkout-api" }, s);
    // A list that never answers: the subject may or may not still be there,
    // and a way in that errors when clicked is worse than none.
    h.listed.mockReturnValue(new Promise(() => {}));
    draw("/logs");
    await screen.findByText(/pick a workload or a pod/i);
    expect(screen.queryByRole("button", { name: /checkout-api/ })).toBeNull();
  });

  it("says a remembered workload is gone rather than offering it", async () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject({ cluster: "prod", kind: "Deployment", namespace: "checkout", name: "payments" }, s);
    h.listed.mockResolvedValue({ items: [{ name: "checkout-api", namespace: "checkout", age: "1d" }] });
    draw("/logs");
    const row = await screen.findByRole("button", { name: /Deployment\/payments/ });
    expect((row as HTMLButtonElement).disabled).toBe(true);
    expect(row.textContent).toMatch(/no longer on this cluster/i);
    // Kept: a Deployment's name outlives its pods and may come back.
    expect(recentLogSubjects("prod")).toHaveLength(1);
  });

  it("drops a pod the cluster has replaced, and forgets it", async () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject({ cluster: "prod", kind: "Deployment", namespace: "checkout", name: "checkout-api" }, s);
    rememberLogSubject({ cluster: "prod", kind: "Pod", namespace: "checkout", name: "checkout-api-7d7-x2mzp" }, s);
    h.listed.mockImplementation((_ctx: string, kind: string) =>
      Promise.resolve({
        items:
          kind === "Pod"
            ? [{ name: "checkout-api-7d7-q7v4t", namespace: "checkout", age: "2m" }]
            : [{ name: "checkout-api", namespace: "checkout", age: "1d" }],
      }),
    );
    draw("/logs");
    await screen.findByRole("button", { name: /Deployment\/checkout-api/ });
    expect(screen.queryByRole("button", { name: /x2mzp/ })).toBeNull();
    await waitFor(() =>
      expect(recentLogSubjects("prod").map((e) => e.name)).toEqual(["checkout-api"]),
    );
  });

  it("still offers what it could not check, rather than calling it gone", async () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject({ cluster: "prod", kind: "Deployment", namespace: "checkout", name: "checkout-api" }, s);
    h.listed.mockResolvedValue({ error: "handler error: client error (Connect)" });
    draw("/logs");
    const row = await screen.findByRole("button", { name: /Deployment\/checkout-api/ });
    expect((row as HTMLButtonElement).disabled).toBe(false);
    expect(row.textContent).not.toMatch(/no longer/i);
  });

  it("offers another cluster's subjects to that cluster alone", async () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject({ cluster: "dev", kind: "Deployment", namespace: "checkout", name: "checkout-api" }, s);
    draw("/logs");
    await screen.findByText(/pick a workload or a pod/i);
    expect(screen.queryByRole("button", { name: /checkout-api/ })).toBeNull();
    expect(h.listed).not.toHaveBeenCalled();
  });

  it("says there is no cluster in focus rather than streaming from nowhere", async () => {
    resetContexts();
    setContexts([]);
    draw();
    expect(await screen.findByText(/no cluster in focus/i)).toBeTruthy();
    expect(h.resolve).not.toHaveBeenCalled();
  });
});

/** The connection readout's whole sentence, counts and all. */
const signal = () =>
  screen
    .getAllByRole("status")
    .map((el) => el.textContent ?? "")
    .find((t) => /following|connecting|reconnecting|stopped|paused/i.test(t)) ?? "";

/** Every Sources row in the rail, as `[pod, revision]`. */
const railPods = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-slot="pod"]')).map(
    (row) =>
      [
        row.querySelector(".status")?.textContent ?? "",
        row.querySelector(".path")?.textContent ?? "",
      ] as const,
  );

/** Every Top terms row in the rail, as `[term, count]`. */
const railTerms = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-slot="term"]')).map(
    (row) =>
      [
        row.querySelector(".status")?.textContent ?? "",
        row.querySelector(".path")?.textContent ?? "",
      ] as const,
  );

const filterField = () =>
  within(screen.getByRole("search", { name: /filter lines/i })).getByRole("searchbox");

const messages = (region?: HTMLElement) => rendered(region).map((r) => r.split("|")[3]);
const logRegion = () => screen.getByRole("log", { name: /logs/i });

describe("what the stream is doing, in a word AND a denominator", () => {
  /**
   * The aggregate flips to `reconnecting` the moment ONE target does, which is
   * the right answer to "am I seeing everything?" and a badly misleading one
   * on its own: on a ten-pod workload a single blip would read as a total
   * outage. So the word never appears without the counts underneath it.
   */
  it("never says the connection word without its denominator", async () => {
    for (const state of [
      { status: "live" as const, live: 4, reconnecting: 0 },
      { status: "reconnecting" as const, live: 3, reconnecting: 1 },
      { status: "connecting" as const, live: 0, reconnecting: 0 },
    ]) {
      const view = draw();
      await body();
      act(() => {
        Object.assign(h.state, state);
        notify();
      });
      expect(signal()).toMatch(/\d+ of 4 streaming/);
      view.unmount();
    }
  });

  it("reads one flapping pod of ten as one of ten, not as a total outage", async () => {
    draw();
    await body();
    act(() => {
      h.state.status = "reconnecting";
      h.state.live = 9;
      h.state.reconnecting = 1;
      h.state.total = 10;
      notify();
    });
    expect(signal()).toContain("Reconnecting");
    expect(signal()).toContain("9 of 10 streaming");
  });

  it("says how much of a big fan-out is already up while it still says connecting", async () => {
    // `status` stays `connecting` until EVERY target has reported once, so on
    // a wide workload lines can be scrolling while the word still says
    // connecting. The counts are what make that honest.
    draw();
    await body();
    act(() => {
      h.state.status = "connecting";
      h.state.live = 46;
      h.state.reconnecting = 0;
      h.state.total = 50;
      notify();
    });
    expect(signal()).toContain("Connecting");
    expect(signal()).toContain("46 of 50 streaming");
  });

  it("takes its words and its colours from core, never from a table of its own", async () => {
    draw();
    await body();
    act(() => {
      h.state.status = "error";
      h.state.live = 0;
      h.state.reconnecting = 0;
      notify();
    });
    // core's `logConnectionStatus` calls this state "Stream stopped", danger.
    expect(signal()).toContain("Stream stopped");
    expect(signal()).toContain("0 of 4 streaming");
  });

  it("says Paused for the held VIEW, and still says what the connection is doing", async () => {
    draw();
    await body();
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(signal()).toContain("Paused");
    // Pausing freezes the pane, not the stream — a reader who paused and then
    // lost half the fan-out must still be able to see that.
    expect(signal()).toContain("4 of 4 streaming");
  });
});

describe("the Stream rail", () => {
  it("draws a Sources row per pod, with core's verdict and the revision", async () => {
    const { container } = draw();
    await body();
    push(...LINES);
    expect(railPods(container)).toEqual([
      ["api-7", "rev 7d764666f9"],
      ["api-8", "rev 7d764666f9"],
      // No pod-template-hash: nothing at all, not a blank `rev`.
      ["api-9", ""],
    ]);
    const tones = Array.from(container.querySelectorAll('[data-slot="pod"] .status')).map((el) =>
      el.getAttribute("data-kind"),
    );
    expect(tones).toEqual(["danger", "warning", "success"]);
  });

  /**
   * THE TRAP. `tallyLogTerms` reads a leading digit as data and ends the term
   * there, so a buffer still carrying its RFC3339 stamp tallies to nothing —
   * silently, with no error, and an empty rail looks exactly like a quiet log.
   */
  it("hands the rail the STRIPPED message, so the tally is not silently empty", async () => {
    const { container } = draw();
    await body();
    push(
      ...LINES,
      line("api-8/api", "14:07:45.100000000", "error pool timeout waited=30.1s in_use=6"),
      line("api-8/api", "14:07:45.900000000", "warn exporter queue is full again"),
    );
    // The terms are the words of the log, not the empty list a stamped buffer
    // would produce.
    const terms = railTerms(container);
    expect(terms.length).toBeGreaterThan(0);
    expect(terms.map(([term]) => term)).toContain("pool timeout");
    // And nothing that could only come from an unstripped line.
    expect(terms.every(([term]) => !/^\d{4}-\d{2}-\d{2}T/.test(term))).toBe(true);
  });

  it("tallies the Top terms of a logfmt stream, whose every line opens with a time of its own", async () => {
    // `time=…` is a key=value no other line repeats, and the tally ends a term at one before it reads a word of
    // the message. The screen has read the message (`msg`) already, and hands it over.
    const { container } = draw();
    await body();
    push(
      ...[1, 2, 3].map((n) =>
        line("api-8/api", `14:07:4${n}.000000000`, `time=2026-08-24T14:07:4${n}Z level=error msg="Write failed: timeout" system=x`),
      ),
    );
    expect(railTerms(container)).toEqual([["Write failed timeout", "3"]]);
    expect(container.querySelector('[data-slot="term"] .status')?.getAttribute("data-kind")).toBe("danger");
  });

  it("counts what the BODY is drawing, so the badge and the terms cannot describe a different set", async () => {
    const { container } = draw();
    await body();
    push(...LINES, line("api-8/api", "14:07:45.100000000", "error pool timeout waited=30.1s in_use=6"));
    expect(screen.getByText("2 errors")).toBeTruthy();

    await userEvent.type(filterField(), "exporter");
    // One warn line left in view, and no error lines at all — so no badge.
    expect(screen.queryByText(/\d+ errors?/)).toBeNull();
    expect(railTerms(container).map(([term]) => term)).not.toContain("pool timeout");
  });

  it("filters a pod's lines out of the body when its box is unticked", async () => {
    const region = await (draw(), body());
    push(...LINES);
    await userEvent.click(screen.getByRole("checkbox", { name: "api-7" }));
    expect(rendered(region).map((r) => r.split("|")[1])).toEqual(["api-8 · api", "api-9 · api"]);
    // And back again — the box is the only way to undo itself.
    await userEvent.click(screen.getByRole("checkbox", { name: "api-7" }));
    expect(rendered(region)).toHaveLength(4);
  });

  it("keeps the Sources section reachable when a tick has emptied the view", async () => {
    const region = await (draw(), body());
    push(line("api-7/api", "14:07:41.208000000", "info only api-7 ever says anything"));
    await userEvent.click(screen.getByRole("checkbox", { name: "api-7" }));
    expect(rendered(region)).toHaveLength(0);
    expect(screen.getByRole("checkbox", { name: "api-7" })).toBeTruthy();
  });
});

describe("exporting what is on screen", () => {
  const flatten = () => String(h.saved.mock.calls[0][1]).split("\n");

  /** jsdom's Blob has no `text()`, so read it the way a browser would. */
  const readBlob = (blob: Blob) =>
    new Promise<string>((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.readAsText(blob);
    });

  it("writes the buffer through the desktop's save dialog, flattened as classic does", async () => {
    draw();
    await body();
    push(...LINES);
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    await waitFor(() => expect(h.saved).toHaveBeenCalledTimes(1));
    expect(h.saved.mock.calls[0][0]).toBe("checkout-api.log");
    expect(flatten()).toEqual([
      "api-7/api | 2026-08-24T14:07:41.208000000Z info starting checkout-api build=4f2a1c",
      "api-7/otel-sidecar | 2026-08-24T14:07:42.100000000Z warn exporter queue is full",
      "api-8/api | 2026-08-24T14:07:43.900000000Z error pool timeout waited=30.0s in_use=5",
      "api-9/api | 2026-08-24T14:07:44.010000000Z GET /healthz 200 1ms",
    ]);
  });

  it("exports WHAT IS ON SCREEN — every filter applied, not the whole buffer", async () => {
    draw();
    await body();
    push(...LINES);
    await userEvent.type(filterField(), "pool");
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    await waitFor(() => expect(h.saved).toHaveBeenCalledTimes(1));
    expect(flatten()).toEqual([
      "api-8/api | 2026-08-24T14:07:43.900000000Z error pool timeout waited=30.0s in_use=5",
    ]);
  });

  /**
   * The one above proves the export is narrowed by the FILTERS. It cannot
   * prove it is not also narrowed by the WINDOW, because jsdom measures no row
   * height, so `computeLogWindow` bails out to "render everything" and the
   * drawn slice and the filtered buffer are the same array — an export of
   * either passes. Measure the rows and the two come apart, which is the only
   * condition under which this property is testable at all.
   */
  it("exports every line the filters left, not the slice the viewport had room for", async () => {
    const restore = measurableRows();
    try {
      const region = await (draw(), body());
      Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 100 });
      push(
        ...Array.from({ length: 300 }, (_, i) =>
          line("api-7/api", "14:07:41.208000000", `${i % 2 === 0 ? "alpha" : "beta"} line ${i}`),
        ),
      );
      await userEvent.type(filterField(), "alpha");

      // 150 lines match, and the window is drawing a small fraction of them —
      // which is what makes the next assertion mean something.
      const drawn = region.querySelectorAll(".logline").length;
      expect(drawn).toBeGreaterThan(0);
      expect(drawn).toBeLessThan(150);

      await userEvent.click(screen.getByRole("button", { name: /export/i }));
      await waitFor(() => expect(h.saved).toHaveBeenCalledTimes(1));
      const written = flatten();
      // Every matching line, not the `drawn` rows. How far someone happened to
      // have scrolled is not a thing they chose to look at.
      expect(written).toHaveLength(150);
      expect(written.every((l) => l.includes("alpha line"))).toBe(true);
      expect(written[written.length - 1]).toContain("alpha line 298");
    } finally {
      restore();
    }
  });

  it("exports the held view while paused, which is what the reader is looking at", async () => {
    draw();
    await body();
    push(...LINES);
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    // The hook freezes `lines` while paused and keeps counting what arrives.
    act(() => {
      h.state.pending = 900;
      notify();
    });
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    await waitFor(() => expect(h.saved).toHaveBeenCalledTimes(1));
    expect(flatten()).toHaveLength(4);
  });

  it("downloads in the browser, where a save dialog is not reachable", async () => {
    h.tauri = false;
    const urls: Blob[] = [];
    const clicked: string[] = [];
    const globalUrl = URL as unknown as {
      createObjectURL: (b: Blob) => string;
      revokeObjectURL: (u: string) => void;
    };
    globalUrl.createObjectURL = (b: Blob) => {
      urls.push(b);
      return "blob:logs";
    };
    globalUrl.revokeObjectURL = () => {};
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(function (this: HTMLAnchorElement) {
        clicked.push(this.download);
      });
    try {
      draw();
      await body();
      push(...LINES);
      await userEvent.click(screen.getByRole("button", { name: /export/i }));
      await waitFor(() => expect(clicked).toEqual(["checkout-api.log"]));
      expect(h.saved).not.toHaveBeenCalled();
      expect(await readBlob(urls[0])).toContain("api-8/api | ");
    } finally {
      click.mockRestore();
    }
  });

  it("offers nothing to export when there is nothing on screen", async () => {
    draw();
    await body();
    expect(screen.getByRole("button", { name: /export/i })).toHaveProperty("disabled", true);
    push(...LINES);
    expect(screen.getByRole("button", { name: /export/i })).toHaveProperty("disabled", false);
  });

  it("heads a failed save with the screen's own sentence, not the backend's", async () => {
    h.saved.mockRejectedValue("save_text_file failed: Permission denied (os error 13)");
    draw();
    await body();
    push(...LINES);
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    const notice = await screen.findByText(/could not save this stream/i);
    // The headline says what failed. `describeError` has nothing to classify
    // in a local filesystem refusal, so the original is what sits underneath
    // — once, in the body, never as the sentence the reader is handed.
    expect(notice.textContent).toBe("Could not save this stream");
  });

  it("classifies what it can, rather than printing String(e) the way classic did", async () => {
    h.saved.mockRejectedValue("save_text_file failed: deadline exceeded");
    draw();
    await body();
    push(...LINES);
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    const notice = await screen.findByText(/could not save this stream/i);
    const alert = notice.closest("[role=status]") as HTMLElement;
    expect(within(alert).getByText(/didn't respond in time/i)).toBeTruthy();
    // The original is still reachable, but only folded away behind `RawError`.
    expect(alert.querySelector("[data-slot=alert-body]")?.firstChild?.textContent).not.toContain(
      "deadline exceeded",
    );
    expect(alert.querySelector("details")?.textContent).toContain("deadline exceeded");
  });

  it("puts the failure away again on the next export", async () => {
    h.saved.mockRejectedValueOnce("save_text_file failed: Permission denied (os error 13)");
    draw();
    await body();
    push(...LINES);
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    await screen.findByText(/could not save this stream/i);

    h.saved.mockResolvedValue("/home/u/checkout-api.log");
    await userEvent.click(screen.getByRole("button", { name: /export/i }));
    await waitFor(() => expect(screen.queryByText(/could not save this stream/i)).toBeNull());
  });
});

describe("what a restart costs the reader", () => {
  it("says nothing before the first restart", async () => {
    draw();
    await body();
    expect(screen.queryByText(/scrollback/i)).toBeNull();
  });

  it("says the scrollback went when a since change reopened the stream", async () => {
    draw();
    await body();
    push(...LINES);
    act(() => {
      // What the hook does on a since/container/tail change: a fresh buffer
      // and a bumped restart count.
      h.state.lines = [];
      h.state.restarts = 1;
      notify();
    });
    expect(screen.getByText(/scrollback/i)).toBeTruthy();
  });

  it("lets the reader put the notice away, and says it again on the NEXT restart", async () => {
    draw();
    await body();
    act(() => {
      h.state.restarts = 1;
      notify();
    });
    await userEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    // `waitFor`, not a bare query: the click is async, and asserting the
    // notice is gone in the same tick passes only while the machine is quick
    // enough to have flushed the state update. Under full-suite load it was
    // not, and this test failed roughly one run in seven.
    await waitFor(() => expect(screen.queryByText(/scrollback/i)).toBeNull());

    act(() => {
      h.state.restarts = 2;
      notify();
    });
    expect(screen.getByText(/scrollback/i)).toBeTruthy();
  });
});

/**
 * The logs of the instance that just died.
 *
 * Everything here turns on one structural fact: a terminated container cannot
 * be streamed. `podLogs(..., { previous: true })` is a ONE-SHOT fetch that
 * returns a whole blob — the streaming path passes `previous: false` as a
 * literal — so following is not disabled by convention here, it is disabled
 * because there is nothing to follow.
 */
describe("the previous instance", () => {
  /** `resolveLogSubject` found a corpse: api-7's `api` container has died. */
  function withCorpse(previous: PreviousInstance[] = TERMINATED) {
    h.resolve.mockResolvedValue({ status: "resolved", targets: TARGETS, pods: PODS, previous });
  }

  const toggle = () => screen.getByRole("button", { name: /previous instance/i });
  /** The play/pause control, under whichever name it currently answers to. */
  const followControl = () => screen.getByRole("button", { name: /^(pause|follow)/i });

  /** Turn the toggle on (or off) and let the one-shot fetch settle. */
  async function press() {
    await userEvent.click(toggle());
  }

  it("swaps the whole buffer for the terminated container's", async () => {
    withCorpse();
    const region = await (draw(), body());
    push(...LINES);
    await press();

    // The stamp is off, the source is split, and the level column says the
    // word the LINE used — every one of those through the same `toRow` the
    // live buffer goes through. A previous line that toned differently from a
    // live one would be a second severity rule in the same pane.
    await waitFor(() =>
      expect(rendered(region)).toEqual([
        `${localTs("2026-08-24T14:07:11.004Z")}|api-7 · api|info|info starting checkout-api build=4f2a1c pool_size=5`,
        `${localTs("2026-08-24T14:07:12.880Z")}|api-7 · api|warn|warn pool saturated within 1.8s of accepting traffic`,
        `${localTs("2026-08-24T14:07:41.902Z")}|api-7 · api|error|error pool timeout waited=30.0s pool_size=5 in_use=5`,
        `${localTs("2026-08-24T14:07:42.410Z")}|api-7 · api|fatal|fatal liveness deadline exceeded, terminating`,
      ]),
    );
    // And the live lines are gone from the pane, not merged into it: the two
    // buffers are different instances of the same container, and interleaving
    // them would invent a history that never happened.
    expect(rendered(region).join("\n")).not.toContain("exporter queue is full");
  });

  it("takes the time window off with the buffer it was picked from, and draws the other buffer whole", async () => {
    // A window is a stretch of one buffer's clock. Kept across the swap it
    // would crop a different instance's lines to a minute they never ran in.
    withCorpse();
    const { container } = draw();
    const region = await body();
    push(...LINES);
    const bar = container.querySelectorAll('[data-slot="bucket"]')[0];
    fireEvent.pointerDown(bar);
    fireEvent.pointerUp(bar);
    expect(screen.getByRole("button", { name: "Remove the time window" })).toBeTruthy();
    await press();
    await waitFor(() => expect(rendered(region)).toHaveLength(4));
    expect(screen.queryByRole("button", { name: "Remove the time window" })).toBeNull();
  });

  it("tones the previous buffer through the kit, exactly as a live line is", async () => {
    withCorpse();
    const region = await (draw(), body());
    await press();
    await waitFor(() => expect(rendered(region)).toHaveLength(4));
    const levels = Array.from(region.querySelectorAll<HTMLElement>("[data-slot=level]"));
    expect(levels.map((el) => el.style.color)).toEqual([
      toneColor("info"),
      toneColor("warn"),
      toneColor("sev"),
      // `fatal` is the kit's own word for the worst tone. The screen passes
      // the level and no colour, here as everywhere.
      toneColor("sev"),
    ]);
  });

  it("fetches the terminated container's buffer once, and asks no window of it", async () => {
    withCorpse();
    draw();
    await body();
    // A window the reader chose for the LIVE tail. The instance that died
    // twenty minutes ago wrote nothing in the last five, so passing it on
    // would hand back an empty file and call it the corpse's logs.
    await userEvent.selectOptions(screen.getByRole("combobox", { name: /since/i }), "5m");
    await press();

    await waitFor(() => expect(h.fetched).toHaveBeenCalledTimes(1));
    const [context, namespace, pod, , options] = h.fetched.mock.calls[0];
    expect([context, namespace, pod]).toEqual(["prod-eu", "checkout", "api-7"]);
    expect(options).toEqual({
      container: "api",
      previous: true,
      // The time column is drawn from the stamp, in the previous buffer as in
      // the live one.
      timestamps: true,
      tailLines: 1000,
    });
  });

  it("asks only for the containers that have actually died", async () => {
    withCorpse();
    draw();
    await body();
    await press();
    await waitFor(() => expect(h.fetched).toHaveBeenCalledTimes(1));
    // api-7's otel-sidecar, api-8 and api-9 are all in scope and none of them
    // has a previous instance — asking for one gets an error, not a buffer.
    expect(h.fetched.mock.calls.map((c: unknown[]) => c[2])).toEqual(["api-7"]);
  });

  it("names the pod, when it terminated and its exit code", async () => {
    withCorpse();
    draw();
    await body();
    await press();

    const banner = await screen.findByText(/reading the previous instance/i);
    const strip = banner.closest("[role=status]") as HTMLElement;
    expect(strip.textContent).toContain("api-7");
    // Core's own formatter, not a second clock in this screen — and the whole
    // date, because a container that died yesterday is not "14:07:42".
    expect(strip.textContent).toContain(absoluteTimestamp("2026-08-24T14:07:42Z"));
    expect(strip.textContent).toContain("exit 137");
    // The exit code alone does not separate an OOM kill from a SIGKILL.
    expect(strip.textContent).toContain("OOMKilled");
  });

  it("names only what the cluster actually sent about the termination", async () => {
    // Found by the mutation pass: with a fixture that always carried an exit
    // code AND a reason, the branches for a termination missing one of them
    // were never drawn, and a mutant that emptied the exit-code-only branch
    // lived. Each fact is dropped rather than faked when it is absent.
    const cases: [PreviousInstance, RegExp, RegExp][] = [
      // An exit code with no reason: the code alone, and no empty bracket
      // where the reason would have gone.
      [{ pod: "api-7", container: "api", exitCode: 137 }, /exit 137/, /[()]/],
      // A reason with no code — the word, and no bare `exit`.
      [{ pod: "api-7", container: "api", reason: "OOMKilled" }, /OOMKilled/, /exit/],
      // Neither, and no finish time: the container is still named, with no
      // dangling separator after it.
      [{ pod: "api-7", container: "api" }, /api-7 · api/, /—/],
    ];
    for (const [instance, says, silent] of cases) {
      withCorpse([instance]);
      const view = draw();
      await body();
      await press();
      const headline = await screen.findByText(/reading the previous instance/i);
      expect(headline.textContent).toMatch(says);
      expect(headline.textContent).not.toMatch(silent);
      view.unmount();
    }
  });

  it("clears follow, and the disabled control SAYS WHY rather than going inert", async () => {
    withCorpse();
    draw();
    await body();
    expect(signal()).toContain("Following");

    await press();

    const control = followControl();
    expect(control).toHaveProperty("disabled", true);
    // A control that stops working without explanation reads as a bug. The
    // reason is in its accessible name and in the banner's own words.
    expect(control.getAttribute("aria-label")).toMatch(/terminated instance cannot be streamed/i);
    expect(screen.getByText(/a terminated instance cannot be streamed/i)).toBeTruthy();
    // And nothing claims to be following a snapshot. Asserted on the live
    // readout's own shape rather than through `signal()`, whose regex the
    // banner's own sentence ("srelens is not following it") matches.
    const readouts = screen.queryAllByRole("status").map((el) => el.textContent ?? "");
    expect(readouts.some((t) => /of \d+ streaming/.test(t))).toBe(false);
  });

  it("restores the live stream when it is turned off", async () => {
    withCorpse();
    const region = await (draw(), body());
    push(...LINES);
    await press();
    await waitFor(() => expect(rendered(region).join("\n")).toContain("pool saturated within 1.8s"));

    // The live stream never stopped: a line that arrived while the corpse was
    // on screen is waiting when the reader comes back.
    push(line("api-9/api", "14:07:50.000000000", "info back up"));
    expect(rendered(region).join("\n")).not.toContain("back up");

    await press();
    expect(rendered(region).map((r) => r.split("|")[3])).toEqual([
      "info starting checkout-api build=4f2a1c",
      "warn exporter queue is full",
      "error pool timeout waited=30.0s in_use=5",
      "GET /healthz 200 1ms",
      "info back up",
    ]);
    expect(followControl()).toHaveProperty("disabled", false);
    expect(signal()).toContain("Following");
    expect(screen.queryByText(/reading the previous instance/i)).toBeNull();
  });

  it("says a container on its first run has no previous instance, rather than drawing nothing", async () => {
    // The DEFAULT resolution: no corpse. A pod that has never restarted is the
    // common case, and an empty body would read as "the crashed instance said
    // nothing", which is a different and much more alarming fact.
    const region = await (draw(), body());
    push(...LINES);
    await press();

    expect(within(region).getByText(/no previous instance/i)).toBeTruthy();
    expect(rendered(region)).toHaveLength(0);
    // Nothing was asked of the cluster: there is no terminated container to
    // ask about, and `podLogs` would only refuse.
    expect(h.fetched).not.toHaveBeenCalled();
  });

  it("says the previous instance logged nothing, which is not the same as having none", async () => {
    withCorpse();
    h.fetched.mockResolvedValue({ logs: "" });
    const region = await (draw(), body());
    await press();
    expect(await within(region).findByText(/logged nothing/i)).toBeTruthy();
    // The banner still names it — the instance existed, and when it died is
    // the fact the reader came for even when its buffer is empty.
    expect(screen.getByText(/reading the previous instance/i)).toBeTruthy();
  });

  it("reads a refused fetch through describeError, never as the backend's string", async () => {
    withCorpse();
    h.fetched.mockResolvedValue({ error: "k8s.podLogs failed: request timeout" });
    draw();
    await body();
    await press();

    const notice = await screen.findByText(/could not read the previous instance/i);
    const card = notice.closest("[role=alert], [role=status]") as HTMLElement;
    expect(within(card).getByText(/didn't respond in time/i)).toBeTruthy();
    expect(card.querySelector("details")?.textContent).toContain("k8s.podLogs failed");
  });

  it("says which corpse it could not read rather than silently dropping it", async () => {
    // Two dead containers, one of which refuses. `resolveLogSubject` is
    // all-or-nothing about targets for exactly this reason: a missing
    // container's lines look exactly like a quiet container's.
    withCorpse([
      TERMINATED[0],
      { pod: "api-8", container: "api", exitCode: 1, reason: "Error", finishedAt: "2026-08-24T14:06:00Z" },
    ]);
    h.fetched.mockImplementation((...a: unknown[]) =>
      a[2] === "api-8"
        ? Promise.resolve({ error: "k8s.podLogs failed: request timeout" })
        : Promise.resolve({ logs: PREVIOUS_BLOB }),
    );
    const region = await (draw(), body());
    await press();

    // api-7's four lines are drawn — throwing them away because api-8 refused
    // would lose the evidence the reader came for.
    await waitFor(() => expect(rendered(region)).toHaveLength(4));
    const notice = await screen.findByText(/could not read the previous instance/i);
    expect(notice.textContent).toContain("api-8");
  });

  it("stops the since window pretending to narrow a snapshot", async () => {
    // The snapshot arrived whole, in one fetch: no time window applies to it,
    // and a control that looks live and does nothing is what this migration
    // has been removing.
    withCorpse();
    draw();
    await body();
    const since = () => screen.getByLabelText("since") as HTMLSelectElement;
    expect(since().disabled).toBe(false);
    await press();
    await waitFor(() => expect(since().disabled).toBe(true));
    await press();
    await waitFor(() => expect(since().disabled).toBe(false));
  });

  it("does not blame the live ring for a snapshot's length", async () => {
    // "Showing the newest N lines · 1 200 earlier lines dropped" is a fact
    // about the bounded live buffer. The previous instance's arrived whole, in
    // one fetch, and saying lines were dropped from it would send a reader
    // looking for logs that were never missing.
    withCorpse();
    const region = await (draw(), body());
    act(() => {
      h.state.lines = LINES;
      h.state.dropped = 1200;
      notify();
    });
    expect(within(region).getByText(/1 200 earlier lines/i)).toBeTruthy();

    await press();
    await waitFor(() => expect(rendered(region)).toHaveLength(4));
    expect(within(region).queryByText(/earlier lines dropped/i)).toBeNull();
  });

  it("hands the rail the STRIPPED previous lines, so the tally is not silently empty", async () => {
    withCorpse();
    h.fetched.mockResolvedValue({
      logs: [
        "2026-08-24T14:07:41.902000000Z error pool timeout waited=30.0s pool_size=5 in_use=5",
        "2026-08-24T14:07:42.100000000Z error pool timeout waited=30.1s pool_size=5 in_use=5",
        "",
      ].join("\n"),
    });
    const { container } = draw();
    await body();
    await press();

    await waitFor(() => expect(railTerms(container).length).toBeGreaterThan(0));
    expect(railTerms(container).map(([term]) => term)).toContain("pool timeout");
    // Two error lines in view, counted by the same badge the live buffer uses.
    expect(screen.getByText("2 errors")).toBeTruthy();
  });

  it("exports the previous buffer under its own name, filters applied", async () => {
    // The window vs the buffer: jsdom measures no row height, so without
    // `measurableRows` the drawn slice and the filtered buffer are the same
    // array and an export of either passes.
    const restore = measurableRows();
    try {
      withCorpse();
      h.fetched.mockResolvedValue({
        logs:
          Array.from(
            { length: 300 },
            (_, i) =>
              `2026-08-24T14:07:41.208000000Z ${i % 2 === 0 ? "alpha" : "beta"} line ${i}`,
          ).join("\n") + "\n",
      });
      const region = await (draw(), body());
      Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 100 });
      await press();
      await waitFor(() => expect(region.querySelectorAll(".logline").length).toBeGreaterThan(0));
      await userEvent.type(filterField(), "alpha");

      const drawn = region.querySelectorAll(".logline").length;
      expect(drawn).toBeLessThan(150);

      await userEvent.click(screen.getByRole("button", { name: /export/i }));
      await waitFor(() => expect(h.saved).toHaveBeenCalledTimes(1));
      // Its own filename: a file of a dead instance's logs that is named like
      // the live one is a file nobody can tell apart afterwards.
      expect(h.saved.mock.calls[0][0]).toBe("checkout-api-previous.log");
      const written = String(h.saved.mock.calls[0][1]).split("\n");
      expect(written).toHaveLength(150);
      expect(written[0]).toBe(
        "api-7/api | 2026-08-24T14:07:41.208000000Z alpha line 0",
      );
      expect(written[written.length - 1]).toContain("alpha line 298");
    } finally {
      restore();
    }
  });
});

/** An observability app whose Loki provider is for workloads, and one for pods only. */
function observability() {
  return {
    manifest: {
      id: "org.example.observability", name: "Observability", version: "1.0.0", srelensApiVersion: "^0.7",
      kind: "declarative", permissions: [], capabilities: [],
      contributions: {
        pages: [], detailTabs: [], detailLinks: [],
        logProviders: [
          { id: "loki", title: "Loki", capability: "loki", language: "logql", forKinds: ["apps/Deployment"], query: "{}" },
          { id: "podLoki", title: "Pod Loki", capability: "loki", language: "logql", forKinds: ["/Pod"], query: "{}" },
        ],
      },
    },
    enabled: true, revision: 7, grants: [], settings: {}, source: "local", installedAt: 0, history: [],
  };
}

describe("log sources (#569)", () => {
  it("offers no source picker while no installed app provides logs", async () => {
    draw();
    await screen.findByRole("log");
    expect(screen.queryByRole("combobox", { name: "Logs from" })).toBeNull();
  });

  it("offers Kubernetes first, then each installed log provider for the subject's kind", async () => {
    h.plugins = [observability()];
    draw();
    const picker = await screen.findByRole("combobox", { name: "Logs from" });
    expect(within(picker).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Kubernetes",
      "Loki · Observability",
    ]);
    expect((picker as HTMLSelectElement).value).toBe("kubernetes");
  });

  it("follows the chosen provider through the view's own stream, without the Kubernetes-only controls", async () => {
    h.plugins = [observability()];
    draw();
    const picker = await screen.findByRole("combobox", { name: "Logs from" });
    expect(screen.getByRole("button", { name: /Previous instance/ })).toBeTruthy();
    fireEvent.change(picker, { target: { value: "org.example.observability/loki" } });
    await waitFor(() => expect(h.seen.at(-1)?.options.source?.key).toContain("org.example.observability/loki"));
    const asked = h.seen.at(-1)!;
    expect(asked.targets).toEqual([{ pod: "checkout-api", label: "loki" }]);
    // "all" is the longest history a provider is asked for, rather than none.
    expect(asked.options.sinceSeconds).toBe(7 * 24 * 3600);
    expect(asked.options.tailLines).toBe(1000);
    expect(h.views.map((v) => [v.app, v.label])).toEqual([["org.example.observability", "logs:loki"]]);
    // A previous instance is the cluster's, not Loki's.
    expect(screen.queryByRole("button", { name: /Previous instance/ })).toBeNull();
    // What the hook would open: the provider's stream for this Deployment.
    await asked.options.source!.open(...([[], () => {}, () => {}, { tailLines: 1000 }] as never[]));
    expect(h.views[0].opened[0].request).toMatchObject({
      id: "org.example.observability", revision: 7, context: "prod-eu", namespace: "checkout",
      source: { kind: "logProvider", provider: "loki", resourceKind: "apps/Deployment", name: "checkout-api" },
    });
    // And back: Kubernetes follows the cluster's own targets again, and the view is closed.
    fireEvent.change(picker, { target: { value: "kubernetes" } });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeUndefined());
    expect(h.seen.at(-1)!.targets).toEqual(TARGETS);
    expect(h.views[0].close).toHaveBeenCalled();
  });

  it("keeps the time window across a change of source, which is the same stretch of time", async () => {
    h.plugins = [observability()];
    const { container } = draw();
    await body();
    push(...LINES);
    const bar = container.querySelectorAll('[data-slot="bucket"]')[0];
    fireEvent.pointerDown(bar);
    fireEvent.pointerUp(bar);
    fireEvent.change(await screen.findByRole("combobox", { name: "Logs from" }), {
      target: { value: "org.example.observability/loki" },
    });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeDefined());
    expect(screen.getByRole("button", { name: "Remove the time window" })).toBeTruthy();
  });

  it("does not call a change of source a cleared scrollback: the new source sends its own history", async () => {
    h.plugins = [observability()];
    draw();
    const picker = await screen.findByRole("combobox", { name: "Logs from" });
    const restart = (count: number) =>
      act(() => {
        h.state.restarts = count;
        notify();
      });
    fireEvent.change(picker, { target: { value: "org.example.observability/loki" } });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeDefined());
    restart(1);
    expect(screen.queryByText("Scrollback cleared")).toBeNull();
    // A provider sends its history again for a new window too.
    fireEvent.change(screen.getByRole("combobox", { name: "since" }), { target: { value: "5m" } });
    restart(2);
    expect(screen.queryByText("Scrollback cleared")).toBeNull();
    fireEvent.change(picker, { target: { value: "kubernetes" } });
    restart(3);
    expect(screen.queryByText("Scrollback cleared")).toBeNull();
    // The cluster does not: a change of window on Kubernetes still clears the scrollback.
    fireEvent.change(screen.getByRole("combobox", { name: "since" }), { target: { value: "15m" } });
    restart(4);
    expect(await screen.findByText("Scrollback cleared")).toBeTruthy();
  });

  it("says how a provider's stream ended, as a failure, and follows it again", async () => {
    h.plugins = [observability()];
    draw();
    fireEvent.change(await screen.findByRole("combobox", { name: "Logs from" }), {
      target: { value: "org.example.observability/loki" },
    });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeDefined());
    const first = h.seen.at(-1)!.options.source!;
    await first.open(...([[], () => {}, () => {}, {}] as never[]));
    act(() =>
      h.views[0].opened[0].handlers.onEnd?.({ type: "error", code: "source", message: "The server answered HTTP 403 Forbidden" }),
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "The stream failed: The server answered HTTP 403 Forbidden",
    );
    // The readout says so too: a failed stream is not one still followed.
    expect(document.body.textContent).toContain("Stream stopped — 0 of");
    expect(document.body.textContent).not.toMatch(/Following — \d+ of/);
    fireEvent.click(screen.getByRole("button", { name: "Follow again" }));
    await waitFor(() => expect(h.seen.at(-1)?.options.source?.key).not.toBe(first.key));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /** Pick the workloads' Loki and wait for its stream to be asked for. */
  async function followLoki() {
    fireEvent.change(await screen.findByRole("combobox", { name: "Logs from" }), {
      target: { value: "org.example.observability/loki" },
    });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeDefined());
  }
  /** The inventory as the next read answers it. */
  function inventoryReads(plugins: unknown[], error?: string) {
    act(() => {
      h.plugins = plugins;
      h.inventoryError = error;
      notify();
    });
  }

  it("keeps following a provider through an inventory read that fails, and says the list may be out of date", async () => {
    h.plugins = [observability()];
    draw();
    await followLoki();
    const followed = h.seen.at(-1)!.options.source!.key;
    inventoryReads([observability()], "The inventory could not be read");
    expect(h.seen.at(-1)?.options.source?.key).toBe(followed);
    expect(document.body.textContent).toContain("Could not check which apps provide logs here");
    expect(document.body.textContent).not.toContain("is no longer offered");
    expect(h.views[0].close).not.toHaveBeenCalled();
  });

  it("lets go of a provider no longer offered, so it does not come back unasked", async () => {
    h.plugins = [observability()];
    draw();
    await followLoki();
    inventoryReads([]);
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeUndefined());
    expect(h.seen.at(-1)!.targets).toEqual(TARGETS);
    expect(document.body.textContent).toContain("Loki · Observability is no longer offered for this Deployment");
    // Kubernetes again: its own controls are back, and its own tail is not a cleared scrollback.
    act(() => {
      h.state.restarts = 1;
      notify();
    });
    expect(screen.queryByText("Scrollback cleared")).toBeNull();
    expect(screen.getByRole("button", { name: /Previous instance/ })).toBeTruthy();
    // The app is back: the reader is told nothing new and nothing reopens behind them.
    inventoryReads([observability()]);
    const picker = await screen.findByRole("combobox", { name: "Logs from" });
    expect((picker as HTMLSelectElement).value).toBe("kubernetes");
    expect(h.seen.at(-1)?.options.source).toBeUndefined();
    expect(h.views).toHaveLength(1);
  });

  it("says what a provider has sent, and that an ended stream is not followed", async () => {
    h.plugins = [observability()];
    draw();
    await followLoki();
    expect(document.body.textContent).toContain(
      "srelens is following checkout-api through Loki · Observability; it has sent no line in the last 7 days.",
    );
    expect(screen.getByRole("button", { name: /Pause/ })).toBeTruthy();
    await h.seen.at(-1)!.options.source!.open(...([[], () => {}, () => {}, {}] as never[]));
    act(() => h.views[0].opened[0].handlers.onEnd?.({ type: "close", reason: "completed" }));
    await screen.findByRole("button", { name: "Follow again" });
    expect(document.body.textContent).toContain("Loki · Observability sent no line in the last 7 days before its stream ended.");
    const follow = screen.getByRole("button", { name: /^Follow$/ }) as HTMLButtonElement;
    expect(follow.disabled).toBe(true);
  });
});

/**
 * What Home's "Live now" reads to know a logs tab is streaming: the route
 * alone says a tab is open, not that a stream started or is still running.
 */
describe("the running-stream registry", () => {
  function drawInTab(tabId: string) {
    return render(
      <ConsoleProvider>
        <TabScope.Provider value={tabId}><Logs route={ROUTE} /></TabScope.Provider>
      </ConsoleProvider>,
    );
  }

  it("marks the tab while its stream runs, and unmarks it when the tab goes", async () => {
    const view = drawInTab("tab-1");
    await body();
    expect(liveLogStreams().has("tab-1")).toBe(true);
    view.unmount();
    expect(liveLogStreams().has("tab-1")).toBe(false);
  });

  it.each(["error", "completed"])("unmarks a stream that ended as %s", async (ended) => {
    drawInTab(`tab-${ended}`);
    await body();
    expect(liveLogStreams().has(`tab-${ended}`)).toBe(true);
    act(() => { h.state.status = ended as typeof h.state.status; notify(); });
    expect(liveLogStreams().has(`tab-${ended}`)).toBe(false);
  });

  it("never marks a tab whose subject left nothing to follow", async () => {
    h.resolve.mockResolvedValue({ status: "empty", detail: "checkout-api has no containers." });
    drawInTab("tab-3");
    await screen.findByText("Nothing to follow");
    expect(liveLogStreams().has("tab-3")).toBe(false);
  });
});

describe("structured lines and the query (#898)", () => {
  it("shows a JSON line as its msg with the other fields after it, not as the raw object", async () => {
    const region = await (draw(), body());
    push(...JSON_LINES);
    expect(rendered(region)[0]).toBe(
      `${localTs("2026-08-24T14:07:41.208Z")}|api-7 · api|info|Wrote batch  system=camlog internal_use=true`,
    );
  });

  it("keeps the two-space gap before the fields from collapsing, without changing the text or its marks", async () => {
    // The row is `white-space: nowrap`, which collapses two spaces to one. The gap is its own pre-spaced span rather
    // than a pre-wrap faint span, which would let the row break and lose the fixed height the window depends on.
    const region = await (draw(), body());
    push(...JSON_LINES);
    const cell = () => region.querySelector("[data-slot=message]") as HTMLElement;
    const gap = cell().querySelector(".whitespace-pre");
    expect(gap?.textContent).toBe("  ");
    expect(gap?.parentElement?.classList.contains("text-faint")).toBe(true);
    expect(cell().querySelector(".text-faint")?.classList.contains("whitespace-pre")).toBe(false);
    expect(cell().textContent).toBe("Wrote batch  system=camlog internal_use=true");
    await userEvent.type(filterField(), "system:camlog");
    expect(Array.from(cell().querySelectorAll("mark")).map((m) => m.textContent)).toEqual(["system=camlog"]);
    expect(cell().textContent).toBe("Wrote batch  system=camlog internal_use=true");
  });

  it("draws no gap for a row with nothing after its message, or no message before its fields", async () => {
    const region = await (draw(), body());
    push(
      line("api-7/api", "14:07:41.208000000", "info a plain line"),
      line("api-7/api", "14:07:42.208000000", '{"level":"warn","system":"camlog"}'),
    );
    expect(region.querySelectorAll(".whitespace-pre")).toHaveLength(0);
  });

  it("filters on a field, and ORs two values of one field", async () => {
    const region = await (draw(), body());
    push(...JSON_LINES);
    await userEvent.type(filterField(), "system:camlog");
    expect(messages(region)).toEqual([
      "Wrote batch  system=camlog internal_use=true",
      "Success: Write failed  system=camlog",
    ]);
    await userEvent.clear(filterField());
    await userEvent.type(filterField(), "level:warn level:error");
    expect(messages(region)).toEqual(["Skipping checkpoint  system=schwarz", "Success: Write failed  system=camlog"]);
  });

  it("excludes with a leading '-'", async () => {
    const region = await (draw(), body());
    push(...LINES);
    await userEvent.type(filterField(), "-pool");
    expect(messages(region)).toHaveLength(3);
    expect(messages(region)).not.toContain("error pool timeout waited=30.0s in_use=5");
  });

  it("finds a phrase across the punctuation the term tally strips", async () => {
    const region = await (draw(), body());
    push(...JSON_LINES);
    await userEvent.type(filterField(), '"success write failed"');
    expect(messages(region)).toEqual(["Success: Write failed  system=camlog"]);
  });

  it("says why a query can't be applied, and leaves the body unfiltered", async () => {
    const region = await (draw(), body());
    push(...LINES);
    await userEvent.type(filterField(), "/(/");
    const error = screen.getByText(/invalid regex/i);
    expect(error.closest('[role="alert"]')).not.toBeNull();
    expect(rendered(region)).toHaveLength(4);
  });

  it("marks what the query matched inside the message", async () => {
    await (draw(), body());
    push(...LINES);
    await userEvent.type(filterField(), "pool");
    const marks = Array.from(logRegion().querySelectorAll("[data-slot=message] mark")).map((m) => m.textContent);
    expect(marks).toEqual(["pool"]);
  });

  it("shows each clause as a pill whose × takes it out of the box", async () => {
    await (draw(), body());
    push(...LINES);
    await userEvent.type(filterField(), "level:error pool");
    await userEvent.click(screen.getByRole("button", { name: "Remove filter level:error" }));
    expect(filterField()).toHaveProperty("value", "pool");
  });

  it("keeps the keyboard in the box when a pill goes from the keyboard", async () => {
    await (draw(), body());
    push(...LINES);
    await userEvent.type(filterField(), "level:error pool");
    screen.getByRole("button", { name: "Remove filter level:error" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(document.activeElement).toBe(filterField());
    expect(filterField()).toHaveProperty("value", "pool");
  });

  it("keeps the keyboard in the box after Clear all", async () => {
    await (draw(), body());
    push(...LINES);
    await userEvent.type(filterField(), "level:error pool");
    screen.getByRole("button", { name: "Clear all" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(document.activeElement).toBe(filterField());
    expect(filterField()).toHaveProperty("value", "");
  });

  it("filters on a level clicked in the rail, writing it into the search box", async () => {
    const region = await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(screen.getByRole("button", { name: "level = warn, 1 line" }));
    expect(filterField()).toHaveProperty("value", "level:warn");
    expect(messages(region)).toEqual(["Skipping checkpoint  system=schwarz"]);
  });

  it("excludes a field value from the rail, and the same button takes it back", async () => {
    const region = await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(screen.getByRole("button", { name: "Exclude system = camlog" }));
    expect(filterField()).toHaveProperty("value", "-system:camlog");
    expect(messages(region)).toEqual(["Skipping checkpoint  system=schwarz"]);
    await userEvent.click(screen.getByRole("button", { name: "Exclude system = camlog" }));
    expect(filterField()).toHaveProperty("value", "");
  });

  it("recounts each facet once for a streamed line, not again for the field list", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    h.facetPasses.length = 0;
    // No new field and no new value: the same three facets, one line heavier.
    push(line("api-9/api", "14:07:44.010000000", '{"level":"info","msg":"Wrote again","system":"camlog"}'));
    // Within a quarter second of the last batch, so the recount lands when that is up.
    expect(await screen.findByRole("button", { name: "level = info, 2 lines" })).toBeTruthy();
    // The Level facet and the one discovered field, once each: the deferred
    // pass. The urgent render must not rerun the field's count because the
    // list of fields came back as a new array of the same names.
    expect([...h.facetPasses].sort()).toEqual(["level", "system"]);
  });
});

describe("the volume histogram (#898)", () => {
  it("narrows the body and the export to a clicked bar, and the window's pill takes it back", async () => {
    const { container } = draw();
    const region = await body();
    push(...LINES);
    const bars = container.querySelectorAll('[data-slot="bucket"]');
    expect(bars).toHaveLength(4);
    fireEvent.pointerDown(bars[0]);
    fireEvent.pointerUp(bars[0]);
    expect(messages(region)).toEqual(["info starting checkout-api build=4f2a1c"]);
    await userEvent.click(screen.getByRole("button", { name: "Export" }));
    await waitFor(() => expect(h.saved).toHaveBeenCalled());
    expect(String(h.saved.mock.calls[0][1]).split("\n")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "Remove the time window" }));
    expect(rendered(region)).toHaveLength(4);
  });

  it("shades the window instead of cropping to it, and the rail counts what is inside it", async () => {
    const { container } = draw();
    await body();
    push(...JSON_LINES);
    const bars = () => container.querySelectorAll('[data-slot="bucket"]');
    expect(screen.getByRole("button", { name: "level = info, 1 line" })).toBeTruthy();
    fireEvent.pointerDown(bars()[1]);
    fireEvent.pointerUp(bars()[1]);
    // The histogram keeps all three bars, so the reader can still see the spike's surroundings.
    expect(bars()).toHaveLength(3);
    expect(screen.getByRole("button", { name: "level = warn, 1 line" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "level = info, 1 line" })).toBeNull();
  });

  it("takes the window away with the rest of the filters when nothing is left to show", async () => {
    const { container } = draw();
    const region = await body();
    push(
      line("api-7/api", "14:07:41.208000000", "info before the gap"),
      line("api-7/api", "14:07:44.010000000", "info after the gap"),
    );
    const bars = container.querySelectorAll('[data-slot="bucket"]');
    // Four one-second bars, and the second is empty.
    fireEvent.pointerDown(bars[1]);
    fireEvent.pointerUp(bars[1]);
    expect(rendered(region)).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "Clear the filter" }));
    expect(rendered(region)).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "Remove the time window" })).toBeNull();
  });
  describe("the legend", () => {
    it("filters on a level clicked in it, writing it into the search box, and takes it back", async () => {
      await (draw(), body());
      push(...LINES);
      await userEvent.click(screen.getByRole("button", { name: "error 1" }));
      expect(filterField()).toHaveProperty("value", "level:error");
      expect(screen.getByRole("button", { name: "error 1" }).getAttribute("aria-pressed")).toBe("true");
      await userEvent.click(screen.getByRole("button", { name: "error 1" }));
      expect(filterField()).toHaveProperty("value", "");
    });

    it("writes every level word of the group, and takes them all out again", async () => {
      await (draw(), body());
      push(
        line("api-7/api", "14:07:41.208000000", "error a"),
        line("api-7/api", "14:07:42.208000000", "fatal b"),
        line("api-7/api", "14:07:43.208000000", "info c"),
      );
      await userEvent.click(screen.getByRole("button", { name: "error 2" }));
      expect(filterField()).toHaveProperty("value", "level:error level:fatal");
      await userEvent.click(screen.getByRole("button", { name: "error 2" }));
      expect(filterField()).toHaveProperty("value", "");
    });

    const errorAndFatal = () =>
      push(
        line("api-7/api", "14:07:41.208000000", "error a"),
        line("api-7/api", "14:07:42.208000000", "fatal b"),
        line("api-7/api", "14:07:43.208000000", "info c"),
      );

    it("adds the words a group is missing when one is typed, and then takes them all out", async () => {
      await (draw(), body());
      errorAndFatal();
      await userEvent.type(filterField(), "level:error");
      // Not pressed: the group is error AND fatal, and only one is in the query.
      expect(screen.getByRole("button", { name: "error 2" }).getAttribute("aria-pressed")).toBe("false");
      await userEvent.click(screen.getByRole("button", { name: "error 2" }));
      expect(filterField()).toHaveProperty("value", "level:error level:fatal");
      expect(screen.getByRole("button", { name: "error 2" }).getAttribute("aria-pressed")).toBe("true");
      await userEvent.click(screen.getByRole("button", { name: "error 2" }));
      expect(filterField()).toHaveProperty("value", "");
    });

    it("turns an excluded word of the group into an included one", async () => {
      await (draw(), body());
      errorAndFatal();
      await userEvent.type(filterField(), "-level:fatal");
      await userEvent.click(screen.getByRole("button", { name: "error 2" }));
      expect(filterField()).toHaveProperty("value", "level:fatal level:error");
    });

    it("keeps the other levels on offer once one is picked, while the bars follow the query", async () => {
      await (draw(), body());
      push(...LINES);
      await userEvent.type(filterField(), "level:error");
      expect(screen.getByRole("button", { name: "warn 1" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "info 1" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "error 1" }).getAttribute("aria-pressed")).toBe("true");
      // The bars and the caption count only the line the query keeps.
      expect(document.querySelector('[data-slot="volume-caption"]')?.textContent).toMatch(/^1 line · /);
    });
  });
});

describe("the line detail (#898)", () => {
  const rows = () => Array.from(logRegion().querySelectorAll(".logline"));

  it("opens on a click, follows the arrow keys, and closes on Escape", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    const panel = () => screen.getByRole("region", { name: "Line detail" });
    expect(within(panel()).getByText("camlog")).toBeTruthy();
    expect(rows()[0].closest("[aria-current]")?.getAttribute("aria-current")).toBe("true");
    await userEvent.keyboard("{ArrowDown}");
    expect(within(panel()).getByText("schwarz")).toBeTruthy();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
  });

  it("filters from the panel through the search box", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    await userEvent.click(screen.getByRole("button", { name: "Filter out system = camlog" }));
    expect(filterField()).toHaveProperty("value", "-system:camlog");
  });

  const panel = () => screen.getByRole("region", { name: "Line detail" });

  it("makes the log a tab stop, and lets Escape through while no line is open", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    expect(logRegion().getAttribute("tabindex")).toBe("0");
    const outer = vi.fn();
    document.addEventListener("keydown", outer);
    try {
      logRegion().focus();
      await userEvent.keyboard("{Escape}");
      expect(outer).toHaveBeenCalledTimes(1);
      // With a line open the same key closes the panel and goes no further.
      await userEvent.click(rows()[0]);
      await userEvent.keyboard("{Escape}");
      expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
      expect(outer).toHaveBeenCalledTimes(1);
    } finally {
      document.removeEventListener("keydown", outer);
    }
  });

  it("shows where focus is on the log, rather than leaving the tab stop without an outline", async () => {
    await (draw(), body());
    expect(logRegion().classList.contains("log-body")).toBe(true);
    // jsdom draws no CSS; what can be held is that the rule the class names exists, in the accent. It is inset: the
    // log is a scroll container flush with the rail, the filter bar and the panel, so an outer ring would be cut.
    const css = readFileSync(join(__dirname, "../styles/next.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    const rule = css.match(/\.log-body:focus-visible\s*\{([^}]*)\}/)?.[1];
    expect(rule).toContain("outline: 2px solid var(--accent)");
    expect(rule).toContain("outline-offset: -2px");
  });

  it("starts from the end the key points to, and stops at the ends", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    logRegion().focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(within(panel()).getByText("Wrote batch")).toBeTruthy();
    await userEvent.keyboard("{ArrowUp}");
    expect(within(panel()).getByText("Wrote batch")).toBeTruthy();
    await userEvent.keyboard("{Escape}{ArrowUp}");
    expect(within(panel()).getByText("Success: Write failed")).toBeTruthy();
    await userEvent.keyboard("{ArrowDown}");
    expect(within(panel()).getByText("Success: Write failed")).toBeTruthy();
  });

  it("holds the line a filter has hidden, and says so", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    await userEvent.click(screen.getByRole("button", { name: "Filter out system = camlog" }));
    await waitFor(() => expect(rows()).toHaveLength(1));
    expect(within(panel()).getByText("Wrote batch")).toBeTruthy();
    expect(within(panel()).getByText("No longer in view")).toBeTruthy();
  });

  it("closes when the pane swaps to the previous instance's buffer", async () => {
    h.resolve.mockResolvedValue({ status: "resolved", targets: TARGETS, pods: PODS, previous: TERMINATED });
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    await userEvent.click(screen.getByRole("button", { name: /previous instance/i }));
    await waitFor(() => expect(rows()).toHaveLength(4));
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
  });

  it("returns focus to the log when Escape closes the panel from inside it", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    screen.getByRole("button", { name: /^copy$/i }).focus();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
    expect(document.activeElement).toBe(logRegion());
  });

  it("returns focus to the log when the close button closes the panel", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    await userEvent.click(screen.getByRole("button", { name: "Close the line detail" }));
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
    expect(document.activeElement).toBe(logRegion());
  });

  it("leaves a modified arrow to the browser", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    logRegion().focus();
    await userEvent.keyboard("{Control>}{ArrowDown}{/Control}{Alt>}{ArrowDown}{/Alt}{Meta>}{ArrowUp}{/Meta}{Shift>}{ArrowUp}{/Shift}");
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
  });

  it("does not open for a click that ended a text selection", async () => {
    await (draw(), body());
    push(...JSON_LINES);
    const selection = vi.spyOn(window, "getSelection").mockReturnValue({ toString: () => "camlog" } as unknown as Selection);
    try {
      await userEvent.click(rows()[0]);
      expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
    } finally {
      selection.mockRestore();
    }
  });

  it("closes when the source is switched, which is a different buffer", async () => {
    h.plugins = [observability()];
    await (draw(), body());
    push(...JSON_LINES);
    await userEvent.click(rows()[0]);
    expect(panel()).toBeTruthy();
    fireEvent.change(await screen.findByRole("combobox", { name: "Logs from" }), {
      target: { value: "org.example.observability/loki" },
    });
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
  });

  it("closes when the followed provider stops being offered", async () => {
    h.plugins = [observability()];
    await (draw(), body());
    push(...JSON_LINES);
    fireEvent.change(await screen.findByRole("combobox", { name: "Logs from" }), {
      target: { value: "org.example.observability/loki" },
    });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeDefined());
    await userEvent.click(rows()[0]);
    expect(panel()).toBeTruthy();
    act(() => {
      h.plugins = [];
      h.inventoryError = undefined;
      notify();
    });
    await waitFor(() => expect(h.seen.at(-1)?.options.source).toBeUndefined());
    expect(screen.queryByRole("region", { name: "Line detail" })).toBeNull();
  });

  describe("keeping the selected line in view", () => {
    /** jsdom does no layout: declare the viewport's geometry, and clamp a write to scrollTop the way a browser does. */
    function scrollable(el: HTMLElement) {
      let top = 0;
      const max = () => h.state.lines.length * 20 - 100;
      Object.defineProperty(el, "clientHeight", { configurable: true, get: () => 100 });
      Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => h.state.lines.length * 20 });
      Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => top,
        set: (v: number) => {
          top = Math.max(0, Math.min(v, max()));
        },
      });
      return {
        get top() {
          return top;
        },
        to(v: number) {
          top = v;
          fireEvent.scroll(el);
        },
        /** What a browser does when it lays the page out after the content got shorter. */
        clamp() {
          top = Math.min(top, max());
        },
      };
    }
    const many = (n: number, from = 0) =>
      Array.from({ length: n }, (_, i) => line("api-7/api", "14:07:41.208000000", `line ${from + i}`));

    /** What the pane reveals through: the browser does the pixels, so the test sees only which row it was asked about. */
    function revealed() {
      const spy = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
      return {
        spy,
        /** The element of the latest reveal. */
        get last() {
          return spy.mock.contexts.at(-1) as HTMLElement | undefined;
        },
      };
    }

    it("reveals the newly selected row, with the earlier-lines banner above the rows", async () => {
      const restore = measurableRows();
      const reveal = revealed();
      try {
        const region = await (draw(), body());
        h.state.dropped = 5;
        push(...JSON_LINES);
        expect(screen.getByText(/earlier lines dropped/)).toBeTruthy();
        region.focus();
        await userEvent.keyboard("{ArrowDown}{ArrowDown}");
        expect(reveal.last).toBe(rows()[1].closest("[aria-current]"));
        expect(reveal.spy).toHaveBeenLastCalledWith({ block: "nearest" });
        // An arrow at the end of the buffer selects the same row again, and still brings it back into view.
        await userEvent.keyboard("{ArrowDown}");
        reveal.spy.mockClear();
        await userEvent.keyboard("{ArrowDown}");
        expect(reveal.spy).toHaveBeenCalledTimes(1);
        expect(reveal.last).toBe(rows()[2].closest("[aria-current]"));
      } finally {
        reveal.spy.mockRestore();
        restore();
      }
    });

    it("reveals the newly selected row when lines wrap, because their heights differ", async () => {
      const restore = measurableRows();
      const reveal = revealed();
      try {
        const region = await (draw(), body());
        push(...JSON_LINES);
        await userEvent.click(screen.getByRole("button", { name: "Wrap" }));
        region.focus();
        await userEvent.keyboard("{ArrowDown}{ArrowDown}");
        expect(reveal.last).toBe(rows()[1].closest("[aria-current]"));
        expect(reveal.spy).toHaveBeenLastCalledWith({ block: "nearest" });
      } finally {
        reveal.spy.mockRestore();
        restore();
      }
    });

    it("brings a windowed-out row into the drawn slice, then reveals it", async () => {
      const restore = measurableRows();
      const reveal = revealed();
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        push(...many(300));
        view.to(0);
        expect(rows().some((r) => r.textContent?.includes("line 299"))).toBe(false);
        region.focus();
        // ArrowUp from nothing selected goes to the last line, 6,000px down and not drawn.
        await userEvent.keyboard("{ArrowUp}");
        expect(view.top).toBeGreaterThan(0);
        // The browser reports the scroll; the slice follows it and the row now has an element.
        view.to(view.top);
        expect(reveal.last?.textContent).toContain("line 299");
        expect(reveal.last?.getAttribute("aria-current")).toBe("true");
        // Once revealed, the reader's own scrolling is theirs: a few rows up moves the slice, the selected row
        // is still drawn, and the view is not pulled back to it.
        reveal.spy.mockClear();
        view.to(view.top - 100);
        expect(reveal.spy).not.toHaveBeenCalled();
      } finally {
        reveal.spy.mockRestore();
        restore();
      }
    });

    it("stays on the selected line when the reveal landed at the end and a new line arrives", async () => {
      // A browser's scrollIntoView moves the viewport and then reports the scroll. That report is not the reader
      // returning to the end: with a line selected the view is pinned to it.
      const region = await (draw(), body());
      const view = scrollable(region);
      const reveal = vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {
        region.scrollTop = region.scrollHeight;
      });
      try {
        push(...many(20));
        view.to(0);
        region.focus();
        await userEvent.keyboard("{ArrowUp}");
        expect(view.top).toBe(300);
        fireEvent.scroll(region);
        push(...many(5, 20));
        expect(view.top).toBe(300);
      } finally {
        reveal.mockRestore();
      }
    });

    it("follows the end again once the line is closed and the reader is back at it", async () => {
      const region = await (draw(), body());
      const view = scrollable(region);
      push(...many(20));
      region.focus();
      await userEvent.keyboard("{ArrowUp}{Escape}");
      view.to(300);
      push(...many(5, 20));
      expect(view.top).toBe(400);
    });

    it("reveals a clicked row, which the panel opening under it would otherwise cover", async () => {
      const reveal = revealed();
      try {
        await (draw(), body());
        push(...many(20));
        await userEvent.click(rows()[19]);
        expect(reveal.last).toBe(rows()[19].closest("[aria-current]"));
        expect(reveal.spy).toHaveBeenLastCalledWith({ block: "nearest" });
        // The same row again still asks, though no render is coming to do it.
        reveal.spy.mockClear();
        await userEvent.click(rows()[19]);
        expect(reveal.spy).toHaveBeenCalledTimes(1);
      } finally {
        reveal.spy.mockRestore();
      }
    });

    it("stops following the end once a row is clicked, so a new line cannot carry it away", async () => {
      const region = await (draw(), body());
      const view = scrollable(region);
      push(...many(20));
      expect(view.top).toBe(300);
      await userEvent.click(rows()[19]);
      push(...many(5, 20));
      expect(view.top).toBe(300);
    });

    /**
     * What the ring does once it is full: a batch drops `dropped` lines from the front and adds the same
     * number at the end, so the buffer keeps its size and everything after the dropped lines moves up.
     */
    function ring(dropped: number, nextLine: number) {
      act(() => {
        h.state.lines = [...h.state.lines.slice(dropped), ...many(dropped, nextLine)];
        h.state.dropped += dropped;
        notify();
      });
    }
    const indexOf = (n: number) => h.state.lines.findIndex((l) => l.text.endsWith(`line ${n}`));
    const selectedRow = () => logRegion().querySelector('[aria-current="true"]') as HTMLElement | null;
    const lineRow = (n: number) => rows().find((r) => r.textContent?.includes(`line ${n}`)) as Element;

    const ROW = 20;
    const BANNER = 30;
    /**
     * jsdom lays nothing out. Stack the children of the rows box the way a browser does, one under the next: a
     * spacer as tall as its style says, the earlier-lines banner BANNER tall, a line `heightOf(wrapper)` tall.
     * `onLayout` runs on every measurement, as the browser's own work on the page does before it answers one.
     */
    function layOut(heightOf: (wrapper: HTMLElement) => number = () => ROW, onLayout: () => void = () => undefined) {
      const proto = window.HTMLElement.prototype;
      const original = proto.getBoundingClientRect;
      const tall = (el: Element) => {
        const box = el as HTMLElement;
        if (box.getAttribute("data-slot") === "dropped-banner") return BANNER;
        return box.style.height !== "" ? parseFloat(box.style.height) : heightOf(box);
      };
      proto.getBoundingClientRect = function (this: HTMLElement) {
        onLayout();
        const box = this.parentElement;
        if (box?.parentElement?.getAttribute("role") === "log") {
          const above = Array.from(box.children).slice(0, Array.from(box.children).indexOf(this));
          return { top: above.reduce((t, el) => t + tall(el), 0), height: tall(this) } as DOMRect;
        }
        // The first row's height is what the screen windows by.
        if (this.classList.contains("logline")) return { height: ROW } as DOMRect;
        return original.call(this);
      };
      return () => {
        proto.getBoundingClientRect = original;
      };
    }
    /** Where the selected row is drawn on screen: where the layout stacks it, less how far the view has scrolled. */
    const onScreen = (view: { top: number }) => (selectedRow() as HTMLElement).getBoundingClientRect().top - view.top;

    it("holds the selected line where it is on screen while a full ring drops lines from the front", async () => {
      const restore = layOut();
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        // The ring is already full, so the earlier-lines banner is above the rows before and after each batch.
        h.state.dropped = 100;
        push(...many(300));
        view.to(2000);
        await userEvent.click(lineRow(100));
        // Where the row sits on screen: the banner, then its place in the buffer at 20px a row, less the scroll.
        const offset = () => BANNER + indexOf(100) * ROW - view.top;
        const before = offset();
        expect(onScreen(view)).toBe(before);
        ring(30, 300);
        expect(indexOf(100)).toBe(70);
        expect(offset()).toBe(before);
        // The view moved with the content, so the row is still drawn, in the same place.
        expect(selectedRow()?.textContent).toContain("line 100");
        expect(onScreen(view)).toBe(before);
        // The reader's own scrolling is theirs: it is not undone, and the next batch holds the row wherever they left it.
        view.to(1000);
        // A nudge of less than a row moves no slice, so nothing is redrawn for it: the next batch still starts from where the reader is.
        view.to(1010);
        expect(view.top).toBe(1010);
        const left = offset();
        ring(10, 330);
        expect(indexOf(100)).toBe(60);
        expect(offset()).toBe(left);
      } finally {
        restore();
      }
    });

    it("holds the selected line through the first drop, which also brings in the earlier-lines banner", async () => {
      const restore = layOut();
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        push(...many(300));
        view.to(2000);
        await userEvent.click(lineRow(100));
        expect(screen.queryByText(/earlier lines dropped/)).toBeNull();
        // 300 lines are windowed: after the batch the row is outside the slice drawn from the old scroll position.
        const offset = () => (h.state.dropped > 0 ? BANNER : 0) + indexOf(100) * ROW - view.top;
        const before = offset();
        ring(30, 300);
        expect(screen.getByText(/earlier lines dropped/)).toBeTruthy();
        expect(offset()).toBe(before);
        expect(onScreen(view)).toBe(before);
      } finally {
        restore();
      }
    });

    // A line is as tall as its text needs when lines wrap: here 20, 30 or 40px by its number.
    const byNumber = (wrapper: HTMLElement) => 20 + (Number(wrapper.textContent?.match(/line (\d+)/)?.[1]) % 3) * 10;

    it("holds a drawn selected line through the first drop by where its row is drawn, not by the row height", async () => {
      // Few enough lines that all are drawn. The first row, which the screen measures its row height from, is 20px,
      // and the rows above the selected one are not: only the drawn row knows where it went.
      const restore = layOut(byNumber);
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        push(...many(30));
        view.to(200);
        await userEvent.click(lineRow(20));
        const before = onScreen(view);
        ring(5, 30);
        expect(screen.getByText(/earlier lines dropped/)).toBeTruthy();
        // Lines 0 to 4 are 20, 30, 40, 20 and 30px, and a banner of 30 arrived: 110px up.
        expect(view.top).toBe(200 - 140 + 30);
        expect(onScreen(view)).toBe(before);
      } finally {
        restore();
      }
    });

    it("does not correct twice when the browser has already pulled the scroll position back", async () => {
      // A batch that shrinks the content leaves the scroll position past the new end, and the browser clamps it
      // when it lays the page out: before the screen reads it, and before it has moved the view.
      let view: ReturnType<typeof scrollable> | undefined;
      const restore = layOut(undefined, () => view?.clamp());
      try {
        const region = await (draw(), body());
        view = scrollable(region);
        h.state.dropped = 100;
        push(...many(30));
        view.to(480);
        await userEvent.click(lineRow(25));
        const before = onScreen(view);
        act(() => {
          h.state.lines = h.state.lines.slice(10);
          h.state.dropped += 10;
          notify();
        });
        expect(view.top).toBe(280);
        expect(onScreen(view)).toBe(before);
      } finally {
        restore();
      }
    });

    it("holds the selected line from where the reveal put the view, though the reveal's scroll event has not arrived", async () => {
      // A click reveals its row, and scrollIntoView moves the view at once; the scroll event only arrives at the next
      // rendering step. A batch that lands in between must not start from the position before the reveal.
      const restore = layOut();
      const reveal = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        h.state.dropped = 100;
        push(...many(30));
        view.to(100);
        // Down to the bottom of the pane, as a browser does for a row below it, and with no event.
        reveal.mockImplementation(() => {
          region.scrollTop = 250;
        });
        await userEvent.click(lineRow(20));
        expect(view.top).toBe(250);
        const before = onScreen(view);
        ring(5, 30);
        // Five rows of 20px left above it: 100px up from where the reveal left the view, not from where it was.
        expect(view.top).toBe(150);
        expect(onScreen(view)).toBe(before);
      } finally {
        reveal.mockRestore();
        restore();
      }
    });

    it("holds the selected line by where its row is drawn when lines wrap and rows differ in height", async () => {
      // No sum of one row height says where a wrapped row is.
      const restore = layOut(byNumber);
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        h.state.dropped = 100;
        push(...many(30));
        await userEvent.click(screen.getByRole("button", { name: "Wrap" }));
        view.to(300);
        await userEvent.click(lineRow(20));
        const before = onScreen(view);
        ring(5, 30);
        expect(indexOf(20)).toBe(15);
        // Lines 0 to 4 are 20, 30, 40, 20 and 30px.
        expect(view.top).toBe(300 - 140);
        expect(onScreen(view)).toBe(before);
      } finally {
        restore();
      }
    });

    it("leaves the view alone when the ring drops the selected line itself, and when lines only arrive", async () => {
      const restore = measurableRows();
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        push(...many(300));
        view.to(2000);
        await userEvent.click(rows().find((r) => r.textContent?.includes("line 100")) as Element);
        push(...many(10, 300));
        expect(view.top).toBe(2000);
        // Line 100 is among the first 150 lines, which this batch drops: it is no longer in the buffer.
        ring(150, 310);
        expect(indexOf(100)).toBe(-1);
        expect(view.top).toBe(2000);
      } finally {
        restore();
      }
    });

    it("does not take the selection moving to another line for the content moving", async () => {
      const restore = measurableRows();
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        push(...many(300));
        view.to(2000);
        await userEvent.click(rows().find((r) => r.textContent?.includes("line 100")) as Element);
        region.focus();
        await userEvent.keyboard("{ArrowDown}{ArrowDown}");
        expect(selectedRow()?.textContent).toContain("line 102");
        expect(view.top).toBe(2000);
      } finally {
        restore();
      }
    });

    it("stops following the end once a line is selected", async () => {
      const restore = measurableRows();
      try {
        const region = await (draw(), body());
        const view = scrollable(region);
        push(...many(20));
        expect(view.top).toBe(300);
        region.focus();
        await userEvent.keyboard("{ArrowUp}");
        push(...many(5, 20));
        expect(view.top).toBe(300);
      } finally {
        restore();
      }
    });
  });
});

describe("under a busy stream (#898)", () => {
  const caption = () => document.querySelector('[data-slot="volume-caption"]')?.textContent ?? "";
  const anotherError = line("api-8/api", "14:07:45.100000000", "error pool timeout waited=30.1s in_use=6");
  const many = (n: number, from = 0) =>
    Array.from({ length: n }, (_, i) => line("api-7/api", "14:07:41.208000000", `line ${from + i}`));
  /** A full ring's next batch: `n` lines off the front and `n` new ones on the end. */
  function ring(n: number, next: number) {
    act(() => {
      h.state.lines = [...h.state.lines.slice(n), ...many(n, next)];
      h.state.dropped += n;
      notify();
    });
  }
  const lineRow = (n: number) =>
    Array.from(logRegion().querySelectorAll(".logline")).find((r) => r.textContent?.includes(`line ${n}`)) as Element;

  it("keeps a drawn line on its own element while a full ring drops lines from the front", async () => {
    // An element that is handed another line under the pointer turns a click, or a selection, into one of that line.
    await (draw(), body());
    push(...many(30));
    const drawn = lineRow(10);
    ring(5, 30);
    expect(drawn.isConnected).toBe(true);
    expect(drawn.textContent).toContain("line 10");
  });

  it("keeps a drawn line on its own element when the targets are resolved again, after the ring has dropped lines", async () => {
    // A row's id belongs to the module, not to one indexing of the targets: numbered afresh from 0 by place in the
    // buffer, a rebuilt row takes the id of a line the ring has since dropped, and that line's element with it.
    const region = await (draw(), body());
    push(...many(30));
    ring(5, 30);
    const drawn = lineRow(10);
    // The same stream under a renamed context: the subject is resolved again, to targets the screen indexes afresh.
    // The answer lands in the request's own tick, so React takes the loading state and the new targets as one render
    // and the screen is rebuilt in place; an answer a network later unmounts it, and with it every drawn element.
    const again = TARGETS.map((t) => ({ ...t }));
    const answer = { status: "resolved", targets: again, pods: PODS, previous: [] };
    h.resolve.mockReturnValue({ then: (done: (a: typeof answer) => void) => done(answer) });
    act(() => setContexts([{ ...CTX, name: "prod-eu-2" }]));
    expect(h.seen[h.seen.length - 1].targets).toBe(again);
    expect(region.isConnected).toBe(true);
    expect(rendered(region)).toHaveLength(30);
    // Replaced by an element of its own, or still its own line: never another's.
    expect(drawn.isConnected ? drawn.textContent : "line 10").toContain("line 10");
  });

  it("recounts the rail, the facets and the histogram at most four times a second, and the last batch always lands", async () => {
    // The quarter second is read off the clock; stopped, a slow machine cannot run it out between two batches.
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      const region = await (draw(), body());
      push(...LINES);
      expect(screen.getByText("1 error")).toBeTruthy();
      push(anotherError);
      // The body draws every batch.
      expect(rendered(region)).toHaveLength(5);
      // The counts wait out the quarter second since their last recount.
      expect(screen.getByText("1 error")).toBeTruthy();
      expect(screen.getByRole("button", { name: "level = error, 1 line" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "error 1" })).toBeTruthy();
      expect(caption()).toMatch(/^4 lines · /);
      await waitFor(() => {
        expect(screen.getByText("2 errors")).toBeTruthy();
        expect(screen.getByRole("button", { name: "level = error, 2 lines" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "error 2" })).toBeTruthy();
        expect(caption()).toMatch(/^5 lines · /);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("empties the counts at once when the buffer is emptied, though a recount is waiting", async () => {
    // A restart leaves nothing to count: the old buffer's tallies must not go on being read for the quarter second a
    // recount waits. The clock is stopped, so that quarter second cannot run out inside the test.
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      const region = await (draw(), body());
      push(...LINES);
      expect(screen.getByText("1 error")).toBeTruthy();
      push(anotherError);
      // The recount is waiting: the counts still hold the last one.
      expect(screen.getByText("1 error")).toBeTruthy();
      act(() => {
        // What the hook does on a restart: a fresh buffer and a bumped restart count.
        h.state.lines = [];
        h.state.restarts = 1;
        notify();
      });
      expect(rendered(region)).toHaveLength(0);
      expect(screen.queryByText("1 error")).toBeNull();
      expect(screen.queryByRole("button", { name: "error 1" })).toBeNull();
      expect(caption()).toBe("");
    } finally {
      vi.useRealTimers();
    }
  });

  describe("holding the view while the reader works in it", () => {
    const heldNote = () => screen.queryByText(/^Held while you select/);
    /** The task after a release, which is when the screen lets go of a press. */
    const released = () => act(() => new Promise((r) => setTimeout(r, 0)));
    /** A press on `el`, and its release, as a browser sends them: the pointer's event, then the mouse's. */
    const press = (el: Element) => {
      fireEvent.pointerDown(el);
      fireEvent.mouseDown(el);
    };
    const lift = (el: Element) => {
      fireEvent.pointerUp(el);
      fireEvent.mouseUp(el);
    };
    /** Select `node`'s text, a caret at its start, or nothing, and say so the way the browser does. */
    const select = (node: Node | null, caret = false) =>
      act(() => {
        const selection = document.getSelection() as Selection;
        if (node === null) selection.removeAllRanges();
        else if (caret) selection.collapse(node, 0);
        else selection.selectAllChildren(node);
        document.dispatchEvent(new Event("selectionchange"));
      });

    it("holds the view while the mouse is down in the log, and lets go when it comes up, leaving Pause alone", async () => {
      const region = await (draw(), body());
      push(...many(5));
      press(lineRow(2));
      push(...many(3, 5));
      expect(rendered(region)).toHaveLength(5);
      // The reader's own control still says what they chose.
      expect(screen.getByRole("button", { name: "Pause" })).toBeTruthy();
      lift(lineRow(2));
      await released();
      expect(rendered(region)).toHaveLength(8);
    });

    it.each([
      ["a pointerup anywhere in the window", () => fireEvent.pointerUp(window)],
      ["a pointercancel", () => fireEvent.pointerCancel(window)],
      ["the window losing focus", () => fireEvent.blur(window)],
      [
        "the window being hidden",
        () => {
          Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
          fireEvent(document, new Event("visibilitychange"));
        },
      ],
    ])("lets go of a press on %s, though not of a selection", async (_, end) => {
      // A release that never reaches the log, or never comes at all, must not leave the view held.
      try {
        const region = await (draw(), body());
        push(...many(5));
        press(lineRow(2));
        push(...many(3, 5));
        expect(rendered(region)).toHaveLength(5);
        end();
        await released();
        expect(rendered(region)).toHaveLength(8);
        // A selection is let go of only by collapsing or leaving the log. (jsdom also queues a selectionchange of
        // its own a task after the selection; it is let through first, so it cannot put back what `end` took.)
        select(lineRow(2));
        await released();
        push(...many(1, 8));
        end();
        await released();
        expect(rendered(region)).toHaveLength(8);
      } finally {
        delete (document as { hidden?: boolean }).hidden;
      }
    });

    it("lets go of a press though something on the page stops its release on the way up", async () => {
      const region = await (draw(), body());
      push(...many(5));
      press(lineRow(2));
      push(...many(3, 5));
      const swallow = (e: Event) => e.stopPropagation();
      document.body.addEventListener("pointerup", swallow);
      try {
        fireEvent.pointerUp(document.body);
        await released();
        expect(rendered(region)).toHaveLength(8);
      } finally {
        document.body.removeEventListener("pointerup", swallow);
      }
    });

    it("keeps holding when the press takes focus from the search box: only the window losing it lets go", async () => {
      const region = await (draw(), body());
      push(...many(5));
      press(lineRow(2));
      fireEvent.blur(filterField());
      push(...many(3, 5));
      await released();
      expect(rendered(region)).toHaveLength(5);
    });

    it("lets go of nothing once the screen has gone, though the release came before it went", async () => {
      // The hold is let go of a task after the release; by then there may be no screen, and no stream, to let go.
      const view = draw();
      await body();
      push(...many(5));
      press(lineRow(2));
      fireEvent.pointerUp(window);
      view.unmount();
      await released();
      expect(h.state.held).toBe(true);
    });

    it("holds the view while text is selected in the log, and lets go when it is cleared", async () => {
      const region = await (draw(), body());
      push(...many(5));
      // A selection elsewhere on the screen is not the log's.
      select(screen.getByRole("button", { name: "Wrap" }));
      push(...many(1, 5));
      expect(rendered(region)).toHaveLength(6);
      select(lineRow(2));
      push(...many(3, 6));
      expect(rendered(region)).toHaveLength(6);
      // A caret is no selection: it is where a click in the log leaves one.
      select(lineRow(2), true);
      expect(rendered(region)).toHaveLength(9);
      select(lineRow(2));
      push(...many(1, 9));
      expect(rendered(region)).toHaveLength(9);
      select(null);
      expect(rendered(region)).toHaveLength(10);
    });

    it.each([
      ["Ctrl+A", () => select(document.body)],
      [
        "a drag from the filter bar to past the log",
        () =>
          act(() => {
            const wrap = screen.getByRole("button", { name: "Wrap" });
            (document.getSelection() as Selection).setBaseAndExtent(wrap, 0, document.body, document.body.childNodes.length);
            document.dispatchEvent(new Event("selectionchange"));
          }),
      ],
    ])("holds the view while %s selects the log, though neither end of the selection is in it", async (_, enclose) => {
      // The text under the selection is rewritten as surely as under one that starts in the log.
      const region = await (draw(), body());
      push(...many(5));
      enclose();
      const selection = document.getSelection() as Selection;
      expect(region.contains(selection.anchorNode) || region.contains(selection.focusNode)).toBe(false);
      push(...many(3, 5));
      expect(rendered(region)).toHaveLength(5);
      select(null);
      expect(rendered(region)).toHaveLength(8);
    });

    it("says the view is held only once a hold has lasted, never for an ordinary click", async () => {
      // Every click holds the view for as long as the button is down; a note that flashed on each one would be noise.
      await (draw(), body());
      push(...many(5));
      vi.useFakeTimers();
      try {
        press(lineRow(2));
        act(() => vi.advanceTimersByTime(100));
        expect(heldNote()).toBeNull();
        lift(lineRow(2));
        act(() => vi.advanceTimersByTime(1000));
        expect(heldNote()).toBeNull();
        // A selection kept is a hold worth naming, and what waits behind it.
        select(lineRow(3));
        h.state.pending = 3;
        push(...many(3, 5));
        act(() => vi.advanceTimersByTime(299));
        expect(heldNote()).toBeNull();
        act(() => vi.advanceTimersByTime(1));
        expect(heldNote()?.textContent).toBe("Held while you select · 3 new lines");
        select(null);
        expect(heldNote()).toBeNull();
        // The next hold is timed afresh: a click after a long selection is still only a click.
        press(lineRow(2));
        act(() => vi.advanceTimersByTime(100));
        expect(heldNote()).toBeNull();
        lift(lineRow(2));
        act(() => vi.advanceTimersByTime(0));
      } finally {
        vi.useRealTimers();
      }
    });

    it("tells a screen reader the view is held while the note shows, without the count that changes under it", async () => {
      await (draw(), body());
      push(...many(5));
      const live = () => document.querySelector('[data-slot="held-live"]') as HTMLElement;
      // Mounted before anything is held, so the first announcement is a change to a region the reader already has.
      expect(live().textContent).toBe("");
      expect(live().getAttribute("role")).toBe("status");
      expect(live().getAttribute("aria-live")).toBe("polite");
      expect(live().getAttribute("aria-atomic")).toBe("true");
      expect(live().className).toContain("sr-only");
      vi.useFakeTimers();
      try {
        select(lineRow(3));
        h.state.pending = 3;
        push(...many(3, 5));
        act(() => vi.advanceTimersByTime(299));
        expect(live().textContent).toBe("");
        act(() => vi.advanceTimersByTime(1));
        expect(heldNote()?.textContent).toBe("Held while you select · 3 new lines");
        // The count is the visible note's alone: said here, it would be read out again on every batch.
        expect(live().textContent).toBe("View held while you select");
        h.state.pending = 40;
        push(...many(1, 8));
        expect(live().textContent).toBe("View held while you select");
        select(null);
        expect(live().textContent).toBe("");
      } finally {
        vi.useRealTimers();
      }
    });

    it("opens the line detail on the line pressed, though lines arrive and the view follows them before the release", async () => {
      // A following view moves on every batch: the release lands on another row, or on none, and is no click at all.
      const restore = measurableRows();
      try {
        const region = await (draw(), body());
        // jsdom lays nothing out: a viewport 100px tall over the lines the screen is handed, 20px each.
        let top = 0;
        const height = () => (h.state.held ? h.state.heldLines : h.state.lines).length * 20;
        Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 100 });
        Object.defineProperty(region, "scrollHeight", { configurable: true, get: height });
        Object.defineProperty(region, "scrollTop", {
          configurable: true,
          get: () => top,
          set: (v: number) => {
            top = Math.max(0, Math.min(v, height() - 100));
          },
        });
        push(...many(300));
        fireEvent.scroll(region);
        const pressed = lineRow(297);
        press(pressed);
        push(...many(100, 300));
        // The browser reports the scroll that following the end made.
        fireEvent.scroll(region);
        lift(pressed);
        fireEvent.click(pressed);
        expect(screen.getByRole("region", { name: "Line detail" }).textContent).toContain("line 297");
        // Then the screen lets the lines that waited in, under the panel still on that line.
        expect(h.state.held).toBe(true);
        await released();
        expect(h.state.held).toBe(false);
        expect(screen.getByRole("region", { name: "Line detail" }).textContent).toContain("line 297");
      } finally {
        restore();
      }
    });
  });

  it("recounts at once while paused, when nothing is arriving to hold back", async () => {
    await (draw(), body());
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    push(...LINES);
    push(anotherError);
    expect(screen.getByText("2 errors")).toBeTruthy();
    expect(caption()).toMatch(/^5 lines · /);
  });
});
