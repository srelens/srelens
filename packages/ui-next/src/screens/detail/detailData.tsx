import { createElement, useEffect, useRef, useState, type ReactNode } from "react";
import {
  K8S_KIND,
  eventVerdict,
  getManifest,
  listCrds,
  listEvents,
  redactSecretManifest,
  resourceStatusLine,
  str,
  type DynamicGvk,
  type EventSummary,
  type K8sObject,
  type ResourceStatusLine,
} from "@srelens/core";
import {
  Alert,
  CodeEditor,
  LoadingState,
  StatusPill,
  Table,
  type Column,
  type TabItem,
} from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { descriptorFor } from "../../lib/kinds/descriptors";
import { useObject } from "../../lib/useObject";
import { ConfigDetailsBody } from "./ConfigBody";
import { CronJobDetailsBody } from "./CronJobBody";
import { GenericBody, identityFacts } from "./GenericBody";
import { JobDetailsBody } from "./JobBody";
import { NodeDetailsBody } from "./NodeBody";
import { PodContainersBody, PodContainersTable, PodDetailsBody, podFacts } from "./PodBody";
import { AnnotationsSection, LabelsSection } from "./sections";
import { SecretDetailsBody } from "./SecretBody";
import { ServiceDetailsBody } from "./ServiceBody";
import { useDeployRevisions, WorkloadDetailsBody, workloadFacts, type RevisionsState } from "./WorkloadBody";
import type { DetailFact, FactsFor } from "./facts";

/**
 * ONE SUBJECT, READ ONCE — everything about a resource detail except what it
 * looks like.
 *
 * srelens draws a resource in two places: a peek beside the list
 * (`ResourceDetailView`) and a page filling a tab (`ResourceTabView`). They are
 * two screens with two designs, and neither imports the other. What they must
 * never do is DISAGREE — about what the object says, about which panes a kind
 * offers, about how many times the cluster is asked. That is what lives here:
 *
 * - one read of the object (`useObject`),
 * - one lazy-load rule per pane, and one target gate over every load,
 * - one table of per-kind bodies, and one derivation of a kind's facts,
 * - one read of core's verdict on the subject.
 *
 * And what is NOT here, deliberately: every header, tab strip, footer, grid
 * and rule. A fact leaves this module as DATA ({@link DetailFact}) precisely
 * so each screen can lay it out its own way. The previous answer was
 * `FactGrid`, a kit component that took the peek's rendered rows and restyled
 * them into the tab's three columns — one screen's markup reshaped into
 * another's, which needed a new exception for every child that was not a fact
 * row and made a layout change in one screen a layout change in both. (#331)
 */

/** k8sKind → the list screen's slug, so this layer can ask the very
 *  `KindDescriptor` the list already resolves what extra panes a kind offers.
 *  Built from core's own table rather than hand-duplicated, so a kind added
 *  there is never silently unresolvable here. */
const SLUG_BY_K8S_KIND: Record<string, string> = Object.fromEntries(
  Object.entries(K8S_KIND)
    .filter(([, k8sKind]) => k8sKind !== "")
    .map(([slug, k8sKind]) => [k8sKind, slug]),
);

/** How a subject is named in a loading or error line, in either screen. */
export function describeTarget(kind: string, namespace: string | null, name: string): string {
  return `${kind} ${namespace ? `${namespace}/` : ""}${name}`;
}

