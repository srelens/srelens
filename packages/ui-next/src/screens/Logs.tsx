import {
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import {
  absoluteTimestamp,
  describeStreamEnd,
  clauseState,
  discoverFields,
  inRange,
  logConnectionStatus,
  logLineHealth,
  logLineLevel,
  logLineWords,
  matchLine,
  parseLogFields,
  parseLogQuery,
  podLogs,
  toggleClause,
  type ClauseMode,
  type ClauseTarget,
  type LogConnectionVerdict,
  type LogLine as StreamLine,
  type LogTarget,
  type QueryClause,
  type QueryLine,
  type TermLine,
  type TimeRange,
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
import { useLogProviderSource, useLogProviders } from "../extensions/logProviders";
import { useActiveContext } from "../lib/clusters";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { EARLIER_PAGE, MAX_LOADED_LINES, useLogStream, type LogStreamStatus } from "../lib/logStream";
import { logHistoryMb, megabytes } from "../lib/logHistory";
import { useThrottledValue } from "../lib/useThrottledValue";
import { groupNumber } from "../lib/numbers";
import { saveOrDownload } from "../lib/saveOrDownload";
import {
  resolveLogSubject,
  type LogSubject,
  type LogSubjectPod,
  type LogSubjectResolution,
  type PreviousInstance,
} from "../lib/logSubject";
import { recentKey, rememberLogSubject, useOfferedRecents } from "../lib/logRecents";
import { useMarkLogStream } from "../lib/liveLogStreams";
import { useTabScope } from "../lib/tabScope";
import { openTab } from "../lib/tabsStore";
import { FilterPills } from "./logs/FilterPills";
import { LineDetail } from "./logs/LineDetail";
import { renderMessage } from "./logs/LogMessage";
import {
  StreamRail,
  STREAM_RAIL_WIDTH,
  type StreamPod,
} from "./logs/StreamRail";
import { clockTime } from "./logs/time";
import { VolumeHistogram } from "./logs/VolumeHistogram";
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

/** The source picker's value for the cluster's own logs; a provider's is `<app>/<provider>`, which has a dot. */
const KUBERNETES_SOURCE = "kubernetes";

/**
 * How far back "all" reaches for a log provider (#569): the longest history the host
 * asks one for. The cluster keeps what a container still has; a log backend keeps far
 * more, so "all" of it is not a query anyone could wait for.
 */
const PROVIDER_ALL_SECONDS = 7 * 24 * 3600;

type ExportScope = "view" | "all" | "600" | "3600";
/** The Export picker's choices; the last three read the history on disk (#914). */
const EXPORT_SCOPES: { value: ExportScope; label: string }[] = [
  { value: "view", label: "Current view" },
  { value: "all", label: "All history" },
  { value: "600", label: "Last 10 minutes" },
  { value: "3600", label: "Last hour" },
];

/** The qualified kind a log provider is declared for (#569), from the route's kind. */
function providerKind(kind: string): string | undefined {
  return (
    {
      pod: "/Pod",
      deployment: "apps/Deployment",
      statefulset: "apps/StatefulSet",
      daemonset: "apps/DaemonSet",
    } as Record<string, string>
  )[kind.toLowerCase()];
}

/** How near the bottom still counts as being at it, in pixels. Classic's 48. */
const STICK_SLACK = 48;

/** How often, at most, a busy stream's lines reach the counts: four times a second. */
const RECOUNT_MS = 250;

/** How long the view must be held before the filter bar says so: longer than a click's press. */
const HELD_NOTE_MS = 300;

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
  completed = 0,
): string {
  return `${verdict.label} — ${live} of ${total} streaming${completed > 0 ? `, ${completed} completed` : ""}`;
}

/**
 * `2026-08-24T14:07:41.208123456Z ` — what the backend prefixes each line with
 * when a stream is opened with `timestamps: true`, which this screen always is
 * because the design gives the time its own column.
 */
const RFC3339 = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:?\d{2})? ?/;

/**
 * The stamp as epoch ms, for the histogram and the time window. Rebuilt at
 * millisecond precision rather than handed to `Date.parse` whole, because
 * the parse is only specified for three fraction digits and the backend
 * sends nine.
 */
function stampTime(stamp: RegExpMatchArray): number | undefined {
  const [, date, time, fraction = "", zone] = stamp;
  const offset = zone === undefined ? "" : zone.length === 5 ? `${zone.slice(0, 3)}:${zone.slice(3)}` : zone;
  const ms = Date.parse(`${date}T${time}${fraction.slice(0, 4)}${offset}`);
  return Number.isNaN(ms) ? undefined : ms;
}

/** No clauses: the query is empty, or it does not parse and so is not applied. */
const NO_CLAUSES: readonly QueryClause[] = [];

