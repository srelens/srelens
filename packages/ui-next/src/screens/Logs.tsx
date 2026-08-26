import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  absoluteTimestamp,
  isTauri,
  logConnectionStatus,
  logLineHealth,
  listResource,
  logLineLevel,
  podLogs,
  saveTextFile,
  type HealthKind,
  type LogConnectionVerdict,
  type LogLine as StreamLine,
  type LogTarget,
} from "@srelens/core";
import {
  Alert,
  AskChip,
  Button,
  EmptyState,
  Eyebrow,
  FilterBar,
  LiveSignal,
  LoadingState,
  LogLine,
  Screen,
  Select,
  SideRail,
  computeLogWindow,
  statusTone,
  toneColor,
  toneWash,
} from "@srelens/ui-kit";
import { useConsole } from "../console";
import { useActiveContext } from "../lib/clusters";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { useLogStream, type LogStreamStatus } from "../lib/logStream";
import { groupNumber } from "../lib/numbers";
import {
  resolveLogSubject,
  type LogSubject,
  type LogSubjectPod,
  type LogSubjectResolution,
  type PreviousInstance,
} from "../lib/logSubject";
import {
  forgetLogSubjects,
  recentKey,
  rememberLogSubject,
  reviewRecents,
  scanKey,
  useRecentLogSubjects,
  type SubjectScan,
} from "../lib/logRecents";
import { openTab } from "../lib/tabsStore";
import {
  StreamRail,
  STREAM_RAIL_WIDTH,
  type StreamPod,
} from "./logs/StreamRail";
import { NoClusterScreen } from "./resourceShell";

/**
 * `/logs/<kind>/<namespace>/<name>` — the live tail of a workload's pods, or
 * of one pod.
 *
 * The subject is in the route rather than in a store because `openTab` dedupes
 * by route string, so the route IS the stream's identity: two workloads
 * followed at once must be two tabs, and the same one opened twice must be
 * one. Same reasoning, and the same four-segment shape, as `detailRoute`.
 */
export function logsRoute(
  kind: string,
  namespace: string,
  name: string,
): string {
  return `/logs/${encodeURIComponent(kind)}/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`;
}

export interface LogsRouteParts {
  /** `Pod` for a single pod; any workload kind `getObject` understands otherwise. */
  kind: string;
  namespace: string;
  name: string;
}

/**
 * The inverse of {@link logsRoute}. Counts segments rather than
 * pattern-matching, because a decoded name can contain anything — including a
 * `/`, which is why every segment is encoded on the way in.
 *
 * A bare `/logs` parses to `null` rather than to a subject nobody named; the
 * screen has its own copy for that.
 */
export function parseLogsRoute(route: string): LogsRouteParts | null {
  const segments = route.split("/");
  if (segments.length !== 5) return null;
  const [empty, prefix, rawKind, rawNamespace, rawName] = segments;
  if (empty !== "" || prefix !== "logs") return null;
  if (!rawKind || !rawNamespace || !rawName) return null;
  try {
    return {
      kind: decodeURIComponent(rawKind),
      namespace: decodeURIComponent(rawNamespace),
      name: decodeURIComponent(rawName),
    };
  } catch {
    // `decodeURIComponent` THROWS a `URIError` on a malformed escape — `%zz`,
    // or a truncated multi-byte sequence. This parser runs DURING RENDER
    // (`describe` and `screenFor` both call it) over routes that are
    // persisted and restored, so a throw here is not a bad tab: it is the
    // whole new-design window failing to boot off one corrupted string in
    // storage. `null` is the contract's existing answer for a route this
    // function cannot make a subject of, and a route whose segments are not
    // decodable is one of those.
    return null;
  }
}

/** The `since` windows the design offers, and what each one means in seconds. */
const SINCE: { value: string; seconds: number | undefined }[] = [
  { value: "5m", seconds: 300 },
  { value: "15m", seconds: 900 },
  { value: "1h", seconds: 3600 },
  // `all` is the absence of a window, not a very large one: the backend takes
  // `null` and hands back everything the container still has.
  { value: "all", seconds: undefined },
];
const SINCE_OPTIONS = SINCE.map((s) => ({ value: s.value, label: s.value }));

/**
 * How many trailing lines a connect asks for.
 *
 * A ceiling rather than a preference. `since=all` on a container that has been
 * up for a week would otherwise open with its entire history, all of which but
 * the last few thousand lines is immediately dropped by the ring anyway — so
 * the only thing an unbounded first connect buys is the wait.
 */
const TAIL_LINES = 1000;

/** Stands for "every container", as a select value that cannot be a name. */
const ALL_CONTAINERS = "";

/** How near the bottom still counts as being at it, in pixels. Classic's 48. */
const STICK_SLACK = 48;

/**
 * The one state this readout can be in that the CONNECTION is not.
 *
 * Everything else comes from core's `logConnectionStatus` — the four
 * connection states, each with the word and the tone core decided for it. This
 * screen writes no table of its own; it only names the state core has no
 * opinion about, because pausing is a fact about the VIEW rather than about
 * the stream, which goes on connecting, dropping and reconnecting underneath a
 * held pane. Shaped as a `LogConnectionVerdict` and toned through
 * `statusTone`, so even this one word takes its colour from core's severity
 * vocabulary rather than picking one.
 */
const PAUSED: LogConnectionVerdict = { label: "Paused", health: "neutral" };

/**
 * What the stream is doing, in a word.
 *
 * The design shows this readout only while following, so a stream that has
 * dropped says nothing at all and a reader watching a failure believes they
 * are still watching it. It is always on here, and paused is one of the states
 * it can report rather than the absence of them.
 */
function connectionSignal(
  status: LogStreamStatus,
  paused: boolean,
): LogConnectionVerdict {
  return paused ? PAUSED : logConnectionStatus(status);
}

