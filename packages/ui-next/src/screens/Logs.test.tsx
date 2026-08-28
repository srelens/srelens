import { describe, it, expect, vi, beforeEach } from "vitest";
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
      /** Per-target truth: how many are streaming, how many are down, of how
       *  many altogether. */
      live: 4,
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
    /** Every call the screen has made into the hook, in order. */
    seen: [] as {
      context: string;
      namespace: string;
      targets: { pod: string; container?: string; label?: string }[];
      options: { sinceSeconds?: number; tailLines?: number; timestamps?: boolean };
    }[],
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
      options: { sinceSeconds?: number; tailLines?: number; timestamps?: boolean } = {},
    ) => {
      useSyncExternalStore(
        h.subscribe,
        () => h.version,
        () => h.version,
      );
      h.seen.push({ context, namespace, targets, options });
      return {
        lines: h.state.lines,
        dropped: h.state.dropped,
        status: h.state.status,
        error: h.state.error,
        paused: h.state.paused,
        pendingWhilePaused: h.state.pending,
        liveTargets: h.state.live,
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
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  isTauri: () => h.tauri,
  saveTextFile: (...a: unknown[]) => h.saved(...a),
  podLogs: (...a: unknown[]) => h.fetched(...a),
  // The bare route's only backend call: one list per kind and namespace, to
  // find out which remembered subjects the cluster still has.
  listResource: (...a: unknown[]) => h.listed(...a),
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
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";

/** Push the version forward and wake every mounted hook. */
function notify() {
  h.version += 1;
  for (const listener of [...h.listeners]) listener();
}

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
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

const LINES = [
  line("api-7/api", "14:07:41.208000000", "info starting checkout-api build=4f2a1c"),
  line("api-7/otel-sidecar", "14:07:42.100000000", "warn exporter queue is full"),
  line("api-8/api", "14:07:43.900000000", "error pool timeout waited=30.0s in_use=5"),
  line("api-9/api", "14:07:44.010000000", "GET /healthz 200 1ms"),
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
  h.state = {
    lines: [],
    dropped: 0,
    status: "live",
    error: undefined,
    paused: false,
    pending: 0,
    live: 4,
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
function push(...lines: { source: string; text: string }[]) {
  act(() => {
    h.state.lines = [...h.state.lines, ...lines];
    notify();
  });
}

/**
 * The log region, after the screen has settled.
 *
 * `findByRole` alone returns the element as soon as one appears, which is
 * before `resolveLogSubject` has settled — and when it does settle, React can
 * replace that node. Tests then held a DETACHED element: `querySelectorAll`
 * answers nothing, and a `clientHeight` defined on it is lost, so the window
 * sees a zero-height viewport and draws no rows at all. That surfaced as four
 * different tests in this file failing about one run in six, each passing
 * alone, and it is why the flakes moved around rather than sitting still.
 *
 * Flushing pending promises first, then re-querying, hands back the node the
 * screen actually settled on.
 *
 * **The ONLY barrier in this file, and the only `findByRole("log")` in it.**
 * The helper existed for thirty-five tests and was used by twenty-eight of
 * them; the other thirty-five sites still awaited `findByRole` raw, which is
 * the same unsettled node under a different name. #375 is one of those:
 * "draws a Sources row per pod" took the unsettled node, pushed lines into the
 * buffer, and the rail tallied an empty one — a failure that reached CI twice,
 * on #373 and again on #380. A raw `findByRole("log")` added back here is that
 * flake returning, so there is exactly one left and it is the line below.
 */
const body = async () => {
  await screen.findByRole("log", { name: /logs/i });
  await act(async () => {});
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
      "14:07:41.208|api-7 · api|info|info starting checkout-api build=4f2a1c",
      "14:07:42.100|api-7 · otel-sidecar|warn|warn exporter queue is full",
      "14:07:43.900|api-8 · api|error|error pool timeout waited=30.0s in_use=5",
      "14:07:44.010|api-9 · api||GET /healthz 200 1ms",
    ]);
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
      "14:07:41.208|web-0|info|info serving on :8080",
      "14:07:42.100|web-1|warn|warn upstream slow",
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
        "14:07:11.004|api-7 · api|info|info starting checkout-api build=4f2a1c pool_size=5",
        "14:07:12.880|api-7 · api|warn|warn pool saturated within 1.8s of accepting traffic",
        "14:07:41.902|api-7 · api|error|error pool timeout waited=30.0s pool_size=5 in_use=5",
        "14:07:42.410|api-7 · api|fatal|fatal liveness deadline exceeded, terminating",
      ]),
    );
    // And the live lines are gone from the pane, not merged into it: the two
    // buffers are different instances of the same container, and interleaving
    // them would invent a history that never happened.
    expect(rendered(region).join("\n")).not.toContain("exporter queue is full");
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