/**
 * Resolves a custom resource's `{group, version, plural}` from the cluster's
 * own CRD list, for `getManifest`'s optional fifth argument.
 *
 * `getManifest(context, kind, namespace, name, invoke, crd?)` needs that GVK
 * to resolve a CRD-backed kind — kind alone is ambiguous to the backend's
 * kind→GVR match, which has no CRD path at all (the same reason
 * `KindActions.delete` is withheld for custom resources in `lib/kinds/
 * custom.ts`). This layer only ever receives a bare `kind` string (not a
 * slug, not a `CrdRef`), and there is nowhere upstream to source one from
 * yet — no descriptor represents a CRD kind today, and threading a `CrdRef`
 * through `KindDescriptor`/props would only work once every future caller
 * remembers to supply it, which is exactly the kind of coordination gap that
 * has already bitten this screen once (see Task 9's own "must remember to
 * set panes.containers" concern). Resolving it here instead means the YAML
 * pane works correctly for a custom resource the moment ANY caller passes
 * its kind — self-contained, not dependent on another task's discipline —
 * at the cost of one extra `listCrds` round trip per custom-resource YAML
 * open (skipped entirely for a built-in kind, and only paid once the reader
 * actually opens the YAML tab, matching this module's existing laziness).
 *
 * A `kind` with no CRD on the cluster (the CRD was deleted, or the caller
 * mis-typed it) is reported as an error, not silently passed through
 * unresolved: an unresolved `crd` would make `getManifest` guess via the
 * same ambiguous kind→GVR match this function exists to avoid, which can
 * fail confusingly or, worse, resolve to the wrong resource entirely.
 *
 * A `kind` claimed by MORE than one CRD is reported the same way, for the
 * same reason. Two groups can legitimately define the same `.kind`
 * (`widgets.example.com` and `widgets.other.io`), and this layer is handed a
 * bare kind string with no group to disambiguate it. Taking the first match
 * would fetch a manifest from possibly the wrong group and render it as
 * though it were the right one — a possibly-wrong success, which is worse
 * than a failure, because nothing on screen would say anything was ambiguous.
 */
async function resolveCrdGvk(
  context: string,
  kind: string,
): Promise<{ crd?: DynamicGvk; error?: string }> {
  const result = await listCrds(context);
  if (result.error) {
    return { error: `Could not look up ${kind}'s CustomResourceDefinition: ${result.error}` };
  }
  const matches = result.crds?.filter((c) => c.kind === kind) ?? [];
  if (matches.length === 0) {
    return {
      error: `${kind} has no matching CustomResourceDefinition on this cluster, so its manifest cannot be resolved.`,
    };
  }
  if (matches.length > 1) {
    // Sorted and de-duplicated so the message reads the same whichever order
    // `listCrds` happened to return them in.
    const groups = [...new Set(matches.map((c) => c.group))].sort().join(", ");
    return {
      error: `${kind} is claimed by more than one CustomResourceDefinition on this cluster (${groups}), so its manifest cannot be resolved unambiguously.`,
    };
  }
  const match = matches[0];
  return { crd: { group: match.group, version: match.version, plural: match.plural } };
}

/**
 * `kind` is the route's, not `object.kind`. A body dispatched on one kind and
 * reading another off the payload is two sources of truth for the fact its own
 * dispatch turned on — live today only because the API server happens to
 * return `kind` on a single-object GET. A body that does not need a prop simply
 * omits it from its own props. (#331)
 *
 * `revisions` is handed to every body and read by exactly one — the workload
 * body's Deploy Revisions table. It is here rather than fetched inside that
 * body because the fact list needs the same list (a Deployment's `Revision`
 * reads "119 (6m ago)") and the fact list is now data drawn by two different
 * screens: two fetches would be one list too many, and two lists arriving at
 * different moments is a pane showing a revision its own table has not heard
 * of.
 */
type PaneBody = (props: {
  kind: string;
  object: K8sObject;
  context: string;
  revisions: RevisionsState;
}) => ReactNode;

/**
 * The Details/Overview pane's per-kind TITLED blocks, keyed on `k8sKind`.
 *
 * A kind absent from the table renders no nested body of its own —
 * `GenericBody` still gives it a complete, correct pane (classic's
 * `GenericDetail`) — and a kind in `SELF_DESCRIBING_KINDS` renders its own
 * entry with no wrapper at all. A table for later work to extend, not a switch
 * to grow.
 *
 * The lead FACT LIST is not here; it is {@link DETAIL_FACTS}, because a fact
 * list is data both screens lay out differently and a block is markup both
 * screens draw the same.
 */
export const DETAILS_BODY: Record<string, PaneBody> = {
  Pod: PodDetailsBody,
  Deployment: WorkloadDetailsBody,
  StatefulSet: WorkloadDetailsBody,
  DaemonSet: WorkloadDetailsBody,
  ReplicaSet: WorkloadDetailsBody,
  Service: ServiceDetailsBody,
  Node: NodeDetailsBody,
  Job: JobDetailsBody,
  CronJob: CronJobDetailsBody,
  ConfigMap: ConfigDetailsBody,
  Secret: SecretDetailsBody,
};