/**
 * The connection's word and its denominator, which are one string because they
 * must never be separated.
 *
 * The aggregate flips to `reconnecting` the moment ANY single target does.
 * That is the right answer to the question the indicator exists for — *am I
 * seeing everything?* — and a badly misleading one on its own: on a ten-pod
 * workload one blip would read as a total outage. And it runs the other way
 * too, because `status` stays `connecting` until every target has reported
 * once: on a wide fan-out lines can already be scrolling while the word still
 * says connecting. The counts are what make both honest, so the word does not
 * get to appear without them.
 *
 * `Paused` carries them as well. Pausing freezes the pane, not the stream, and
 * a reader who paused and then lost half the fan-out has to be able to see it.
 *
 * `streaming` rather than `following` for the count, though `Following` is the
 * aggregate's own word: the two halves say different things — what the stream
 * as a whole is doing, and how many of its targets are actually delivering —
 * and "Following — 4 of 4 following" reads as one fact said twice.
 */
function connectionLabel(
  verdict: LogConnectionVerdict,
  live: number,
  total: number,
): string {
  return `${verdict.label} — ${live} of ${total} streaming`;
}

/**
 * Save `content` to `filename`: through the native save dialog in the desktop
 * shell, and as a browser download in web mode.
 *
 * Both halves are needed, and neither works in the other's place — a Tauri
 * webview does not prompt on `<a download>`, and a browser has no
 * `save_text_file` command to invoke. Classic reached the same conclusion
 * (`apps/desktop/src/components/LogsView.tsx`); this is that decision written
 * where the new screen can use it, not a second policy.
 */