/** One line as the body draws it: the four columns, plus what a filter reads. */
interface Row extends QueryLine {
  /**
   * Which row this is, for as long as its line is in the buffer: the key it is drawn under. Nothing in a line can be
   * (two identical lines a second apart are ordinary), and its place cannot: a full ring moves every line up each
   * batch, and a row keyed by its place hands its element to the next line, under the reader's pointer or selection.
   */
  readonly id: number;
  /**
   * The line's instant on the reader's clock, `HH:MM:SS.mmm`, or `""` for a
   * line without a parseable stamp. Derived from `at`, so the two cannot
   * disagree about when the line happened.
   */
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
  readonly level: string;
  /** The line with its stamp off — what the term tally and a regex clause read. An export writes {@link Row.raw}. */
  readonly text: string;
  /** What the message column leads with: a JSON or logfmt `msg`, else `text`. */
  readonly message: string;
  /** Whether `message` came out of a field, so the row appends the rest inline. */
  readonly structured: boolean;
  /** The line exactly as it arrived, stamp and all — what an export writes. */
  raw: string;
  /**
   * The host cut the line at its limit and `raw` is its start (#747). Said
   * beside the line, never written into it, so the filter and an export read
   * what the container wrote.
   */
  truncated: boolean;
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
function toRow(line: StreamLine, byLabel: ReadonlyMap<string, LogTarget>, id: number): Row {
  const stamp = line.text.match(RFC3339);
  const at = stamp ? stampTime(stamp) : undefined;
  // The reader's clock, like the histogram's axis and the window pill beside
  // it, rather than the stamp's own wall time (UTC from the backend): two
  // clocks on one screen put every line a zone's offset from the axis.
  const ts = at === undefined ? "" : `${clockTime(at)}.${String(new Date(at).getMilliseconds()).padStart(3, "0")}`;
  const text = stamp ? line.text.slice(stamp[0].length) : line.text;
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
  const health = logLineHealth(text);
  const parsed = parseLogFields(text);
  return {
    id,
    ts,
    pod,
    container,
    // An unlabelled line is the single-target stream, whose gutter would
    // otherwise be blank; a labelled one shows the container exactly when its
    // label carried it.
    namesContainer: line.source === "" || slash >= 0,
    level: logLineLevel(text) ?? "",
    health,
    text,
    message: parsed.message,
    structured: parsed.structured,
    fields: parsed.fields,
    words: logLineWords(parsed.message),
    ...(at === undefined ? {} : { at }),
    raw: line.text,
    truncated: line.truncated === true,
    haystack: `${text} ${health}`.toLowerCase(),
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
 * whose name outlives its pods. The check is `useOfferedRecents`, which Home's
 * "Pick up where you left off" runs too.
 */
function RecentSubjects({ context, clusterId }: { context: string; clusterId: string }) {
  const offered = useOfferedRecents(context, clusterId);

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
      kind={kind}
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
  kind,
  namespace,
  name,
  targets,
  pods,
  terminated,
}: {
  context: string;
  clusterName: string;
  kind: string;
  namespace: string;
  name: string;
  targets: LogTarget[];
  pods: LogSubjectPod[];
  terminated: PreviousInstance[];
}) {
  const { ask } = useConsole();
  const [text, setText] = useState("");
  /** The window picked off the histogram, if any. The one filter that isn't query text. */
  const [range, setRange] = useState<TimeRange | undefined>(undefined);
  /** Where focus goes when a pill is removed: the × that had it is about to unmount. */
  const searchRef = useRef<HTMLInputElement>(null);
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
  /** What Export writes: the view on screen, or the history on disk (#914). */
  const [exportScope, setExportScope] = useState<ExportScope>("view");
  /** Read once, when the screen opens its stream: a change applies to the next one, never restarts this. */
  const [spoolMaxBytes] = useState(() => megabytes(logHistoryMb()));
  /** The line the detail panel is open on. The Row itself, so the panel outlives the line leaving the view. */
  const [selected, setSelected] = useState<Row | null>(null);

  const viewportRef = useRef<HTMLDivElement>(null);
  const rowsRef = useRef<HTMLDivElement>(null);
  /**
   * Whether new lines should move the view. Held in a ref, not state: it is
   * written from a scroll handler that fires far more often than the screen
   * renders, and read by a layout effect that must see the value from the
   * scroll that just happened, not from the render before it.
   */
  const stickRef = useRef(true);
  /** An arrow key moved the selection and its row is still to be scrolled into view; see {@link revealSelected}. */
  const revealRef = useRef(false);
  /**
   * The selected line as of the last commit or scroll, for the effect that holds it in place: its place in
   * `filtered`, where its row is drawn within the rows (when it is drawn), how tall the earlier-lines banner
   * was, and the scroll position.
   */
  const heldRef = useRef<{
    row: Row;
    index: number;
    top: number | undefined;
    banner: number;
    scroll: number;
    wrap: boolean;
  } | null>(null);

  /**
   * Where the lines come from (#569): the cluster, or one of the installed apps'
   * log providers for this kind. The picker is drawn only when there is a choice.
   * A provider's lines carry `pod/container` from its own labels, so the rail and
   * the container filter read them as they read the cluster's.
   */
  const resourceKind = providerKind(kind);
  const offered = useLogProviders(context, resourceKind);
  const providers = offered.choices;
  const [picked, setPicked] = useState<{ key: string; label: string } | null>(null);
  /** The provider that stopped being offered while followed, named until the next choice. */
  const [lost, setLost] = useState<string | null>(null);
  const chosen = picked ? providers.find((p) => p.key === picked.key) : undefined;
  // Only a current list says a provider is gone; until one answers, the last list is
  // followed. A gone provider is let go of, so it does not come back unasked.
  if (picked && !chosen && offered.status === "ready") {
    setPicked(null);
    setLost(picked.label);
    setSelected(null);
  }
  const provider = useLogProviderSource(chosen, { context, namespace, resourceKind: resourceKind ?? "", name });
  const chooseSource = useCallback(
    (key: string) => {
      const next = providers.find((p) => p.key === key);
      setPicked(next ? { key: next.key, label: next.label } : null);
      setLost(null);
      // A different source is a different buffer: the line the panel holds is not in it.
      setSelected(null);
      // A previous instance is the cluster's to hand back, never a provider's.
      if (next) setPrevious(false);
    },
    [providers],
  );

  const sinceSeconds = SINCE.find((s) => s.value === since)?.seconds;
  const stream = useLogStream(context, namespace, provider.targets ?? targets, {
    // Always on: the design gives the time its own column, so the stamp is not
    // an option the reader turns on — it is where the first column comes from.
    timestamps: true,
    sinceSeconds: chosen ? (sinceSeconds ?? PROVIDER_ALL_SECONDS) : sinceSeconds,
    tailLines: TAIL_LINES,
    source: provider.source,
    // The cluster's own stream keeps its history on disk (#914); an app's source keeps its own.
    spoolMaxBytes: provider.source ? undefined : spoolMaxBytes,
  });
  /** Whether the stream keeps a history the screen can load from and export (#914). */
  const spooled =
    !previous && stream.history !== undefined && stream.history.failed === undefined && stream.exportSpool !== undefined;
  // Home's "Live now" lists this tab only while the stream is actually running.
  useMarkLogStream(useTabScope(), stream.status !== "error" && stream.status !== "completed");
  // A provider's stream that ended sends no more statuses, so the hook's last one
  // would go on saying it follows: the ending is what the readout says instead.
  const ended = chosen ? provider.end : null;
  /** Whether new lines are arriving in THIS pane. A snapshot is not followed,
   *  however healthy the connection underneath it is. */
  const following = !previous && !stream.paused && !ended;

  /**
   * Hold the view while the reader works in it: from a press in the log to its release, and while a selection lies
   * in it. A following view moves every frame, so a press and its release land on different lines, which is no
   * click, and the text under a selection is rewritten before it can be copied. The reader's Pause is theirs and
   * is left alone; see the hook's `hold`.
   */
  const pressedRef = useRef(false);
  const selectingRef = useRef(false);
  const { hold } = stream;
  const syncHold = useCallback(() => hold(pressedRef.current || selectingRef.current), [hold]);
  useEffect(() => {
    const onSelection = () => {
      const selection = document.getSelection();
      const viewport = viewportRef.current;
      selectingRef.current =
        selection !== null &&
        !selection.isCollapsed &&
        viewport !== null &&
        (viewport.contains(selection.anchorNode) || viewport.contains(selection.focusNode));
      syncHold();
    };
    // Anywhere on the window: a drag that started in the log can end anywhere, and a press the browser takes back,
    // or whose release never reaches the page (the window loses focus or is hidden), must not leave the view held.
    // Let go a task later, after the click a release makes: let go at once, the lines that waited move the view
    // first, and the pressed row is gone. A selection is not let go of here; only collapsing or leaving the log does.
    // A release is heard on its way down, so nothing on the page that stops it on its way up keeps it from the
    // window. Not `blur`: on the way down that is every field losing focus, as one does to the press itself; on
    // the way up, where none of those bubble, it is the window's own.
    let letGo: ReturnType<typeof setTimeout> | undefined;
    const onRelease = () => {
      if (!pressedRef.current) return;
      pressedRef.current = false;
      letGo = setTimeout(syncHold, 0);
    };
    const onVisibility = () => {
      if (document.hidden) onRelease();
    };
    const releases = ["pointerup", "pointercancel"] as const;
    document.addEventListener("selectionchange", onSelection);
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("blur", onRelease);
    for (const type of releases) window.addEventListener(type, onRelease, true);
    return () => {
      clearTimeout(letGo);
      document.removeEventListener("selectionchange", onSelection);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("blur", onRelease);
      for (const type of releases) window.removeEventListener(type, onRelease, true);
    };
  }, [syncHold]);
  /**
   * Whether the hold has lasted long enough to name. Every click holds the view for as long as the button is down,
   * and a note that flashed on each one would be noise; a press kept, or a selection, is worth the words.
   */
  const [heldLong, setHeldLong] = useState(false);
  useEffect(() => {
    if (!stream.held) {
      setHeldLong(false);
      return;
    }
    const timer = setTimeout(() => setHeldLong(true), HELD_NOTE_MS);
    return () => clearTimeout(timer);
  }, [stream.held]);
  /**
   * The restarts the "Scrollback cleared" notice is not about. That notice says a
   * change of window reopened the stream and nothing already sent comes back. A change
   * of source (a choice, a provider no longer offered, Follow again) reopens it with
   * the new source's own history, so the restart it causes is marked as seen.
   */
  const sourceKey = provider.source?.key;
  const [sourceSeen, setSourceSeen] = useState(sourceKey);
  if (sourceSeen !== sourceKey) {
    setSourceSeen(sourceKey);
    setSeenRestart(stream.restartCount + 1);
  }

  const byLabel = useMemo(() => indexTargets(targets), [targets]);
  /**
   * Each line made into a row ONCE. The ring hands back the same line objects
   * on every commit (`appendLogLines` concatenates), so a cache keyed on the
   * object turns "re-split every buffered line per arriving line" into one
   * parse per line. That matters now that a row is a JSON parse and a logfmt
   * scan, not one regex. Rebuilt with `byLabel`, which a row's pod and
   * container are read through.
   */
  const rowOf = useMemo(() => {
    const cache = new WeakMap<StreamLine, Row>();
    let next = 0;
    return (line: StreamLine): Row => {
      let row = cache.get(line);
      if (row === undefined) {
        row = toRow(line, byLabel, next++);
        cache.set(line, row);
      }
      return row;
    };
  }, [byLabel]);
  const liveRows = useMemo(() => stream.lines.map((l) => rowOf(l)), [stream.lines, rowOf]);

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
    () => (snapshot.status === "ready" ? snapshot.lines.map((l) => rowOf(l)) : []),
    [snapshot, rowOf],
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

  /** The query as typed: the pills and facet toggles edit this text by the spans it reports. */
  const typed = useMemo(() => parseLogQuery(text), [text]);
  /**
   * The query as applied, a beat behind the keystroke: React renders the box
   * first and the pass over the buffer after, so typing never waits on it.
   */
  const deferredText = useDeferredValue(text);
  const applied = useMemo(() => parseLogQuery(deferredText), [deferredText]);
  const clauses = applied.ok ? applied.clauses : NO_CLAUSES;

  /** The pod boxes and the container select: the scope every view starts from. */
  const inScope = useCallback(
    (row: Row) => !hidden.includes(row.pod) && (container === ALL_CONTAINERS || row.container === container),
    [hidden, container],
  );
  /** Every filter applied: what the body draws, from every batch. */
  const filtered = useMemo(
    () =>
      rows.filter(
        (row) =>
          inScope(row) &&
          (clauses.length === 0 || matchLine(row, clauses)) &&
          (range === undefined || inRange(row, range)),
      ),
    [rows, inScope, clauses, range],
  );
  /**
   * The rows the rail, the facets and the histogram count. Each count is a pass over the whole buffer, and a busy
   * stream commits every frame: while lines are arriving these take the stream's rows at most four times a second,
   * the last batch always landing. A filter still applies at once, over the rows they have. Paused or reading a
   * snapshot, nothing is arriving to hold back.
   */
  const counted = useThrottledValue(rows, following ? RECOUNT_MS : 0);
  const scoped = useMemo(() => counted.filter(inScope), [counted, inScope]);
  /** The query applied, but not the window: what the histogram counts, so it can shade the window instead of cropping to it. */
  const queried = useMemo(
    () => (clauses.length === 0 ? scoped : scoped.filter((row) => matchLine(row, clauses))),
    [scoped, clauses],
  );
  /** The body's lines as of the last recount: what the rail tallies. */
  const countedFiltered = useMemo(
    () => (range === undefined ? queried : queried.filter((row) => inRange(row, range))),
    [queried, range],
  );
  /** In scope and in the window, but not queried: what the facets count over. */
  const windowed = useMemo(
    () => (range === undefined ? scoped : scoped.filter((row) => inRange(row, range))),
    [scoped, range],
  );
  /**
   * The buffer's facet fields, from every row: sections must not come and go as
   * the reader filters. Found over the deferred rows and handed on as the SAME
   * array while the names are unchanged — a new array on every batch would
   * recount every facet in the urgent render, ahead of the deferral below.
   */
  const deferredRows = useDeferredValue(counted);
  const fieldsRef = useRef<string[]>([]);
  const fields = useMemo(() => {
    const next = discoverFields(deferredRows);
    const last = fieldsRef.current;
    if (next.length === last.length && next.every((f, i) => f === last[i])) return last;
    return (fieldsRef.current = next);
  }, [deferredRows]);
  /** The facets recount a beat behind, like the query, so a busy stream never starves the box. */
  const facetLines = useDeferredValue(windowed);
  /** The histogram counts the queried lines, not the windowed ones, and recounts a beat behind too. */
  const volumeLines = useDeferredValue(queried);
  /**
   * What the legend offers: the query minus its level clauses, as the rail's
   * Level facet counts, so picking a level leaves the others on offer. Not the
   * window, for the legend describes the chart. With no level clause that is
   * `queried` itself, and the pass over the buffer is not made twice.
   */
  const legendRows = useMemo(
    () =>
      clauses.some((c) => c.field?.toLowerCase() === "level")
        ? scoped.filter((row) => matchLine(row, clauses, "level"))
        : queried,
    [scoped, clauses, queried],
  );
  const legendLines = useDeferredValue(legendRows);
  /** A facet click: rewrites the search box, the one place filters live. */
  const toggleFilter = useCallback(
    (target: ClauseTarget, mode: ClauseMode) => setText((q) => toggleClause(q, target, mode)),
    [],
  );
  /**
   * A legend click: every level word of a group in (`include`) or out, in one
   * update. `toggleClause` flips what is there, so only the words that are not
   * already where the click wants them are flipped: with `level:error` typed,
   * "put the group in" adds `fatal` and leaves `error` be. Each word is read
   * off the text being edited, not off what the legend was drawn from, and a
   * word that is excluded is not included, so it flips in place.
   */
  const toggleLevels = useCallback(
    (words: string[], include: boolean) =>
      setText((q) => {
        const parsed = parseLogQuery(q);
        const included = (value: string) =>
          parsed.ok && clauseState(parsed.clauses, { field: "level", value }) === "include";
        return words
          .filter((value) => included(value) !== include)
          .reduce((query, value) => toggleClause(query, { field: "level", value }, "include"), q);
      }),
    [],
  );

  const selectedIndex = selected === null ? -1 : filtered.indexOf(selected);

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
    setRange(undefined);
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
   * applied, as of the last recount — and each one's RFC3339 stamp already off.
   *
   * The stamp is the trap. A stamped line opens with a digit, `tallyLogTerms`
   * reads a leading digit as a value and ends the term run there, so a stamped
   * buffer tallies to NOTHING — no error, no warning, just an empty rail that
   * looks exactly like a quiet log. `row.text` is the stripped text; the
   * raw line never goes near this.
   *
   * The row's message and severity go with it. The tally reads a JSON `msg`
   * from the text, but not a logfmt one: a logfmt line opens with its own
   * `time=…`, and the tally would end the term there and show nothing.
   */
  const railLines = useMemo<TermLine[]>(
    () => countedFiltered.map((row) => ({ source: row.pod, text: row.text, message: row.message, health: row.health })),
    [countedFiltered],
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
   * shape. The column on screen shows the time on the reader's clock, to the
   * millisecond (`14:07:41.208`), because a reader watching a live tail knows
   * what day it is; a file read tomorrow does not, and the full RFC3339, in
   * the zone the cluster sent it, is what makes an exported line greppable
   * against anything else. Which lines are exported is what "on screen"
   * governs — not how much of each one an 86px gutter had room for.
   *
   * While paused, `lines` is the frozen view, so this exports exactly the
   * pane the reader paused. That is the point: someone who pauses on a
   * failure and exports means "this, the thing I stopped on", not "this plus
   * the nine hundred lines that arrived while I was reading it".
   *
   * With history on disk, the picker beside Export chooses the history instead (#914).
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

  /** Export the history on disk: all of it, or the last ten minutes or hour of it (#914). */
  function exportHistory(scope: Exclude<ExportScope, "view">) {
    setSaveError(undefined);
    void stream
      .exportSpool?.(`${name}.log`, scope === "all" ? undefined : Number(scope))
      .catch((e: unknown) => setSaveError(e));
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
    // A selected line pins the view: the scroll that reveals it can land within
    // the slack of the end, and that is not the reader coming back to follow.
    stickRef.current =
      selected === null &&
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <
        STICK_SLACK;
    // The reader moved, or the browser did: either way the next shift is measured from here.
    if (heldRef.current !== null) heldRef.current.scroll = viewport.scrollTop;
    measure();
  }

  const signal = ended
    ? logConnectionStatus(ended.type === "error" ? "error" : "completed")
    : connectionSignal(stream.status, stream.paused);
  // A provider sends its history again on every restart, so none of its restarts
  // cleared anything; the notice is about the cluster's own stream alone.
  const restarted = !chosen && stream.restartCount > seenRestart;
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

  /**
   * Hold the selected line where it is on screen. Once the ring is full each
   * batch drops lines from the front, and everything after them moves up under a
   * `scrollTop` that has not changed: the line being read would slide out of
   * view, and the panel opened on it would be left describing a line the
   * reader can no longer see. So the view moves with the content, by how far the
   * line's row moved.
   *
   * That is measured where the row is drawn, which counts everything above it
   * that moved, the earlier-lines banner that the first drop brings in among it.
   * A row outside the drawn slice has no element, and is placed by arithmetic
   * instead: its place in `filtered` times the row height, and the banner. The
   * new position is the last known one plus that shift, not the browser's
   * current one plus it: a batch that shortens the content gets the scroll
   * position clamped by the layout this effect has just forced, and a shift
   * added on top of that would correct twice.
   *
   * It acts on a change to the lines or the selection, never on a scroll: the
   * reader's own scrolling is theirs, and only moves where the shift is
   * measured from. A line that is not in `filtered`, before or after, has
   * nowhere to be held.
   */
  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const box = rowsRef.current;
    const row = box?.querySelector<HTMLElement>('[aria-current="true"]');
    // Within the rows, which scrolling does not change.
    const top = box && row ? row.getBoundingClientRect().top - box.getBoundingClientRect().top : undefined;
    const banner = box?.querySelector('[data-slot="dropped-banner"]')?.getBoundingClientRect().height ?? 0;
    const held = heldRef.current;
    if (
      viewport !== null &&
      held !== null &&
      held.row === selected &&
      held.wrap === wrap &&
      held.index >= 0 &&
      selectedIndex >= 0 &&
      !stickRef.current
    ) {
      const moved =
        top !== undefined && held.top !== undefined
          ? top - held.top
          : wrap
            ? 0
            : (selectedIndex - held.index) * metrics.rowHeight + banner - held.banner;
      if (moved !== 0) {
        viewport.scrollTop = held.scroll + moved;
        // The window was drawn from the scroll position before this; draw it again from the new one.
        measure();
      }
    }
    heldRef.current =
      selected === null || viewport === null
        ? null
        : { row: selected, index: selectedIndex, top, banner, scroll: viewport.scrollTop, wrap };
  }, [filtered, selected, selectedIndex, wrap, metrics.rowHeight, window_.start, window_.end, measure]);

  function moveSelection(step: 1 | -1) {
    if (filtered.length === 0) return;
    const next =
      selectedIndex < 0
        ? step > 0
          ? 0
          : filtered.length - 1
        : Math.min(filtered.length - 1, Math.max(0, selectedIndex + step));
    setSelected(filtered[next]);
    // Reading, not following: a new line must not yank the view off the selection.
    stickRef.current = false;
    revealRef.current = true;
    // A row outside the drawn slice has no element to scroll to. Fixed-height arithmetic gets the view near
    // enough to draw it; it leaves out the scroller's padding and the dropped-lines banner, so it is only
    // that close, and the reveal below settles the pixels.
    const viewport = viewportRef.current;
    if (viewport !== null && window_.virtualized && (next < window_.start || next >= window_.end)) {
      viewport.scrollTop = next * metrics.rowHeight;
    }
    // Same row again (an arrow at the end of the buffer): no render is coming to reveal it.
    if (next === selectedIndex) revealSelected();
  }

  /**
   * Scroll the row an arrow key selected into view, once it has an element.
   * The browser does the pixels: rows do not start at the top of the scroller
   * (its padding, the dropped-lines banner), and wrapped rows differ in height,
   * so no sum over a row height can say where one is. A row outside the slice
   * is not there yet; the scroll that draws it comes back here through the effect.
   * Only a key asks for it, so a reader who scrolls away is left where they are.
   */
  function revealSelected() {
    if (!revealRef.current) return;
    const row = rowsRef.current?.querySelector<HTMLElement>('[aria-current="true"]');
    if (row === null || row === undefined) return;
    revealRef.current = false;
    row.scrollIntoView({ block: "nearest" });
    // The view has moved, and the scroll event that says so only arrives at the next rendering step. A batch that
    // lands before it is held from here, not from where the view was.
    const viewport = viewportRef.current;
    if (heldRef.current !== null && viewport !== null) heldRef.current.scroll = viewport.scrollTop;
  }
  useLayoutEffect(revealSelected, [selected, window_.start, window_.end]);

  function closeDetail() {
    setSelected(null);
    // The × and Esc in the panel unmount the control that had focus; the log is where the reader was.
    viewportRef.current?.focus();
  }

  function onLogKeyDown(e: KeyboardEvent) {
    // A modified arrow is the browser's or the OS's (selection, word jumps, history).
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    if (e.key === "ArrowDown") moveSelection(1);
    else if (e.key === "ArrowUp") moveSelection(-1);
    else if (e.key === "Escape" && selected !== null) closeDetail();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  const windowLabel = since !== "all" ? ` in the last ${since}` : chosen ? " in the last 7 days" : "";

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
            // An ended stream has nothing to pause; Follow again, by its notice, opens it.
            disabled={previous || ended !== null}
            aria-label={previous ? `Follow — ${NO_FOLLOWING.toLowerCase()}` : undefined}
          >
            {following ? (
              <Icons.pause size={13} aria-hidden="true" />
            ) : (
              <Icons.play size={13} aria-hidden="true" />
            )}
            {following ? "Pause" : "Follow"}
          </Button>
          {spooled && (
            <Select aria-label="What to export" value={exportScope}
              onValueChange={(value) => setExportScope(value as ExportScope)} options={EXPORT_SCOPES} />
          )}
          {/* Disabled on an empty view rather than hidden: a control that
              vanishes leaves the reader wondering whether this screen exports
              at all, where a greyed one says "yes, once there is something to
              export". Classic disabled it on the same condition. A history
              on disk is something to export even when the view is empty. */}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => (spooled && exportScope !== "view" ? exportHistory(exportScope) : exportView())}
            disabled={(!spooled || exportScope === "view") && filtered.length === 0}
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
            query={{ lines: facetLines, clauses, fields, onToggle: toggleFilter }}
          />
        }
      >
        <FilterBar
          value={text}
          onValueChange={setText}
          label="Filter lines"
          placeholder="Filter lines"
          inputRef={searchRef}
        >
          {providers.length > 0 && (
            <div className="flex items-center gap-1.5">
              {/* "from", not "source": the rail's Sources are the pods, and one
                  word must not name two things on one screen. */}
              <Eyebrow>from</Eyebrow>
              <Select
                value={chosen ? chosen.key : KUBERNETES_SOURCE}
                onValueChange={chooseSource}
                options={[
                  { value: KUBERNETES_SOURCE, label: "Kubernetes" },
                  ...providers.map((p) => ({ value: p.key, label: p.label })),
                ]}
                // The snapshot of a terminated container is the cluster's; a
                // provider is a source of the live tail.
                disabled={previous}
                aria-label="Logs from"
              />
            </div>
          )}
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
              other than the live stream must not look like one that is. Not
              drawn for a provider: a terminated container's buffer is the
              cluster's, and a log backend has no "previous instance". */}
          {!chosen && (
            <Button
              variant="secondary"
              size="xs"
              aria-pressed={previous}
              onClick={() => {
                setPrevious((p) => !p);
                // A different buffer: the line the panel holds is not in it, and a time window is a stretch
                // of this instance's clock. A change of source keeps the window: it is the same stretch of time.
                setSelected(null);
                setRange(undefined);
              }}
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
          )}
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
          {/* Why a following pane stopped moving under a press or a selection, and what waits for its release. */}
          {!previous && !stream.paused && stream.held && heldLong && (
            <Eyebrow>
              {`Held while you select${stream.pendingWhilePaused > 0 ? ` · ${groupNumber(stream.pendingWhilePaused)} new lines` : ""}`}
            </Eyebrow>
          )}
          {!previous && (
            <LiveSignal
              label={connectionLabel(
                signal,
                ended ? 0 : stream.liveTargets,
                stream.totalTargets,
                stream.completedTargets,
              )}
              tone={statusTone(signal.health)}
            />
          )}
        </FilterBar>
        {!typed.ok && (
          // Not FilterBar's `invalid`, whose screen-reader text calls every
          // problem an invalid regular expression. An unclosed quote is not one.
          <div role="alert" className="mx-3 mt-2 text-[0.75rem]" style={{ color: toneColor("sev") }}>
            {typed.error}
          </div>
        )}
        <FilterPills
          query={text}
          clauses={typed.ok ? typed.clauses : NO_CLAUSES}
          range={range}
          onQueryChange={setText}
          onRangeChange={setRange}
          onRemoved={() => searchRef.current?.focus()}
        />
        <VolumeHistogram
          lines={volumeLines}
          legendLines={legendLines}
          range={range}
          onRangeChange={setRange}
          clauses={typed.ok ? typed.clauses : NO_CLAUSES}
          onToggleLevels={toggleLevels}
        />

        {offered.status === "error" && (
          // The list the picker is drawn from could not be read again: what it
          // offers may be out of date, and an app that stopped providing logs
          // is not known to have.
          <FailureAlert
            title="Could not check which apps provide logs here"
            error={offered.error}
            domain="local"
            className="mx-3 mt-3"
          />
        )}
        {lost && !chosen && (
          // The choice was made from a list that has since changed: say so
          // rather than quietly draw the cluster's lines under the old name.
          <Alert
            tone="warn"
            title={`${lost} is no longer offered for this ${kind}`}
            className="mx-3 mt-3"
          >
            The app that provides it was disabled or removed, or is no longer
            enabled for this cluster. Following Kubernetes instead.
          </Alert>
        )}
        {chosen && provider.end && (
          // A provider's stream ends only with a reason, and a failure is not
          // an ending: `describeStreamEnd` words the two differently.
          <Alert
            tone={provider.end.type === "error" ? "sev" : "info"}
            title={describeStreamEnd(provider.end)}
            className="mx-3 mt-3"
          >
            <Button
              variant="secondary"
              size="xs"
              onClick={provider.retry}
            >
              Follow again
            </Button>
          </Alert>
        )}
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

        {/* Only while the history is in play: over the previous instance, or once the history stopped, the
            error belongs to a load the reader can no longer retry. */}
        {spooled && stream.loadError !== undefined && (
          <FailureAlert title="Could not load earlier lines" error={stream.loadError} className="mx-3 mt-3" />
        )}

        <div
          ref={viewportRef}
          onScroll={trackScroll}
          onPointerDown={() => {
            pressedRef.current = true;
            syncHold();
          }}
          // A tab stop: the arrow keys move the selection, so the log has to be reachable without a pointer.
          tabIndex={0}
          onKeyDown={onLogKeyDown}
          role="log"
          aria-label={`${name} logs`}
          className={`scroll log-body min-h-0 flex-1 py-1 font-mono text-[0.75rem] leading-[1.85] ${
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
              hint={
                chosen && ended
                  ? `${chosen.label} sent no line${windowLabel} before its stream ended.`
                  : chosen
                    ? `srelens is following ${name} through ${chosen.label}; it has sent no line${windowLabel}.`
                    : `srelens is following ${targets.length} container${targets.length === 1 ? "" : "s"} across ${podCount(targets)}; none of them has written a line${windowLabel}.`
              }
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
              {spooled && stream.history !== undefined ? (
                (() => {
                  const onDisk = stream.dropped + 1 - stream.history.first;
                  const passed = stream.history.first - 1;
                  if (onDisk <= 0 && passed <= 0) return null;
                  // The lines before the first one shown are on disk, not gone (#914):
                  // the banner says how many and fetches them a page at a time.
                  return (
                    <div
                      data-slot="dropped-banner"
                      // Its button is no press "in the log" to hold the view for: its click pauses the view itself.
                      onPointerDown={(e) => e.stopPropagation()}
                      className="flex flex-wrap items-center gap-x-2 px-2.5 py-1 text-muted"
                    >
                      {onDisk > 0 && <span>{groupNumber(onDisk)} earlier lines on disk</span>}
                      {onDisk > 0 &&
                        (stream.lines.length >= MAX_LOADED_LINES ? (
                          <span>{groupNumber(MAX_LOADED_LINES)} lines loaded · Export for the rest</span>
                        ) : (
                          // `aria-disabled`, not `disabled`: a disabled button drops the keyboard focus it has, and
                          // the reader would have to find the button again for the next page.
                          <Button
                            variant="secondary"
                            size="sm"
                            aria-disabled={stream.loadingEarlier || undefined}
                            className="aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
                            onClick={() => {
                              if (stream.loadingEarlier) return;
                              // A following view moves every frame, and a busy stream pushes the page it is reading
                              // straight back out of it (or past the lines it must join), so the page goes into a view
                              // that holds still. Following again is the reader's to ask for.
                              if (!stream.paused) stream.togglePause();
                              void stream.loadEarlier();
                            }}
                          >
                            {stream.loadingEarlier ? "Loading…" : `Load ${groupNumber(EARLIER_PAGE)} earlier`}
                          </Button>
                        ))}
                      {passed > 0 && <span>{groupNumber(passed)} oldest lines passed the history limit</span>}
                    </div>
                  );
                })()
              ) : (
                !previous && stream.dropped > 0 && (
                  // The ring bit. A reader who scrolls to the top of a capped
                  // buffer is not at the beginning of the log, and silence here
                  // would let them believe they are. It is a fact about the LIVE
                  // ring, though: the previous instance's buffer arrived whole,
                  // in one fetch, and nothing has pushed anything out of it.
                  <div data-slot="dropped-banner" className="px-2.5 py-1 text-muted">
                    Showing the newest {groupNumber(filtered.length)} lines ·{" "}
                    {groupNumber(stream.dropped)} earlier lines dropped
                  </div>
                )
              )}
              {window_.topPad > 0 && (
                <div style={{ height: window_.topPad }} aria-hidden="true" />
              )}
              {drawn.map((row) => (
                // The row's own id is the key, so a drawn line keeps its element
                // while the batches move it (see `Row.id`). It sits on the wrapper,
                // which is the click target and carries the selection; `.logline`
                // inside it is what `measure` reads.
                <div
                  key={row.id}
                  aria-current={row === selected ? "true" : undefined}
                  onClick={() => {
                    // A drag that selected text to copy is not a request for the panel.
                    if (window.getSelection()?.toString()) return;
                    // Reading this line, not following: a new line must not carry it away, and the panel
                    // opening takes the bottom of the pane, where this row may be.
                    stickRef.current = false;
                    revealRef.current = true;
                    setSelected(row);
                    // Same row again: no render is coming to reveal it.
                    if (row === selected) revealSelected();
                  }}
                  className="cursor-pointer"
                  style={row === selected ? { background: toneWash("accent") } : undefined}
                >
                  <LogLine
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
                    message={renderMessage(row, clauses)}
                  />
                </div>
              ))}
              {window_.bottomPad > 0 && (
                <div style={{ height: window_.bottomPad }} aria-hidden="true" />
              )}
            </div>
          )}
        </div>
        {selected !== null && (
          <LineDetail
            line={selected}
            inView={selectedIndex >= 0}
            clauses={clauses}
            onToggle={toggleFilter}
            onClose={closeDetail}
          />
        )}
      </SideRail>
    </LogsScreen>
  );
}