/**
 * A kind's lead fact list — the unheaded block at the top of its detail.
 *
 * Keyed on `k8sKind` like {@link DETAILS_BODY}, and falling back to
 * `identityFacts` (Namespace, Created, Controlled by) for every kind without
 * one of its own, which is what `GenericBody` used to render as a section.
 * DaemonSet is deliberately absent: its numbers are per-node rather than
 * replicas, so it leads with the identity facts and states its own counts in
 * a titled Scheduling block — exactly what it did before.
 *
 * THE POINT OF THIS TABLE: it is the one derivation of a subject's facts, and
 * both screens read it. They can differ in how a fact reads and cannot differ
 * in what it says — the property `FactGrid` was protecting by restyling one
 * screen's DOM into the other's, kept without restyling anything.
 */
const DETAIL_FACTS: Record<string, FactsFor> = {
  Pod: podFacts,
  Deployment: workloadFacts,
  StatefulSet: workloadFacts,
  ReplicaSet: workloadFacts,
};

/** The facts a subject shows, whichever screen is showing them. */
export function detailFacts(input: { kind: string; object: K8sObject; revisions?: RevisionsState }): DetailFact[] {
  const build = DETAIL_FACTS[input.kind] ?? identityFacts;
  return build({ kind: input.kind, object: input.object, revisions: input.revisions?.revisions });
}

/** Same seam, for the Containers pane a kind's descriptor opts into via
 *  `panes.containers` — a tab of its own in the peek. */
const CONTAINERS_BODY: Record<string, PaneBody> = {
  Pod: PodContainersBody,
};

/** Same seam, for the Metrics pane a kind's descriptor opts into via
 *  `panes.metrics`. */
const METRICS_BODY: Record<string, PaneBody> = {};

/**
 * The full tab's INLINE containers table — the same kinds as
 * {@link CONTAINERS_BODY}, in the summary form the design draws on Overview.
 *
 * A kind opts into containers ONCE, through its descriptor's
 * `panes.containers`; these two tables only say what a container looks like on
 * each surface. A kind that sets the flag and has no entry here shows no
 * table rather than a broken one, which is the same answer the peek gives for
 * a missing `CONTAINERS_BODY`.
 */
const CONTAINERS_TABLE: Record<
  string,
  (props: { object: K8sObject; context: string }) => ReactNode
> = {
  Pod: PodContainersTable,
};

type LoadStatus = "loading" | "ready" | "error";

export interface LoadState<T> {
  status: LoadStatus;
  data?: T;
  error?: string;
}

/**
 * A pane's own data, loaded only once that pane has actually been opened for
 * the CURRENT `target`. Used by both screens — the peek's YAML and Events
 * panes, and the full tab's pod-usage tiles — under one rule rather than two
 * written to look alike. `enabled` is the caller's "the reader has looked at
 * this, for this subject" signal, not "the object is ready". A peek fills on
 * nearly every row click; fetching the manifest and the events eagerly on
 * every one of those, when a reader usually looks at neither, is two wasted
 * calls per row. The caller keeps `enabled` true across a pane switch so
 * switching back to an already-opened pane never refetches, the same
 * generation-counter guard `useObject` uses against a stale result landing
 * after the target changed — but `enabled` can also go back to false on a
 * new target (a pane the reader isn't currently on), in which case the
 * settled data for the old target is dropped rather than left rendering
 * under the new one.
 *
 * The returned value is GATED on the target the held data was fetched for
 * matching the `target` passed in THIS render — not merely reset by the
 * effect below, which only runs after commit and paint. The pane-state hook's
 * own subject-change reset (`openedPanes`) is safe because it happens
 * synchronously during render; this hook's settled data lives in its own
 * `useState`, and when the pane stays mounted across a subject change
 * (exactly what persisting `activeTab` buys), a real browser paints one
 * committed frame pairing the NEW subject's heading with the OLD subject's
 * data before that effect ever gets to run. The gate makes that
 * structurally unrenderable rather than merely fast: it is a plain
 * comparison computed fresh every render, so it holds even on the very
 * first commit after `target` changes, with no dependency on effect
 * ordering that a future refactor could quietly undo.
 */