async function saveOrDownload(
  filename: string,
  content: string,
): Promise<void> {
  if (isTauri()) {
    await saveTextFile(filename, content);
    return;
  }
  const url = URL.createObjectURL(new Blob([content], { type: "text/plain" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}

/**
 * `2026-08-24T14:07:41.208123456Z ` — what the backend prefixes each line with
 * when a stream is opened with `timestamps: true`, which this screen always is
 * because the design gives the time its own column.
 */
const RFC3339 = /^\d{4}-\d{2}-\d{2}T(\d{2}:\d{2}:\d{2})(\.\d+)?(?:Z|[+-]\d{2}:?\d{2})? ?/;

/** One line as the body draws it: the four columns, plus what a filter reads. */
interface Row {
  /** `14:07:41.208`, or `""` for a line that arrived without a stamp. */
  ts: string;
  /** Which pod wrote the line. DATA — read off the target, not the label. */
  pod: string;
  /**
   * Which container wrote it. DATA, and the reason it does not come out of
   * the line's `source` string: that string is the target's **display**
   * label, and `resolveLogSubject` drops the container from it whenever every
   * target in scope shares one container name — the ordinary Deployment with
   * three replicas of one container. Recovering the container by splitting
   * the label there yields `""` for every line, while the container select is
   * populated from the targets, which carry the real name: the reader picks
   * their container and the screen goes blank. The label is a decision about
   * what to SHOW; this is what the line IS, and they do not travel in one
   * string.
   */
  container: string;
  /**
   * Whether the source gutter names the container as well as the pod — the
   * display half of the same fact, taken from the shape of the label rather
   * than recomputed here. A stream whose targets all run one container says
   * the pod alone; repeating that one word down a 200px gutter is noise.
   */
  namesContainer: boolean;
  /**
   * The level word AS THE LINE SPELLS IT — `error`, `WARNING`, `warn`,
   * `debug` — from {@link logLineLevel}, or `""` when the line carries none.
   *
   * This is what the level column prints, and it is deliberately not
   * {@link Row.health}: the severity vocabulary's words (`danger`,
   * `warning`) appear nowhere in the log being described, so a column showing
   * them reports something the reader cannot grep for — and `WARNING` does not
   * fit the kit's 44px gutter either. The kit turns the word into a colour
   * through its own `LEVEL_TONE`, which is why no `tone` is passed alongside
   * it: a call site that maps "error" to red itself is how the same word ends
   * up red on one screen and grey on the next.
   */
  level: string;
  /**
   * Core's severity for the line, from {@link logLineHealth} — a text-scan
   * heuristic over the same one rule the level word comes from. Not drawn;
   * it is here so the filter can match `danger` and `warning` as well as the
   * literal word the line used.
   */
  health: HealthKind;
  message: string;
  /** The line exactly as it arrived, stamp and all — what an export writes. */
  raw: string;
  /** Lower-cased message + severity word, which is what the filter matches. */
  haystack: string;
}

/** The line's origin as one string: `api-7 · api` on screen, `api-7/api` in a
 *  file, and the pod alone for a stream with no container to disambiguate. */
function sourceOf(row: Row, separator: string): string {
  return row.namesContainer && row.container !== ""
    ? `${row.pod}${separator}${row.container}`
    : row.pod;
}

/**
 * Every target under the label its lines arrive tagged with — the index
 * {@link toRow} recovers a line's pod and container through.
 *
 * The labels are unique by construction: `resolveLogSubject` emits `pod` alone
 * only when one container name is shared across the whole scope (so one target
 * per pod), `pod/container` when they differ, and `""` only when there is
 * exactly one target altogether — which is why the empty key resolves that
 * lone target rather than standing for "unknown".
 */
function indexTargets(targets: readonly LogTarget[]): ReadonlyMap<string, LogTarget> {
  return new Map(targets.map((t) => [t.label ?? "", t]));
}

/**
 * Split a stream line into the columns the design draws.
 *
 * The line's `source` is its target's **display label**, so the pod and the
 * container are looked up through {@link indexTargets} rather than parsed back
 * out of it — see {@link Row.container} for what parsing it costs. Splitting on
 * `/` survives only as the fallback for a source no target claims, which is a
 * line the screen did not ask for; naming it from its own string beats drawing
 * it nameless.
 */
function toRow(line: StreamLine, byLabel: ReadonlyMap<string, LogTarget>): Row {
  const stamp = line.text.match(RFC3339);
  const ts = stamp ? `${stamp[1]}${(stamp[2] ?? "").slice(0, 4)}` : "";
  const message = stamp ? line.text.slice(stamp[0].length) : line.text;
  const slash = line.source.indexOf("/");
  const target = byLabel.get(line.source);
  const pod =
    target !== undefined
      ? target.pod
      : slash < 0
        ? line.source
        : line.source.slice(0, slash);
  const container =
    target !== undefined
      ? (target.container ?? "")
      : slash < 0
        ? ""
        : line.source.slice(slash + 1);
  const health = logLineHealth(message);
  return {
    ts,
    pod,
    container,
    // An unlabelled line is the single-target stream, whose gutter would
    // otherwise be blank; a labelled one shows the container exactly when its
    // label carried it.
    namesContainer: line.source === "" || slash >= 0,
    level: logLineLevel(message) ?? "",
    health,
    message,
    raw: line.text,
    haystack: `${message} ${health}`.toLowerCase(),
  };
}

/** `3 pods`, `1 pod` — counted over distinct pods, not over targets: a pod
 *  running three containers is three targets and one pod. */
function podCount(targets: readonly LogTarget[]): string {
  const pods = new Set(targets.map((t) => t.pod)).size;
  return `${pods} ${pods === 1 ? "pod" : "pods"}`;
}

/**
 * Why the follow control is off while a previous instance is on screen.
 *
 * Not a UI convention with an exception carved out for it: `podLogs` is a
 * one-shot fetch and the streaming path passes `previous: false` as a literal,
 * so there is genuinely nothing to follow. A control that stops working
 * without saying so reads as a bug, and the sentence is one string because it
 * is said twice on purpose — in the disabled control's own name, for a reader
 * who lands on it, and in the banner, for a reader looking at the pane.
 */
const NO_FOLLOWING = "A terminated instance cannot be streamed";

/**
 * The previous instance's buffer, and how the fetch that produced it went.
 *
 * `idle` and `loading` are distinct from `ready` with no lines, because "still
 * fetching" and "the crashed instance said nothing" are different facts about
 * a pane that looks identical.
 */
type Snapshot =
  | { status: "idle" }
  | { status: "loading" }
  | {
      status: "ready";
      lines: StreamLine[];
      /** The first target whose buffer could not be read, if any: a partial
       *  read must say what is missing — an absent container's lines look
       *  exactly like a quiet container's. */
      failure?: { where: string; error: string };
    };

/** `api-7 · api` — a target as this screen names one. */
function whereOf(instance: PreviousInstance): string {
  return `${instance.pod} · ${instance.container}`;
}

/**
 * One terminated instance in a sentence: where it ran, when it ended and how.
 *
 * The whole date rather than the design's bare `14:07:42`, through core's own
 * `absoluteTimestamp`: a container that died yesterday afternoon is not
 * "14:07:42", and a second clock in this screen is how two surfaces start
 * disagreeing about the same instant. Each fact is dropped rather than faked
 * when the cluster did not send it.
 */
function terminationLine(instance: PreviousInstance): string {
  const when = absoluteTimestamp(instance.finishedAt);
  const exit =
    instance.exitCode === undefined
      ? instance.reason
      : instance.reason === undefined
        ? `exit ${instance.exitCode}`
        : // The code alone does not separate an OOM kill from a SIGKILL, and
          // the reason alone does not survive a `grep` for the code.
          `exit ${instance.exitCode} (${instance.reason})`;
  const facts = [...(when === "" ? [] : [`terminated ${when}`]), ...(exit === undefined ? [] : [exit])];
  return facts.length === 0 ? whereOf(instance) : `${whereOf(instance)} — ${facts.join(", ")}`;
}

/** The banner's headline. One corpse is named in full; several are counted,
 *  because six of these sentences in a strip is not a headline. */
function previousHeadline(instances: readonly PreviousInstance[]): string {
  return instances.length === 1
    ? `Reading the previous instance of ${terminationLine(instances[0])}`
    : `Reading the previous instance of ${instances.length} containers`;
}

/** The screen's frame, shared by every state it can be in. */
function LogsScreen({
  eyebrow,
  actions,
  children,
}: {
  eyebrow: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Screen title="Logs" eyebrow={eyebrow} fill actions={actions}>
      {children}
    </Screen>
  );
}

/** The question the header's ask control sends, named because the control
 *  now states it in its accessible name as well as sending it. */
const SUMMARISE_QUESTION = "Summarise the last 500 log lines and group errors by cause";

export function Logs({ route }: { route: string }) {
  const cluster = useActiveContext();
  const parts = parseLogsRoute(route);

  if (!cluster) return <NoClusterScreen title="Logs" noun="logs" />;

  if (!parts) {
    // A bare `/logs`. The doors that carry a subject are the row menu's
    // *Follow logs* and the detail screen's *Logs*; this is the one someone
    // opens by accident, and it names what it needs instead of streaming
    // from nowhere — with whatever this reader has already followed as the
    // way in, since "open one from Workloads" is an instruction, not a door.
    return (
      <LogsScreen eyebrow={cluster.name}>
        <EmptyState
          title="Pick a workload or a pod to follow"
          hint="srelens tails every container behind a Deployment, StatefulSet, DaemonSet or Job — or one pod on its own. Open one from Workloads and choose Follow logs."
          action={
            // Keyed on the cluster: the recents are that cluster's, and so is
            // every answer already collected about them.
            <RecentSubjects
              key={cluster.stableId}
              context={cluster.name}
              clusterId={cluster.stableId}
            />
          }
          className="flex-1"
        />
      </LogsScreen>
    );
  }

  return (
    // Keyed on the subject: a different workload is a different stream, and
    // remounting is how its buffer, filters and scroll position start clean
    // rather than inheriting the last one's.
    <LogsSubject
      key={`${cluster.stableId}:${route}`}
      context={cluster.name}
      clusterId={cluster.stableId}
      clusterName={cluster.name}
      {...parts}
    />
  );
}

/**
 * The recently-followed subjects, offered as a way in — and checked before
 * they are.
 *
 * **Every entry is verified against the cluster before it is offered.** A list
 * of dead pods that error when clicked is worse than no list: the reader
 * trusted it. One `listResource` per kind and namespace answers for every
 * remembered subject in it, so the check costs a call per pair rather than one
 * per name, and `reviewRecents` decides what each answer means — including the
 * asymmetry between a pod, whose exact name never comes back, and a workload,
 * whose name outlives its pods.
 */
function RecentSubjects({ context, clusterId }: { context: string; clusterId: string }) {
  const entries = useRecentLogSubjects(clusterId);
  const [scans, setScans] = useState<ReadonlyMap<string, SubjectScan>>(
    () => new Map(),
  );
  /**
   * Which lists have already been asked for. A ref, not state: it exists to
   * stop the effect below asking twice, and putting it in `scans` would mean
   * depending on `scans` in the effect that sets it — which is a fetch per
   * answer, forever.
   */
  const asked = useRef(new Set<string>());

  // A string, because the dependency is the SET of lists to fetch: `entries`
  // changes identity whenever any subject is remembered or forgotten, and
  // re-running on that would re-ask for lists already in hand.
  const scanKeys = useMemo(
    () => [...new Set(entries.map(scanKey))].sort().join("\n"),
    [entries],
  );

  useEffect(() => {
    let alive = true;
    for (const key of scanKeys === "" ? [] : scanKeys.split("\n")) {
      if (asked.current.has(key)) continue;
      asked.current.add(key);
      const [kind, namespace] = key.split("\u0000");
      // `listResource` reports failure by returning `{ error }` rather than
      // throwing, so this reads the field — and keeps the failure AS a
      // failure: see `SubjectScan` for why an empty list is not the same
      // answer as an unanswered one.
      void listResource(context, kind, namespace).then((out) => {
        if (!alive) return;
        const scan: SubjectScan =
          out.error !== undefined
            ? { error: true }
            : { names: (out.items ?? []).map((item) => item.name) };
        setScans((current) => new Map(current).set(key, scan));
      });
    }
    return () => {
      alive = false;
    };
  }, [context, scanKeys]);

  const { offered, forget } = useMemo(
    () => reviewRecents(entries, scans),
    [entries, scans],
  );

  // A pod the cluster has replaced is not coming back under that name, and
  // leaving it in would push a live workload off the end of the cap. Settles
  // in one pass: the forgotten entries leave `entries`, and the next review
  // has nothing left to forget.
  useEffect(() => {
    forgetLogSubjects(forget.map(recentKey));
  }, [forget]);

  if (offered.length === 0) return null;

  return (
    <div
      data-slot="recents"
      className="flex flex-col items-stretch gap-1 text-left"
    >
      <Eyebrow>recently followed</Eyebrow>
      {offered.map(({ entry, presence }) => (
        <Button
          key={recentKey(entry)}
          variant="secondary"
          size="sm"
          // Not a way in, and not pretending to be one. Shown rather than
          // hidden for the same reason the stale-namespace banner is shown:
          // silence reads as "you never followed this", when the true story
          // is "what you followed is not there just now".
          disabled={presence === "gone"}
          onClick={() =>
            openTab(logsRoute(entry.kind, entry.namespace, entry.name), {
              clusterName: context,
            })
          }
          className="justify-between gap-3"
        >
          <span>{`${entry.kind}/${entry.name}`}</span>
          <span className="path">
            {presence === "gone" ? "no longer on this cluster" : entry.namespace}
          </span>
        </Button>
      ))}
    </div>
  );
}

/**
 * Resolving the route's subject to the pods and containers to follow, and
 * saying why it could not.
 *
 * The stream is not opened until every in-scope pod's containers are known —
 * `resolveLogSubject` is all-or-nothing for exactly this reason — so the
 * streaming half of the screen is a separate component that only mounts once
 * there are targets. That also means a `since` change cannot re-resolve the
 * subject: the lookup lives above the state that drives the stream.
 */
function LogsSubject({
  context,
  clusterId,
  clusterName,
  kind,
  namespace,
  name,
}: LogsRouteParts & { context: string; clusterId: string; clusterName: string }) {
  const [attempt, setAttempt] = useState(0);
  const [resolution, setResolution] = useState<LogSubjectResolution | null>(
    null,
  );

  const subject = useMemo<LogSubject>(
    () =>
      kind.toLowerCase() === "pod"
        ? { type: "pod", context, namespace, name }
        : { type: "workload", context, namespace, kind, name },
    [context, namespace, kind, name],
  );

  useEffect(() => {
    let alive = true;
    setResolution(null);
    void resolveLogSubject(subject).then((next) => {
      if (alive) setResolution(next);
    });
    return () => {
      alive = false;
    };
  }, [subject, attempt]);

  /**
   * Remembered once it has actually streamed, not once it was asked for.
   *
   * `resolved` is the only state that carries targets, and a subject nobody
   * could open is not a way in — offering back the route that just failed is
   * the exact trap the recents exist to avoid. Keyed by the cluster's
   * `stableId` rather than its name, so a context renamed in the kubeconfig
   * keeps what was followed on it.
   */
  useEffect(() => {
    if (resolution?.status !== "resolved") return;
    rememberLogSubject({ cluster: clusterId, kind, namespace, name });
  }, [resolution, clusterId, kind, namespace, name]);

  const head = `${clusterName} / ${namespace} / ${name}`;

  if (resolution === null) {
    return (
      <LogsScreen eyebrow={head}>
        <LoadingState label="Finding the containers to follow" />
      </LogsScreen>
    );
  }

  if (resolution.status === "error") {
    return (
      <LogsScreen eyebrow={head}>
        {/* The classification's own `raw`, not the FriendlyError: `describeError`
            is idempotent over it, so this goes through the one error path this
            package has rather than opening a second. */}
        <FailureState
          title={`Could not open logs for ${name}`}
          error={resolution.error.raw}
          onRetry={() => setAttempt((a) => a + 1)}
          className="m-3"
        />
      </LogsScreen>
    );
  }

  if (resolution.status === "empty") {
    return (
      <LogsScreen eyebrow={head}>
        <EmptyState
          title="Nothing to follow"
          hint={resolution.detail}
          action={
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setAttempt((a) => a + 1)}
            >
              Look again
            </Button>
          }
          className="flex-1"
        />
      </LogsScreen>
    );
  }

  return (
    <LogsStream
      context={context}
      clusterName={clusterName}
      namespace={namespace}
      name={name}
      targets={resolution.targets}
      // Fetched with the targets, not for the rail: the pod objects were
      // already on the wire here for their containers, and their status and
      // labels came back on the same round trip.
      pods={resolution.pods}
      // Same round trip again: `lastState.terminated` is on those objects, so
      // the screen knows which containers have a corpse to read before the
      // reader asks for one.
      terminated={resolution.previous}
    />
  );
}

function LogsStream({
  context,
  clusterName,
  namespace,
  name,
  targets,
  pods,
  terminated,
}: {
  context: string;
  clusterName: string;
  namespace: string;
  name: string;
  targets: LogTarget[];
  pods: LogSubjectPod[];
  terminated: PreviousInstance[];
}) {
  const { ask } = useConsole();
  const [text, setText] = useState("");
  // `all`, as classic has always defaulted, and not the design's drawn `5m`.
  // A mock is drawn against a fixture that never stops talking; a real
  // workload that has been quiet for six minutes renders "Nothing has been
  // logged yet" under a 5m window — true, useless, and indistinguishable from
  // a broken screen. The tail cap already bounds what arrives, so the age
  // window costs the reader their logs and buys nothing.
  const [since, setSince] = useState("all");
  const [container, setContainer] = useState(ALL_CONTAINERS);
  const [wrap, setWrap] = useState(false);
  const [metrics, setMetrics] = useState({
    scrollTop: 0,
    viewportHeight: 0,
    rowHeight: 0,
  });
  /** The pods whose boxes the reader has unticked. Names, not indices: the
   *  rail is rebuilt on every render and an index means nothing across one. */
  const [hidden, setHidden] = useState<readonly string[]>([]);
  /** Which restart the reader has already been told about. */
  const [seenRestart, setSeenRestart] = useState(0);
  /** Whether the pane is showing the instance that died instead of the live one. */
  const [previous, setPrevious] = useState(false);
  /** That instance's buffer. A one-shot fetch, not a subscription. */
  const [snapshot, setSnapshot] = useState<Snapshot>({ status: "idle" });
  /** Why the last export did not land, if it did not. */
  const [saveError, setSaveError] = useState<unknown>(undefined);

  const viewportRef = useRef<HTMLDivElement>(null);
  const rowsRef = useRef<HTMLDivElement>(null);
  /**
   * Whether new lines should move the view. Held in a ref, not state: it is
   * written from a scroll handler that fires far more often than the screen
   * renders, and read by a layout effect that must see the value from the
   * scroll that just happened, not from the render before it.
   */
  const stickRef = useRef(true);

  const sinceSeconds = SINCE.find((s) => s.value === since)?.seconds;
  const stream = useLogStream(context, namespace, targets, {
    // Always on: the design gives the time its own column, so the stamp is not
    // an option the reader turns on — it is where the first column comes from.
    timestamps: true,
    sinceSeconds,
    tailLines: TAIL_LINES,
  });

  const byLabel = useMemo(() => indexTargets(targets), [targets]);
  const liveRows = useMemo(
    () => stream.lines.map((l) => toRow(l, byLabel)),
    [stream.lines, byLabel],
  );

  /**
   * Fetch the terminated containers' buffers.
   *
   * One `podLogs` per corpse, in parallel, and nothing at all when there is no
   * corpse — asking for the previous instance of a container that has never
   * died gets a refusal from the API, and a refusal is not what "this pod has
   * not restarted" should look like.
   *
   * **`sinceSeconds` is deliberately not passed.** It is the window the reader
   * chose for the LIVE tail, and the instance being read died before it
   * started: a five-minute window over a container that was killed twenty
   * minutes ago hands back an empty file and calls it the crash logs.
   * `timestamps` IS passed, because the time column is drawn from the stamp
   * here exactly as it is for a live line.
   *
   * `podLogs` reports failure by returning `{ error }` rather than throwing,
   * so this reads the field; the `Promise.all` always settles.
   */
  useEffect(() => {
    if (!previous || terminated.length === 0) return;
    let alive = true;
    setSnapshot({ status: "loading" });
    void Promise.all(
      terminated.map(async (instance) => ({
        instance,
        out: await podLogs(context, namespace, instance.pod, undefined, {
          container: instance.container,
          previous: true,
          timestamps: true,
          tailLines: TAIL_LINES,
        }),
      })),
    ).then((results) => {
      if (!alive) return;
      const lines: StreamLine[] = [];
      let failure: { where: string; error: string } | undefined;
      for (const { instance, out } of results) {
        if (out.error !== undefined) {
          // Kept, not swallowed. One container of a crash-looping pod whose
          // buffer silently never appears is indistinguishable from one that
          // died without saying anything.
          failure ??= { where: whereOf(instance), error: out.error };
          continue;
        }
        // The SAME `label` the live stream tags this target's lines with, so
        // `toRow` splits the source identically and a previous line is drawn,
        // filtered, tallied and exported by exactly one set of rules.
        const source =
          targets.find(
            (t) => t.pod === instance.pod && t.container === instance.container,
          )?.label ?? "";
        for (const text of (out.logs ?? "").split("\n")) {
          // A blob ends on a newline; a trailing empty row is not a log line.
          if (text !== "") lines.push({ source, text });
        }
      }
      setSnapshot({
        status: "ready",
        lines,
        ...(failure === undefined ? {} : { failure }),
      });
    });
    return () => {
      alive = false;
    };
  }, [previous, terminated, context, namespace, targets]);

  const previousRows = useMemo(
    () => (snapshot.status === "ready" ? snapshot.lines.map((l) => toRow(l, byLabel)) : []),
    [snapshot, byLabel],
  );

  /**
   * What the pane is drawing. The two buffers are never merged: they are
   * different instances of the same container, and interleaving them would
   * invent a history that never happened.
   */
  const rows = previous ? previousRows : liveRows;

  const containers = useMemo(
    () =>
      [
        ...new Set(
          targets.map((t) => t.container ?? "").filter((c) => c !== ""),
        ),
      ].sort(),
    [targets],
  );

  const query = text.trim().toLowerCase();
  const filtered = useMemo(
    () =>
      rows.filter((row) => {
        if (hidden.includes(row.pod)) return false;
        if (container !== ALL_CONTAINERS && row.container !== container)
          return false;
        if (query !== "" && !row.haystack.includes(query)) return false;
        return true;
      }),
    [rows, hidden, container, query],
  );

  const togglePod = useCallback((pod: string, checked: boolean) => {
    setHidden((current) =>
      checked
        ? current.filter((p) => p !== pod)
        : current.includes(pod)
          ? current
          : [...current, pod],
    );
  }, []);

  /** Puts every filter back, the pod boxes among them — otherwise the state
   *  that reports "no lines match" cannot undo the tick that caused it. */
  const clearFilters = useCallback(() => {
    setText("");
    setContainer(ALL_CONTAINERS);
    setHidden([]);
  }, []);

  const railPods = useMemo<StreamPod[]>(
    () =>
      pods.map((pod) => ({
        name: pod.name,
        // The label is the screen's copy; the hash is the fact. Spread rather
        // than `revision: undefined`, so an absent label stays absent instead
        // of arriving as a present-but-empty figure.
        ...(pod.revision === undefined
          ? {}
          : { revision: `rev ${pod.revision}` }),
        checked: !hidden.includes(pod.name),
        // core's verdict, decided once in `resolveLogSubject`. The rail is
        // told; it does not work this out, and neither does this screen.
        tone: pod.health,
      })),
    [pods, hidden],
  );

  /**
   * What the rail tallies: THE LINES THE BODY IS DRAWING, with every filter
   * applied — and each one's RFC3339 stamp already off.
   *
   * The stamp is the trap. A stamped line opens with a digit, `tallyLogTerms`
   * reads a leading digit as a value and ends the term run there, so a stamped
   * buffer tallies to NOTHING — no error, no warning, just an empty rail that
   * looks exactly like a quiet log. `row.message` is the stripped text; the
   * raw line never goes near this.
   */
  const railLines = useMemo<StreamLine[]>(
    () => filtered.map((row) => ({ source: row.pod, text: row.message })),
    [filtered],
  );

  /**
   * Export what is ON SCREEN.
   *
   * `filtered`, not the buffer: the reader narrowed to the thing they are
   * looking at, and a file that quietly contains the nine thousand lines they
   * filtered out is not the thing they asked for. The whole of `filtered`
   * rather than the drawn window, though — the window is an artefact of how
   * far they happen to have scrolled, not of what they chose to look at.
   *
   * Each line goes out RAW, stamp and all, in classic's `source | line`
   * shape. The column on screen truncates the stamp to `14:07:41.208` because
   * a reader watching a live tail knows what day it is; a file read tomorrow
   * does not, and the full RFC3339 is what makes an exported line greppable
   * against anything else. Which lines are exported is what "on screen"
   * governs — not how much of each one an 86px gutter had room for.
   *
   * While paused, `lines` is the frozen view, so this exports exactly the
   * pane the reader paused. That is the point: someone who pauses on a
   * failure and exports means "this, the thing I stopped on", not "this plus
   * the nine hundred lines that arrived while I was reading it".
   */
  function exportView() {
    setSaveError(undefined);
    const content = filtered
      .map((row) => `${sourceOf(row, "/")} | ${row.raw}`)
      .join("\n");
    // The previous instance's buffer goes out under its own name. Two files of
    // the same workload's logs, one of them from an instance that no longer
    // exists, are not tellable apart afterwards if they are called the same
    // thing.
    void saveOrDownload(
      `${name}${previous ? "-previous" : ""}.log`,
      content,
    ).catch((e: unknown) => setSaveError(e));
  }

  // Sample the viewport and one row so the render can window the list. Both
  // degrade to 0 in jsdom, which `computeLogWindow` reads as "render
  // everything" — one of the bail-outs that make it correct.
  const measure = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const firstRow = rowsRef.current?.querySelector<HTMLElement>(".logline");
    const rowHeight = firstRow ? firstRow.getBoundingClientRect().height : 0;
    setMetrics((m) =>
      m.scrollTop === viewport.scrollTop &&
      m.viewportHeight === viewport.clientHeight &&
      m.rowHeight === rowHeight
        ? m
        : {
            scrollTop: viewport.scrollTop,
            viewportHeight: viewport.clientHeight,
            rowHeight,
          },
    );
  }, []);

  useLayoutEffect(() => {
    measure();
    const viewport = viewportRef.current;
    if (!viewport || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [measure, filtered.length, wrap]);

  /**
   * Stick to bottom, and the reason it is a threshold rather than a flag the
   * reader sets: following is the default, and the moment someone scrolls up
   * is the moment they are reading the line that made them open the screen.
   * Yanking them back to the newest line there loses exactly the thing they
   * were looking at, and no amount of "scroll down to resume" copy makes that
   * acceptable — so arriving lines move the view only while the view is
   * already at the end of the buffer.
   */
  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !stickRef.current) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [filtered, wrap]);

  function trackScroll() {
    const viewport = viewportRef.current;
    if (!viewport) return;
    stickRef.current =
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <
      STICK_SLACK;
    measure();
  }

  const signal = connectionSignal(stream.status, stream.paused);
  /** Whether new lines are arriving in THIS pane. A snapshot is not followed,
   *  however healthy the connection underneath it is. */
  const following = !previous && !stream.paused;
  const restarted = stream.restartCount > seenRestart;
  const window_ = computeLogWindow({
    total: filtered.length,
    scrollTop: metrics.scrollTop,
    viewportHeight: metrics.viewportHeight,
    rowHeight: metrics.rowHeight,
    wrap,
  });
  const drawn = window_.virtualized
    ? filtered.slice(window_.start, window_.end)
    : filtered;
  const windowLabel = since === "all" ? "" : ` in the last ${since}`;

  /** A corpse that refused, while others answered — a banner over lines that
   *  are still there, rather than a card in place of them. */
  const snapshotFailure =
    previous && snapshot.status === "ready" ? snapshot.failure : undefined;
  const partialFailure = rows.length > 0 ? snapshotFailure : undefined;

  /**
   * What the body says INSTEAD of lines while a previous instance is asked
   * for. `null` means the ordinary body draws.
   *
   * A pod on its first run has no corpse, and that is the common case rather
   * than an edge: an empty body there would read as "the crashed instance said
   * nothing", which is a different and far more alarming fact than "nothing
   * has crashed". Each state below is a fact the pane cannot tell apart on its
   * own — still fetching, nothing to fetch, fetched and empty, refused.
   */
  function previousBody(): ReactNode {
    if (!previous) return null;
    if (terminated.length === 0) {
      return (
        <EmptyState
          title="No previous instance"
          hint={`srelens is following ${targets.length} container${targets.length === 1 ? "" : "s"} across ${podCount(targets)}, and none of them has terminated and been restarted. There is a previous instance to read only once a container has died at least once.`}
        />
      );
    }
    if (snapshot.status !== "ready") {
      return <LoadingState label="Reading the previous instance" />;
    }
    if (rows.length > 0) return null;
    if (snapshotFailure !== undefined) {
      return (
        <FailureState
          title={`Could not read the previous instance of ${snapshotFailure.where}`}
          error={snapshotFailure.error}
          className="m-3"
        />
      );
    }
    return (
      <EmptyState
        title="The previous instance logged nothing"
        hint="srelens read the terminated container's buffer and it was empty: the instance died before writing a line, or the kubelet has already rotated it away."
      />
    );
  }
  const previousNotice = previousBody();

  return (
    <LogsScreen
      eyebrow={`${clusterName} / ${namespace} / ${name} · ${podCount(targets)}`}
      actions={
        <>
          {/* A `Button`, not the row's `AskChip`, for the reason `Events.tsx`
              and `Overview.tsx` both give: `.row-ask` is `opacity: 0` until a
              `.tbl tbody tr` is hovered, which is right for one of forty rows
              and invisible in a header, where there is no row to hover. This
              header advertised an action nobody could see from #344 until it
              was measured on a real screen. */}
          <Button
            type="button"
            size="sm"
            aria-label={`Summarise this stream: ${SUMMARISE_QUESTION}`}
            onClick={() => ask(SUMMARISE_QUESTION)}
          >
            Summarise this stream
          </Button>
          {/* Disabled while a previous instance is on screen, and it says why
              in its own accessible name: `previous` forbids following because
              the API cannot follow a terminated container, not because this
              screen would rather not. The visible word stays inside the name,
              so the two never disagree. */}
          <Button
            variant="secondary"
            size="sm"
            onClick={stream.togglePause}
            disabled={previous}
            aria-label={previous ? `Follow — ${NO_FOLLOWING.toLowerCase()}` : undefined}
          >
            {following ? (
              <Icons.pause size={13} aria-hidden="true" />
            ) : (
              <Icons.play size={13} aria-hidden="true" />
            )}
            {following ? "Pause" : "Follow"}
          </Button>
          {/* Disabled on an empty view rather than hidden: a control that
              vanishes leaves the reader wondering whether this screen exports
              at all, where a greyed one says "yes, once there is something to
              export". Classic disabled it on the same condition. */}
          <Button
            variant="secondary"
            size="sm"
            onClick={exportView}
            disabled={filtered.length === 0}
          >
            Export
          </Button>
        </>
      }
    >
      <SideRail
        head="Stream"
        width={STREAM_RAIL_WIDTH}
        rail={
          <StreamRail
            pods={railPods}
            lines={railLines}
            onTogglePod={togglePod}
          />
        }
      >
        <FilterBar
          value={text}
          onValueChange={setText}
          label="Filter lines"
          placeholder="Filter lines"
        >
          <div className="flex items-center gap-1.5">
            <Eyebrow>since</Eyebrow>
            <Select
              value={since}
              onValueChange={setSince}
              options={SINCE_OPTIONS}
              // The window narrows the LIVE tail, and there is no live tail on
              // screen while the terminated instance's snapshot is: that buffer
              // arrived whole, in one fetch, and no time window applies to it.
              // The value is kept rather than cleared — it is the window the
              // reader chose for the stream they will go back to.
              disabled={previous}
              aria-label="since"
            />
          </div>
          {containers.length > 0 && (
            <div className="flex items-center gap-1.5">
              <Eyebrow>container</Eyebrow>
              <Select
                value={container}
                onValueChange={setContainer}
                options={[
                  { value: ALL_CONTAINERS, label: "all" },
                  ...containers.map((c) => ({ value: c })),
                ]}
                aria-label="container"
              />
            </div>
          )}
          {/* The design's rotate-ccw toggle. Warn-tinted while on, from the
              same tokens the banner under it uses — a pane showing something
              other than the live stream must not look like one that is. */}
          <Button
            variant="secondary"
            size="xs"
            aria-pressed={previous}
            onClick={() => setPrevious((p) => !p)}
            style={
              previous
                ? {
                    borderColor: toneColor("warn"),
                    color: toneColor("warn"),
                    background: toneWash("warn"),
                  }
                : undefined
            }
          >
            <Icons.revert size={12} aria-hidden="true" />
            Previous instance
          </Button>
          <Button
            variant="secondary"
            size="xs"
            aria-pressed={wrap}
            onClick={() => setWrap((w) => !w)}
            // The design draws every message `break-all`, which makes every row a
            // different height and defeats the windowing outright — 5,000 wrapped
            // rows in the DOM, re-rendered on every line that arrives. Unwrapped
            // is the default so the arithmetic holds; the toggle is here because
            // without it a line wider than the pane has no way to be read at all,
            // and the design offers none.
          >
            Wrap
          </Button>
          <span className="flex-1" />
          {/* Both of these speak for the live stream, and neither is true of a
              snapshot: a readout saying `Following — 4 of 4 streaming` over a
              buffer from an instance that no longer exists is the one thing
              this banner is here to prevent. */}
          {!previous && stream.paused && stream.pendingWhilePaused > 0 && (
            <Eyebrow>
              {groupNumber(stream.pendingWhilePaused)} new lines
            </Eyebrow>
          )}
          {!previous && (
            <LiveSignal
              label={connectionLabel(
                signal,
                stream.liveTargets,
                stream.totalTargets,
              )}
              tone={statusTone(signal.health)}
            />
          )}
        </FilterBar>

        {previous && terminated.length > 0 && (
          // Directly under the filter bar, as the design places it: what is
          // being read, and why nothing is arriving.
          <Alert
            tone="warn"
            title={previousHeadline(terminated)}
            className="mx-3 mt-3"
          >
            {terminated.length > 1 &&
              terminated.map((instance) => (
                <div key={`${instance.pod}/${instance.container}`}>
                  {terminationLine(instance)}
                </div>
              ))}
            {NO_FOLLOWING}: the cluster hands a terminated container's buffer
            back once, so this is a snapshot and srelens is not following it.
          </Alert>
        )}

        {previous && partialFailure !== undefined && (
          // Some of the corpses answered and one did not. The lines that did
          // arrive stay on screen — throwing them away because a second
          // container refused would lose the evidence the reader came for.
          <FailureAlert
            title={`Could not read the previous instance of ${partialFailure.where}`}
            error={partialFailure.error}
            className="mx-3 mt-3"
          />
        )}

        {restarted && (
          // A `since` change reopens the stream, and the reader loses every line
          // they had scrolled through. The hook counts those restarts precisely
          // so this is sayable; without it the pane simply empties and a reader
          // who had found the line they came for believes the log did.
          <Alert
            tone="warn"
            title="Scrollback cleared"
            onDismiss={() => setSeenRestart(stream.restartCount)}
            dismissLabel="Dismiss the scrollback notice"
            className="mx-3 mt-3"
          >
            Changing the window reopens the stream, and the lines it had already
            delivered are not sent again. srelens is following from here.
          </Alert>
        )}

        {saveError !== undefined && (
          <FailureAlert
            title="Could not save this stream"
            error={saveError}
            className="mx-3 mt-3"
          />
        )}

        <div
          ref={viewportRef}
          onScroll={trackScroll}
          role="log"
          aria-label={`${name} logs`}
          className={`scroll min-h-0 flex-1 py-1 font-mono text-[0.75rem] leading-[1.85] ${
            wrap ? "" : "whitespace-nowrap"
          }`}
        >
          {previousNotice !== null ? (
            previousNotice
          ) : !previous && stream.status === "error" && stream.error ? (
            <FailureState
              title={`Could not follow ${name}'s logs`}
              error={stream.error.raw}
              className="m-3"
            />
          ) : rows.length === 0 ? (
            <EmptyState
              // The body is `whitespace-nowrap` so an unwrapped line keeps the
              // fixed height the windowing depends on. Prose inherits it too,
              // and then no `max-width` can wrap: this hint ran off the pane
              // and under the rail on a real cluster.
              className="whitespace-normal"
              title="Nothing has been logged yet"
              hint={`srelens is following ${targets.length} container${targets.length === 1 ? "" : "s"} across ${podCount(targets)}; none of them has written a line${windowLabel}.`}
            />
          ) : filtered.length === 0 ? (
            // Deliberately not the sentence above it. "Nothing yet" and "nothing
            // matching" are different facts with different remedies, and one
            // message for both tells a reader their filter is fine when it is
            // the only thing hiding the line they came for.
            <EmptyState
              className="whitespace-normal"
              title="No lines match"
              hint={`${groupNumber(rows.length)} line${rows.length === 1 ? " is" : "s are"} in the buffer; none of them matches this filter.`}
              action={
                <Button variant="secondary" size="sm" onClick={clearFilters}>
                  Clear the filter
                </Button>
              }
            />
          ) : (
            <div ref={rowsRef}>
              {!previous && stream.dropped > 0 && (
                // The ring bit. A reader who scrolls to the top of a capped
                // buffer is not at the beginning of the log, and silence here
                // would let them believe they are. It is a fact about the LIVE
                // ring, though: the previous instance's buffer arrived whole,
                // in one fetch, and nothing has pushed anything out of it.
                <div className="px-2.5 py-1 text-muted">
                  Showing the newest {groupNumber(filtered.length)} lines ·{" "}
                  {groupNumber(stream.dropped)} earlier lines dropped
                </div>
              )}
              {window_.topPad > 0 && (
                <div style={{ height: window_.topPad }} aria-hidden="true" />
              )}
              {drawn.map((row, i) => (
                // The index is the key: two identical lines a second apart are
                // ordinary in a log, so nothing in a line is a stable identity.
                <LogLine
                  key={window_.start + i}
                  ts={row.ts}
                  source={sourceOf(row, " · ")}
                  // The word the LINE used, from core's one level scan — never
                  // the severity vocabulary's name for it, which appears nowhere
                  // in the log the reader is grepping against.
                  //
                  // And no `tone` beside it. The kit owns level→tone precisely
                  // so the same word is not red here and grey on the next
                  // screen; an override belongs to a line singled out for some
                  // reason OTHER than its level, and none is.
                  level={row.level}
                  message={row.message}
                />
              ))}
              {window_.bottomPad > 0 && (
                <div style={{ height: window_.bottomPad }} aria-hidden="true" />
              )}
            </div>
          )}
        </div>
      </SideRail>
    </LogsScreen>
  );
}