export function useLoad<T>(
  enabled: boolean,
  target: readonly [string, string, string | null, string],
  load: () => Promise<{ data?: T; error?: string }>,
): LoadState<T> {
  const targetKey = target.join(" ");
  const [state, setState] = useState<LoadState<T> & { targetKey: string }>({
    status: "loading",
    targetKey,
  });
  const gen = useRef(0);

  useEffect(() => {
    // Reset on every identity change, not only an enabling one: `target`
    // can change while `enabled` stays whatever it already was (or flips
    // to false, on a subject change for a pane that isn't the one on
    // screen), and either way the settled data for the OLD identity must
    // not go on being held once something newer is being asked for.
    const mine = ++gen.current;
    setState({ status: "loading", targetKey });
    if (!enabled) return;
    load().then(
      (result) => {
        if (gen.current !== mine) return;
        if (result.error) {
          setState({ status: "error", error: result.error, targetKey });
          return;
        }
        setState({ status: "ready", data: result.data, targetKey });
      },
      (e: unknown) => {
        if (gen.current !== mine) return;
        setState({ status: "error", error: e instanceof Error ? e.message : String(e), targetKey });
      },
    );
    return () => {
      if (gen.current === mine) gen.current++;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, ...target]);

  // The gate itself: if the data currently held was fetched for a different
  // target than the one being rendered right now, it is not this render's
  // to show — substitute "loading" rather than let it leak through for even
  // one committed frame.
  return state.targetKey === targetKey ? state : { status: "loading" };
}

/** The pane ids both screens use. The LABELS are each screen's own — the peek
 *  heads the first pane "Details" and the tab heads it "Overview" — but a pane
 *  is the same pane in both, which is what lets one memory, one gate and one
 *  lazy-load rule serve them. */
export const PANE_DETAILS = "details";
export const PANE_CONTAINERS = "containers";
export const PANE_METRICS = "metrics";
export const PANE_YAML = "yaml";
export const PANE_EVENTS = "events";

/** One subject, as data and blocks — no chrome, no layout. */
export interface DetailSubject {
  object?: K8sObject;
  status: ReturnType<typeof useObject>["status"];
  error?: string;
  /** The kind's own row actions and extra panes, or `undefined` for a CRD. */
  descriptor: ReturnType<typeof descriptorFor>;
  /** Core's one verdict on this subject — the peek's status line and the
   *  tab's metric strip are two renderings of it, never two readings. */
  statusLine: ResourceStatusLine | null;
  /** Whether this kind's descriptor offers containers, and metrics. What each
   *  screen DOES with that differs: the peek gives containers a tab, the tab
   *  folds the table into Overview. */
  hasContainers: boolean;
  hasMetrics: boolean;
  /** The lead fact list, as data. Each screen lays it out itself. */
  facts: DetailFact[];
  /** The kind's titled blocks — a flat run of sibling sections, drawn the
   *  same in both screens, which is why they are shared as markup. Nothing
   *  may be wrapped around any one of them: `.section + .section` is what
   *  draws the hairline between two blocks. */
  body: ReactNode;
  /** The peek's Containers pane, for a kind whose descriptor offers one. */
  containersPane: ReactNode;
  /** The full tab's inline containers table, for the same kinds. */
  containersTable: ReactNode;
  metricsPane: ReactNode;
  /** Labels and Annotations close every kind's detail, so each screen places
   *  them — the peek stacks them under the rest, the tab reads them side by
   *  side. A body that drew them itself could only produce one of those.
   *  `kind` stays required on `AnnotationsSection` for the reason its own
   *  comment gives: a Secret's annotation can be the secret. */
  labels: ReactNode;
  annotations: ReactNode;
  /** This kind's manifest goes through `redactSecretManifest`. */
  isSecret: boolean;
}

/**
 * Read one subject and derive everything about it that is not a look.
 *
 * Called first by each screen; what it hands back is the same in both. The
 * screen then decides which panes it offers ({@link useDetailPaneState}) and
 * how any of it is laid out.
 */
export function useDetailSubject({
  context,
  kind,
  namespace,
  name,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
}): DetailSubject {
  const { object, status, error } = useObject(context, kind, namespace, name);

  const slug = SLUG_BY_K8S_KIND[kind];
  const descriptor = slug ? descriptorFor(slug) : undefined;
  // Never a branch on the kind's name: it is the kind's descriptor that says
  // the kind HAS containers, and each screen decides where they go.
  const hasContainers = descriptor?.panes?.containers ?? false;
  const hasMetrics = descriptor?.panes?.metrics ?? false;

  // Fetched here rather than inside the workload body, because the fact list
  // needs it too and the fact list is data now — see `useDeployRevisions`'
  // own comment. Runs for every kind and fetches for one.
  const revisions = useDeployRevisions(
    context,
    str(object?.metadata?.namespace),
    str(object?.metadata?.name),
    kind === "Deployment",
  );

  const DetailsBody = DETAILS_BODY[kind];
  const ContainersBody = CONTAINERS_BODY[kind];
  const MetricsBody = METRICS_BODY[kind];
  const ContainersTable = CONTAINERS_TABLE[kind];
  const meta = object?.metadata ?? {};

  const bodyProps = object ? { kind, object, context, revisions } : undefined;

  return {
    object,
    status,
    error,
    descriptor,
    statusLine: object ? resourceStatusLine(kind, object) : null,
    hasContainers,
    hasMetrics,
    facts: object ? detailFacts({ kind, object, revisions }) : [],
    body: bodyProps && (
      <GenericBody kind={kind} object={object as K8sObject} context={context}>
        {DetailsBody && createElement(DetailsBody, bodyProps)}
      </GenericBody>
    ),
    containersPane: bodyProps && ContainersBody ? createElement(ContainersBody, bodyProps) : null,
    containersTable:
      hasContainers && object && ContainersTable
        ? // The context travels with the object: the table's Ports cells open
          // a forward, and a forward is made in ONE cluster.
          createElement(ContainersTable, { object, context })
        : null,
    metricsPane: bodyProps && MetricsBody ? createElement(MetricsBody, bodyProps) : null,
    labels: <LabelsSection labels={meta.labels ?? {}} />,
    annotations: <AnnotationsSection kind={kind} annotations={meta.annotations ?? {}} />,
    isSecret: kind === "Secret",
  };
}

/** Which pane is showing, and the data of the two that fetch their own. */
export interface DetailPaneState {
  active: string;
  selectTab: (id: string) => void;
  /** Ready to seat in whatever chrome the screen draws. */
  yamlPane: ReactNode;
  eventsPane: ReactNode;
}

/**
 * The pane a reader is on, and the two panes that fetch for themselves.
 *
 * `tabs` is the SCREEN's — the peek offers Details/Containers/YAML/Events, the
 * tab offers Overview/YAML/Events — and all this layer does with it is refuse
 * to stay on a pane the screen does not offer. The lazy-load rule, the target
 * gate and the Secret redaction are the same in both.
 */
export function useDetailPaneState({
  context,
  kind,
  namespace,
  name,
  tabs,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  tabs: TabItem[];
}): DetailPaneState {
  const [activeTab, setActiveTab] = useState<string>(PANE_DETAILS);

  // Which panes have been opened at least once for the CURRENT subject — the
  // lazy-load gate for YAML and Events (see `useLoad`'s doc comment). Reset
  // whenever the subject changes, via React's documented "adjust state
  // during render" recipe: the comparison and the reset below happen before
  // this render's hooks (`useLoad`) run, so a subject switch can never fire
  // a stale fetch gated on the *previous* subject's opened panes, and a new
  // pod's YAML tab can never show the last pod's cached manifest.
  //
  // `activeTab` is deliberately NOT reset here. Which pane is selected is
  // navigation intent, not data that goes stale — the peek fills on nearly
  // every row click on an already-mounted screen, and someone comparing YAML
  // (or scanning Events) across several rows should not be thrown back to
  // Details, and charged a click to return, on every single one. It falls
  // back to the first pane only through the guard below (`tabs.some(...)`),
  // for the one case that genuinely needs it: the new subject's kind doesn't
  // have the pane that was selected (e.g. a Pod's Containers tab, followed by
  // a ConfigMap).
  const targetKey = `${context}|${kind}|${namespace ?? ""}|${name}`;
  const [trackedTargetKey, setTrackedTargetKey] = useState(targetKey);
  const [openedPanes, setOpenedPanes] = useState<ReadonlySet<string>>(() => new Set());
  if (targetKey !== trackedTargetKey) {
    setTrackedTargetKey(targetKey);
    // Seeded with the pane on screen right now, not emptied outright: if the
    // reader is looking at YAML or Events when the subject changes under
    // them, that pane owes them the NEW subject's data, not a permanent
    // "loading" until they re-click a tab that already looks selected. A
    // pane they are not currently on stays lazy for the new subject too.
    setOpenedPanes(new Set([activeTab]));
  }

  function selectTab(id: string) {
    setActiveTab(id);
    setOpenedPanes((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
  }

  // Gated on `openedPanes` alone, NOT also on `status === "ready"`: a
  // subject change cycles `status` through "loading" and back to "ready"
  // even when the pane was already open (the seeded-reset case above), and
  // ANDing that transient cycle into `enabled` toggled it true → false →
  // true, firing this effect twice for what is conceptually one refetch. The
  // object's own readiness doesn't gate this fetch — `getManifest` and
  // `listEvents` don't depend on `useObject` having succeeded, and while the
  // object is loading or has errored the pane isn't visible anyway (each
  // screen's own early return short-circuits before any tab renders).
  const target = [context, kind, namespace, name] as const;
  // `getManifest` needs a CRD's group/version/plural to resolve a
  // custom-resource manifest — see `resolveCrdGvk`'s own doc comment for why
  // this is looked up here rather than threaded in from a descriptor.
  const isBuiltInKind = SLUG_BY_K8S_KIND[kind] !== undefined;
  // The Details pane keeps a Secret's values out of the DOM until the reader
  // reveals them; `k8s.getManifest` returns them in the clear (only
  // `k8s.getObject` redacts — see `crates/kube/src/manifest.rs`), so without
  // this the reveal gate is worth nothing to anyone who clicks one tab over.
  // The redaction goes here, on the result, rather than inside `getManifest`:
  // classic calls that same function and deliberately shows the manifest
  // unredacted, and classic is frozen. Divergence from classic here is the
  // point, not an oversight.
  const isSecret = kind === "Secret";
  const yamlState = useLoad<string>(openedPanes.has(PANE_YAML), target, async () => {
    let crd: DynamicGvk | undefined;
    if (!isBuiltInKind) {
      const resolved = await resolveCrdGvk(context, kind);
      if (resolved.error) return { error: resolved.error };
      crd = resolved.crd;
    }
    const result = await getManifest(context, kind, namespace, name, undefined, crd);
    if (result.error !== undefined || result.yaml === undefined || !isSecret) {
      return { data: result.yaml, error: result.error };
    }
    // Fails closed: on any shape it does not understand the redactor returns
    // an error and no YAML at all, which surfaces as the pane's error rather
    // than as an unredacted manifest.
    const redacted = redactSecretManifest(result.yaml);
    return redacted.error !== undefined ? { error: redacted.error } : { data: redacted.yaml };
  });
  const eventsState = useLoad<EventSummary[]>(openedPanes.has(PANE_EVENTS), target, () =>
    listEvents(context, namespace, { kind, name }).then((r) => ({ data: r.events, error: r.error })),
  );

  // Falls back to the first pane rather than pointing at one that isn't
  // offered — relevant when a kind's panes shrink under a mounted screen, and
  // when the reader promotes a peek that was showing Containers into a tab
  // that has no such pane.
  const active = tabs.some((t) => t.id === activeTab) ? activeTab : PANE_DETAILS;

  return {
    active,
    selectTab,
    yamlPane: (
      <YamlPane state={yamlState} kind={kind} namespace={namespace} name={name} redacted={isSecret} />
    ),
    eventsPane: <EventsPane state={eventsState} kind={kind} namespace={namespace} name={name} />,
  };
}

/**
 * The manifest, and the notice that says a Secret's values are not in it.
 *
 * Shared by both screens because it is CONTENT: an editor, a loading line and
 * an error line, drawn the same however wide the surface is. Neither screen
 * restyles it.
 */
function YamlPane({
  state,
  kind,
  namespace,
  name,
  redacted,
}: {
  state: LoadState<string>;
  kind: string;
  namespace: string | null;
  name: string;
  /** This kind's manifest went through `redactSecretManifest` — say so. */
  redacted: boolean;
}) {
  if (state.status === "loading") {
    return <LoadingState label={`Loading ${describeTarget(kind, namespace, name)}'s manifest`} />;
  }
  if (state.status === "error" || state.data === undefined) {
    return (
      <FailureState
        title={`Could not load ${describeTarget(kind, namespace, name)}'s manifest`}
        error={state.error}
      />
    );
  }
  // The height, which the pane got wrong until #331's second round. Three
  // things have to hold together and only the last of them is obvious:
  //
  // - `fill` on the editor. Without it the kit's `CodeEditor` grows with its
  //   content up to `maxHeight`, which defaults to 520px — a little under 28
  //   lines of 12px type at a 1.55 line height, which is exactly where the
  //   manifest was being cut, with the rest of the pane left blank beneath
  //   it. Its own wrapper's `h-full` did not save it: `height` and
  //   `max-height` are different properties, and the cap wins the used height.
  // - a column that owns the pane's height (`h-full`), so the notice and the
  //   editor divide it rather than stack inside an auto-height box.
  // - `min-h-0` on the editor's seat. `fill` resolves to `height: 100%`,
  //   which is nothing at all against a parent whose own height is auto, and
  //   a flex child's default `min-height: auto` refuses to shrink below its
  //   content — the pair of them is what makes the editor scroll internally
  //   instead of pushing the notice off the top.
  //
  // Same slot either way, so the redacted case is one more row in the column
  // rather than a second layout to keep in step.
  return (
    <div data-slot="yaml-editor" className="flex h-full flex-col gap-2">
      {/* Told, not silently shown less: a manifest quietly missing its values
          reads as the manifest the cluster has, and someone comparing it
          against `kubectl get -o yaml` would have no idea why the two
          disagree. Tone "info" is a `status` region rather than an `alert`, so
          it never competes with this pane's own error state for a screen
          reader's attention. */}
      {redacted && (
        <Alert tone="info" title="Values redacted">
          This Secret's values are not shown here. Reveal them one key at a time in the Details pane.
        </Alert>
      )}
      <div className="min-h-0 flex-1">
        <CodeEditor value={state.data} readOnly language="yaml" fill ariaLabel={`${name} manifest`} />
      </div>
    </div>
  );
}

const EVENT_COLUMNS: Column<EventSummary>[] = [
  {
    key: "type",
    header: "Type",
    // Both halves of core's one rule for an event, not a literal comparison
    // against "Warning" written here — see `eventVerdict`. A `Badge` used to
    // sit here and could carry only `health` (a single tone on the whole
    // label); it has no dot and no notion of "colour the word but not always",
    // so it could not express the design's `bad` half at all. `StatusPill` is
    // the design's own `Status` component: the dot is always toned by `kind`,
    // and `tinted` colours and bolds the WORD only when `bad` is true — a
    // Warning reads bold and danger-red, a Normal (or any type this cluster
    // invented) reads a plain word beside a neutral dot.
    render: (e) => {
      const { health, bad } = eventVerdict(e.type);
      return <StatusPill status={e.type} kind={health} tinted={bad} />;
    },
  },
  { key: "reason", header: "Reason" },
  { key: "object", header: "Object" },
  { key: "message", header: "Message" },
  { key: "age", header: "Age" },
];

/** The subject's events. Shared for the same reason as {@link YamlPane}: a
 *  table of what happened is content, not a layout either screen owns. */
function EventsPane({
  state,
  kind,
  namespace,
  name,
}: {
  state: LoadState<EventSummary[]>;
  kind: string;
  namespace: string | null;
  name: string;
}) {
  if (state.status === "loading") {
    return <LoadingState label={`Loading events for ${describeTarget(kind, namespace, name)}`} />;
  }
  if (state.status === "error" || state.data === undefined) {
    return (
      <FailureState
        title={`Could not load events for ${describeTarget(kind, namespace, name)}`}
        error={state.error}
      />
    );
  }
  // `Table` renders `emptyText` itself for a genuinely empty list — an early
  // `return null` here would leave a healthy, event-free object looking like
  // a broken pane instead of a labelled one.
  return <Table columns={EVENT_COLUMNS} data={state.data} getRowKey={(e) => e.name} emptyText="No events" />;
}
