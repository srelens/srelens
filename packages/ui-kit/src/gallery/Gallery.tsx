import { useEffect, useState } from "react";
import { ActionBar } from "../ActionBar";
import { AgentMark } from "../AgentMark";
import { Alert } from "../Alert";
import { ArcField } from "../ArcField";
import { AskChip } from "../AskChip";
import { Avatar } from "../Avatar";
import { Badge } from "../Badge";
import { Breadcrumb } from "../Breadcrumb";
import { Button } from "../Button";
import { ClusterRail } from "../ClusterRail";
import { CodeEditor } from "../CodeEditor";
import { ColumnPicker } from "../ColumnPicker";
import { Combobox } from "../Combobox";
import { Checkbox } from "../Checkbox";
import { ConfirmDialog } from "../ConfirmDialog";
import { Dialog } from "../Dialog";
import { DiffLines, type DiffRow } from "../DiffLines";
import { ConsoleDock } from "../ConsoleDock";
import { ContextMenu } from "../ContextMenu";
import { CopyCommand } from "../CopyCommand";
import { CustomizeMark, type MarkAppearance } from "../CustomizeMark";
import { Drawer } from "../Drawer";
import { DrillCard } from "../DrillCard";
import { EmptyState } from "../EmptyState";
import { ErrorState } from "../ErrorState";
import { Eyebrow } from "../Eyebrow";
import { Field } from "../Field";
import { FilterBar } from "../FilterBar";
import { IconButton } from "../IconButton";
import { Inspector } from "../Inspector";
import { KubectlPreview } from "../KubectlPreview";
import { KV, KVList } from "../KV";
import { LiveSignal } from "../LiveSignal";
import { LoadingState } from "../LoadingState";
import { LogLine } from "../LogLine";
import { Mark } from "../Mark";
import { Meter } from "../Meter";
import { MultiSelect } from "../MultiSelect";
import { MetricTile } from "../MetricTile";
import { NavIcon } from "../NavIcon";
import { PairList } from "../PairList";
import { Panel } from "../Panel";
import { Popover } from "../Popover";
import { Progress } from "../Progress";
import { Radio } from "../Radio";
import { RawError } from "../RawError";
import { ResizeHandle } from "../ResizeHandle";
import { ResourceTree, type ResourceNode } from "../ResourceTree";
import { Screen } from "../Screen";
import { Section } from "../Section";
import { SegmentBar } from "../SegmentBar";
import { Select } from "../Select";
import { Sidebar } from "../Sidebar";
import { SideRail } from "../SideRail";
import { Sparkline } from "../Sparkline";
import { Spinner } from "../Spinner";
import { Stat } from "../Stat";
import { StatusBar } from "../StatusBar";
import { StatusPill } from "../StatusPill";
import { StatusRow } from "../StatusRow";
import { SubHead } from "../SubHead";
import { SurfaceToast } from "../SurfaceToast";
import { Switch } from "../Switch";
import { Table, type Column } from "../Table";
import { TabStrip } from "../TabStrip";
import { Tabs } from "../Tabs";
import { TextInput } from "../TextInput";
import { Titlebar } from "../Titlebar";
import { Toast } from "../Toast";
import { Toolbar } from "../Toolbar";
import { Tooltip } from "../Tooltip";
import { WorkspaceSwitcher } from "../WorkspaceSwitcher";
import { WorkspaceTree } from "../WorkspaceTree";
import type { Tone } from "../tone";

const TONES: Tone[] = ["muted", "ok", "info", "accent", "warn", "sev"];

/**
 * What a cluster's refusal actually looks like on the wire — kube-rs's
 * `ApiError` Display, as `podCount` hands it to the overview. Written out in
 * full because the catalogue's job is to show the states that break on a real
 * cluster, and this is the one that used to be printed at the reader.
 */
const API_ERROR =
  'ApiError: Unauthorized: Unauthorized (Status { status: Some("Failure"), metadata: ' +
  "Some(ListMeta { continue_: None, remaining_item_count: None, resource_version: None, " +
  'self_link: None }), reason: Some("Unauthorized"), code: Some(401), message: Some("Unauthorized") })';

/**
 * A stand-in for a real icon. The kit does not depend on an icon set — callers
 * pass their own — so the catalogue brings its own shape to show the hole.
 */
function DotIcon({ size = 14, ...rest }: { size?: number; "aria-hidden"?: boolean | "true" | "false" }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" {...rest}>
      <circle cx="8" cy="8" r="5" fill="currentColor" />
    </svg>
  );
}

/** One cluster's worth of navigation, shared by the trees and the sidebar. */
const RESOURCE_NODES: ResourceNode[] = [
  {
    id: "workloads",
    label: "Workloads",
    icon: DotIcon,
    children: [
      { id: "pods", label: "Pods", icon: DotIcon, count: 412 },
      { id: "deployments", label: "Deployments", icon: DotIcon, count: 38 },
      { id: "statefulsets", label: "StatefulSets", icon: DotIcon, count: 4 },
      // Zero is a figure, not an absence: a node with no count shows none.
      { id: "cronjobs", label: "CronJobs", icon: DotIcon, count: 0 },
      { id: "jobs", label: "Jobs", icon: DotIcon },
    ],
  },
  {
    id: "network",
    label: "Network",
    icon: DotIcon,
    children: [
      { id: "services", label: "Services", icon: DotIcon, count: 51 },
      { id: "ingresses", label: "Ingresses", icon: DotIcon, count: 7 },
    ],
  },
  // Children present but empty: a group that folds onto nothing, not a leaf.
  { id: "crds", label: "Custom resources", icon: DotIcon, defaultExpanded: false, children: [] },
  { id: "events", label: "Events", icon: DotIcon, count: 214 },
];

/** What the mark editor is put back to, and what it starts from. */
const CLUSTER_MARK: MarkAppearance = {
  name: "prod-eu",
  short: "PE",
  color: "var(--accent)",
  mark: "text",
  withText: true,
};

/**
 * The kit's living catalogue, and the only visual review surface this design
 * has — there are no visual regression tests, so a component missing from here
 * is a component nobody looks at.
 *
 * Every section shows the states, not the happy path. The states are what break
 * on a real cluster: a pod over its limit, a series with no samples yet, a node
 * reporting a figure nobody designed for.
 */
export function Gallery() {
  // The inputs are controlled, so the catalogue has to hold their value; typing
  // into a component that never updates is not a working example of it.
  const [text, setText] = useState("kube-system");
  const [empty, setEmpty] = useState("");
  const [ns, setNs] = useState("kube-system");
  const [tab, setTab] = useState("pods");
  const [drawer, setDrawer] = useState(false);
  const [workspace, setWorkspace] = useState("local");
  const [openTabs, setOpenTabs] = useState([
    { id: "control", title: "Control room", sub: "prod-eu", pinned: true },
    { id: "workloads", title: "Workloads", sub: "prod-eu" },
    { id: "logs", title: "checkout-api logs", sub: "prod-eu", preview: true },
  ]);
  const [openTab, setOpenTab] = useState("workloads");
  const [boxes, setBoxes] = useState({ a: true, b: false });
  const [refresh, setRefresh] = useState("30");
  const [live, setLive] = useState(true);
  const [scope, setScope] = useState("kube-system");
  // Sorted by default so the design's active-sort caret (only column shown, no
  // funnel) is visible without a click.
  const [sort, setSort] = useState<import("../Table").TableSort | null>({
    key: "name",
    direction: "asc",
  });
  const [tableFilterKey, setTableFilterKey] = useState<string | null>(null);
  const [picked2, setPicked2] = useState<Set<string>>(new Set());
  const [picked, setPicked] = useState<string[]>([]);
  const [hiddenColumns, setHiddenColumns] = useState<ReadonlySet<string>>(new Set(["age"]));
  const [manifest, setManifest] = useState("apiVersion: v1\nkind: Pod\nmetadata:\n  name: web-1\n");
  const [dialog, setDialog] = useState<null | "plain" | "danger" | "busy">(null);
  const [modal, setModal] = useState(false);
  const [consoleOpen, setConsoleOpen] = useState(true);
  const [ask, setAsk] = useState("why is checkout-api restarting");
  const [drillStep, setDrillStep] = useState("diagnose");
  const [resource, setResource] = useState("pods");
  const [treeQuery, setTreeQuery] = useState("");
  const [sidebarQuery, setSidebarQuery] = useState("");
  const [peekWidth, setPeekWidth] = useState(260);
  const [podFilter, setPodFilter] = useState("checkout");
  const [inspectorTab, setInspectorTab] = useState("overview");
  const [railCluster, setRailCluster] = useState("prod-eu");
  const [cluster, setCluster] = useState("prod-eu");
  const [appearance, setAppearance] = useState<MarkAppearance>(CLUSTER_MARK);
  // The busy dialog is deliberately undismissable — that is the state being
  // shown — so the catalogue releases it rather than trapping whoever opened it.
  useEffect(() => {
    if (dialog !== "busy") return;
    const timer = setTimeout(() => setDialog(null), 2500);
    return () => clearTimeout(timer);
  }, [dialog]);

  return (
    <div className="kit-gallery">
      <h1>Design system</h1>

      <section>
        <h2>Badge</h2>
        <div className="kit-gallery__row">
          {TONES.map((tone) => (
            <Badge key={tone} tone={tone}>
              {tone}
            </Badge>
          ))}
        </div>
        <div className="kit-gallery__row">
          {TONES.map((tone) => (
            <Badge key={tone} tone={tone} solid>
              {tone}
            </Badge>
          ))}
        </div>
      </section>

      <section>
        <h2>Meter</h2>
        <Meter value={0} ariaLabel="empty" />
        <Meter value={42} ariaLabel="ok" />
        <Meter value={72} ariaLabel="warning" />
        <Meter value={95} ariaLabel="severe" />
        {/* A pod over its limit reports more than 100%: the bar clamps, the
            number does not. */}
        <Meter value={150} ariaLabel="over limit" />
        {/* Captioned: the number moves above the bar rather than doubling, and
            the caption is not the accessible name — the meter still needs one. */}
        <Meter value={42} ariaLabel="Node CPU" label="CPU" detail="3 of 8 cores" />
      </section>

      <section>
        <h2>Sparkline</h2>
        <Sparkline points={[3, 5, 2, 8, 6, 9, 4]} tone="ok" ariaLabel="a normal series" />
        <Sparkline points={[3, 5, 2, 8, 6, 9, 4]} tone="sev" fill={false} ariaLabel="no fill" />
        {/* One sample is where the version this came from produced NaN. */}
        <Sparkline points={[7]} tone="warn" ariaLabel="a single sample" />
        {/* The normal state of a chart that has just been opened. */}
        <Sparkline points={[]} ariaLabel="no samples yet" />
      </section>

      <section>
        <h2>Button</h2>
        <div className="kit-gallery__row">
          <Button>primary</Button>
          <Button variant="secondary">secondary</Button>
          <Button variant="outline">outline</Button>
          <Button variant="ghost">ghost</Button>
          <Button variant="danger">danger</Button>
        </div>
        <div className="kit-gallery__row">
          <Button size="xs">xs</Button>
          <Button size="sm">sm</Button>
          <Button size="default">default</Button>
          <Button size="lg">lg</Button>
        </div>
        {/* Disabled is not a rare state: half the toolbar is disabled until a
            resource is selected. */}
        <div className="kit-gallery__row">
          <Button disabled>disabled</Button>
          <Button variant="danger" disabled>
            disabled danger
          </Button>
        </div>
      </section>

      <section>
        <h2>IconButton</h2>
        <div className="kit-gallery__row">
          <IconButton icon={DotIcon} label="Logs" />
          <IconButton icon={DotIcon} label="Delete" danger />
          {/* The disabled form carries its reason, which is the whole point of
              the title override. */}
          <IconButton icon={DotIcon} label="Restart" disabled title="No pod selected" />
        </div>
      </section>

      <section>
        <h2>TextInput</h2>
        <div className="kit-gallery__row">
          <TextInput value={text} onValueChange={setText} aria-label="a filled input" />
          <TextInput
            value={empty}
            onValueChange={setEmpty}
            placeholder="namespace"
            aria-label="an empty input"
          />
          <TextInput value="bad name" onValueChange={() => {}} invalid aria-label="an invalid input" />
          <TextInput value="frozen" onValueChange={() => {}} disabled aria-label="a disabled input" />
        </div>
      </section>

      <section>
        <h2>Field</h2>
        <Field label="Namespace">
          <TextInput value={text} onValueChange={setText} />
        </Field>
        <Field label="Name" hint="Lowercase letters, numbers and dashes">
          <TextInput value={empty} onValueChange={setEmpty} />
        </Field>
        {/* The error replaces the hint rather than joining it. */}
        <Field label="Name" hint="Lowercase letters, numbers and dashes" error="Already taken">
          <TextInput value="prod" onValueChange={() => {}} invalid />
        </Field>
        {/* With an action the label cannot wrap the control; see the component. */}
        <Field label="Manifest" action={<Button size="xs">Preview</Button>}>
          <TextInput value={empty} onValueChange={setEmpty} />
        </Field>
      </section>

      <section>
        <h2>Select</h2>
        <div className="kit-gallery__row">
          <Select
            value={ns}
            onValueChange={setNs}
            options={[{ value: "default" }, { value: "kube-system" }, { value: "argocd" }]}
            aria-label="a namespace"
          />
          {/* An empty string is a real value here, not a sentinel. */}
          <Select
            value=""
            onValueChange={() => {}}
            options={[{ value: "", label: "All namespaces" }, { value: "default" }]}
            aria-label="an all-namespaces select"
          />
          {/* Nothing chosen yet: the placeholder leads and cannot be picked. */}
          <Select
            value="none"
            onValueChange={() => {}}
            options={[{ value: "a" }, { value: "b" }]}
            placeholder="Pick a context"
            aria-label="an unselected select"
          />
        </div>
      </section>

      <section>
        <h2>StatusPill</h2>
        <div className="kit-gallery__row">
          <StatusPill status="Running" kind="success" />
          <StatusPill status="Pending" kind="warning" />
          <StatusPill status="CrashLoopBackOff" kind="danger" />
          <StatusPill status="Terminating" kind="info" />
          <StatusPill status="Unknown" />
        </div>
        {/* `tinted` colours the word only where the state is bad: red
            `Degraded`, plain `Running`. Off by default — a table where every
            cell says something in colour says nothing. */}
        <div className="kit-gallery__row">
          <StatusPill status="Degraded" kind="danger" tinted />
          <StatusPill status="Progressing" kind="warning" tinted />
          <StatusPill status="Running" kind="success" tinted />
          <StatusPill status="Unknown" tinted />
        </div>
      </section>

      <section>
        <h2>StatusRow</h2>
        {/* The overview's `NOT READY` list: workloads and pods mixed in one
            list, ordered by severity rather than by kind. Not a table — no
            header, no sort, no selection. */}
        <div className="flex flex-col">
          <StatusRow
            status="Degraded"
            kind="danger"
            flagged
            name="checkout-api"
            facts={["checkout", "9/12"]}
            onActivate={() => {}}
          />
          <StatusRow
            status="CrashLoopBackOff"
            kind="danger"
            flagged
            name="checkout-api-7d9f4b8c6-x2mzp"
            facts={["checkout", "0/1"]}
            onActivate={() => {}}
          />
          <StatusRow
            status="Progressing"
            kind="warning"
            flagged
            name="payments-worker"
            facts={["payments", "4/4"]}
            onActivate={() => {}}
          />
          {/* A word longer than the verdict column pushes the name along
              rather than being cut in half, and a name longer than its row
              truncates rather than shoving the facts off the end. */}
          <StatusRow
            status="ContainerStatusUnknown"
            kind="warning"
            flagged
            name="search-indexer-59c7d4f6b8-qm2xk-a-deliberately-overlong-name"
            facts={["search", "0/1"]}
            onActivate={() => {}}
          />
          {/* Healthy: a coloured dot beside a plain grey word. The asymmetry
              is the point — a list where every row says something in colour
              says nothing. */}
          <StatusRow status="Running" kind="success" flagged={false} name="payments-api" facts={["payments", "3/3"]} />
          {/* No facts and no activation: the row draws no empty facts box and
              is not a target. */}
          <StatusRow status="Unknown" kind="neutral" flagged={false} name="orphaned-replicaset" />
        </div>
      </section>

      <section>
        <h2>Spinner</h2>
        <div className="kit-gallery__row">
          <Spinner />
          <Spinner className="size-8" />
          {/* Inline beside text is where it spends most of its life. */}
          <span className="inline-flex items-center gap-2 text-[0.8125rem]">
            <Spinner label="Fetching pods" /> Fetching pods
          </span>
        </div>
      </section>

      <section>
        <h2>ConfirmDialog</h2>
        <div className="kit-gallery__row">
          <Button size="xs" onClick={() => setDialog("plain")}>
            confirm
          </Button>
          <Button size="xs" variant="danger" onClick={() => setDialog("danger")}>
            destructive
          </Button>
          {/* In flight: both controls disabled, Escape and the overlay inert. */}
          <Button size="xs" variant="secondary" onClick={() => setDialog("busy")}>
            busy
          </Button>
        </div>
        {dialog ? (
          <ConfirmDialog
            title={dialog === "danger" ? "Delete pod?" : "Apply changes?"}
            message={
              dialog === "danger"
                ? "web-1 will be removed. This cannot be undone."
                : "The manifest will be applied to the cluster."
            }
            confirmLabel={dialog === "danger" ? "Delete" : "Apply"}
            danger={dialog === "danger"}
            busy={dialog === "busy"}
            onConfirm={() => setDialog(null)}
            onCancel={() => setDialog(null)}
          />
        ) : null}
      </section>

      <section>
        <h2>Dialog</h2>
        {/* The frame ConfirmDialog puts a question in, with the task left to
            the caller: a title, a body, and one row of controls. */}
        <div className="kit-gallery__row">
          <Button size="xs" onClick={() => setModal(true)}>
            customise
          </Button>
        </div>
        {modal && (
          <Dialog
            title="Customise kind-local"
            onClose={() => setModal(false)}
            footer={
              <>
                <Button size="sm" variant="secondary" onClick={() => setModal(false)}>
                  Reset
                </Button>
                <Button size="sm" onClick={() => setModal(false)}>
                  Done
                </Button>
              </>
            }
          >
            <div className="p-3 text-[0.8125rem] text-muted">
              Whatever the task is. This one would hold a CustomizeMark.
            </div>
          </Dialog>
        )}
      </section>

      <section>
        <h2>LoadingState</h2>
        <LoadingState />
        <LoadingState label="Loading pods" />
      </section>
      <section>
        <h2>Panel</h2>
        <Panel title="Cluster">A titled surface.</Panel>
        <Panel title="Cluster" description="Every node in the current context">
          A description under the title.
        </Panel>
        {/* A description with no title still earns a header. */}
        <Panel description="No title, still a header">Body.</Panel>
        {/* Untitled omits the header rather than ruling off an empty one. */}
        <Panel>No title at all.</Panel>
      </section>

      <section>
        <h2>Section</h2>
        {/* The other shape beside Panel: a run of flat blocks divided by
            hairline rules, which is what a detail body is made of. The first
            carries no heading, and no rule is drawn above it. */}
        <div className="card">
          <Section>
            <KV k="Replicas" v="9 ready · 12 desired" />
            <KV k="Strategy" v="RollingUpdate · surge 25% · unavailable 0" />
          </Section>
          <Section title="Conditions">
            <KV k={<StatusPill status="Available" kind="danger" tinted />} v="False · MinimumReplicasUnavailable" />
            <KV k={<StatusPill status="ReplicaFailure" kind="success" tinted />} v="False · —" />
          </Section>
          <Section title="Labels">
            <PairList breakValues pairs={[["app.kubernetes.io/name", "checkout-api"]]} />
          </Section>
        </div>
      </section>

      <section>
        <h2>Tabs</h2>
        <Tabs
          tabs={[
            { id: "pods", label: "Pods" },
            { id: "services", label: "Services" },
            { id: "events", label: "Events" },
          ]}
          active={tab}
          onChange={setTab}
          label="Resource views"
        />
        {/* The keyboard contract is the part worth checking here: the strip is
            one Tab stop, and Left/Right/Home/End move between tabs. */}
        <p className="text-[0.75rem] text-muted">showing: {tab}</p>
        {/* The segmented variant: the same control and the same keyboard
            contract, wearing the design's rounded pill instead of the window
            chrome's flat strip. It is what a detail peek draws. */}
        <Tabs
          variant="segmented"
          tabs={[
            { id: "pods", label: "Details" },
            { id: "services", label: "Containers" },
            { id: "events", label: "Events" },
          ]}
          active={tab}
          onChange={setTab}
          label="Resource views, segmented"
        />
        {/* The underline variant: what the resource FULL TAB draws — words on
            the page surface, the active one ruled in the accent. */}
        <Tabs
          variant="underline"
          tabs={[
            { id: "pods", label: "Overview" },
            { id: "services", label: "YAML" },
            { id: "events", label: "Events" },
          ]}
          active={tab}
          onChange={setTab}
          label="Resource views, underlined"
        />
      </section>

      <section>
        <h2>Drawer</h2>
        <Button size="xs" onClick={() => setDrawer((v) => !v)}>
          {drawer ? "close" : "open"} the drawer
        </Button>
        <div className="flex" style={{ height: 180 }}>
          <div className="flex-1 text-[0.75rem] text-muted">
            the list this docks beside — it shrinks rather than being covered
          </div>
          <Drawer
            open={drawer}
            title="Pod · web-1"
            onClose={() => setDrawer(false)}
            defaultWidth={320}
          >
            Drag the left edge to resize. Escape closes it.
          </Drawer>
        </div>
      </section>
      <section>
        <h2>MetricTile</h2>
        <div className="kit-gallery__row">
          <MetricTile label="Pods" value={248} />
          <MetricTile label="Restarts" value={9} tone="sev" description="last hour" />
          <MetricTile label="Nodes" value={12} tone="ok" />
          {/* The figure stays in the body colour whatever the tone: severity is
              context around the number, not the number itself. */}
          <MetricTile label="Pending" value={3} tone="warn" action={<Button size="xs">view</Button>} />
        </div>
      </section>

      <section>
        <h2>SegmentBar</h2>
        <SegmentBar
          ariaLabel="Pods: 18 running, 3 pending, 1 failed"
          segments={[
            { value: 18, tone: "ok", label: "Running" },
            { value: 3, tone: "warn", label: "Pending" },
            { value: 1, tone: "sev", label: "Failed" },
          ]}
        />
        {/* A cluster with nothing scheduled yet is the first render, not an
            edge case — it must not divide by zero into a NaN width. */}
        <p className="text-[0.75rem] text-muted">nothing scheduled yet</p>
        <SegmentBar
          ariaLabel="Empty cluster"
          segments={[
            { value: 0, tone: "ok", label: "Running" },
            { value: 0, tone: "sev", label: "Failed" },
          ]}
        />
      </section>

      <section>
        <h2>Toolbar</h2>
        <Toolbar>
          <TextInput value={ns} onValueChange={setNs} placeholder="filter" aria-label="filter" />
          <span className="flex-1" />
          <Button size="xs" variant="secondary">
            refresh
          </Button>
        </Toolbar>
      </section>

      <section>
        <h2>Screen</h2>
        {/* Bounded here; in the app it takes the full height of its pane. */}
        <div className="card overflow-hidden" style={{ height: 220 }}>
          <Screen
            title="Pods"
            eyebrow="Workloads"
            description="Everything scheduled in this namespace."
            actions={<Button size="xs">new</Button>}
          >
            <p className="text-[0.75rem] text-muted">the table goes here</p>
          </Screen>
        </div>
      </section>

      <section>
        <h2>EmptyState</h2>
        <EmptyState title="No pods" />
        <EmptyState title="No pods" hint="Nothing is scheduled in this namespace." />
        <EmptyState
          title="No pods"
          hint="Nothing is scheduled in this namespace."
          action={<Button size="xs">create pod</Button>}
        />
      </section>

      <section>
        <h2>ErrorState</h2>
        {/* The states differ in what they announce, not just how they look:
            this one is a live region, the two above are not. */}
        <ErrorState title="Could not load pods" />
        <ErrorState
          title="Could not load pods"
          detail="dial tcp 10.0.0.1:6443: connection refused"
          onRetry={() => {}}
          action={{ label: "Diagnose in Toolbox", onClick: () => {} }}
        />
        {/* The shape a classified failure takes: a sentence the reader can act
            on, with what the cluster actually said folded away under it. */}
        <ErrorState
          title="Could not list pods on prod-eu"
          detail="The cluster rejected your credentials. Your token or client certificate may have expired — refresh your kubeconfig credentials and try again."
          raw={API_ERROR}
          onRetry={() => {}}
        />
      </section>

      <section>
        <h2>RawError</h2>
        {/* Standing on its own, for a surface too narrow for a paragraph: the
            overview's Fleet rows are 286px and say one word plus this. */}
        <RawError text={API_ERROR} />
        <RawError text={API_ERROR} label="What the cluster said" />
      </section>

      <section>
        <h2>NavIcon</h2>
        {/* Takes its colour from the row it sits in, so hover and the active
            state reach it without it knowing about them. */}
        <div className="kit-gallery__row">
          <span className="inline-flex items-center gap-2 text-[0.8125rem]">
            <NavIcon icon={DotIcon} /> Pods
          </span>
          <span className="inline-flex items-center gap-2 text-[0.8125rem]" style={{ color: "var(--accent)" }}>
            <NavIcon icon={DotIcon} /> Deployments
          </span>
        </div>
      </section>
      <section>
        <h2>Combobox</h2>
        {/* Use over Select when the list is long enough to want searching. */}
        <Combobox
          value={scope}
          onValueChange={setScope}
          options={[
            { value: "kube-system" },
            { value: "default" },
            { value: "monitoring" },
            { value: "cert-manager" },
          ]}
          ariaLabel="Scope"
        />
        <p className="text-[0.75rem] text-muted">chosen: {scope}</p>
      </section>

      <section>
        <h2>MultiSelect</h2>
        {/* Stays open while toggling, so several can be picked in one visit.
            `allLabel` is how a filter says "no filter" without a sentinel
            value the caller has to invent. */}
        <MultiSelect
          options={[{ value: "default" }, { value: "kube-system" }, { value: "monitoring" }]}
          selection={picked}
          onChange={setPicked}
          allLabel="All namespaces"
          ariaLabel="Namespaces"
        />
        <p className="text-[0.75rem] text-muted">
          selected: {picked.length === 0 ? "(all)" : picked.join(", ")}
        </p>
      </section>

      <section>
        <h2>ColumnPicker</h2>
        {/* The pinned column is the row identifier: offered, but never off. */}
        <ColumnPicker
          columns={[
            { key: "name", label: "Name" },
            { key: "namespace", label: "Namespace" },
            { key: "status", label: "Status" },
            { key: "age", label: "Age" },
          ]}
          hidden={hiddenColumns}
          pinnedKey="name"
          onToggle={(key) =>
            setHiddenColumns((current) => {
              const next = new Set(current);
              if (next.has(key)) next.delete(key);
              else next.add(key);
              return next;
            })
          }
        />
        <p className="text-[0.75rem] text-muted">
          hidden: {hiddenColumns.size === 0 ? "(none)" : [...hiddenColumns].join(", ")}
        </p>
      </section>

      <section>
        <h2>CopyCommand</h2>
        {/* The other half of the pair below, and the distinction is the whole
            point of there being two: this hands the reader something to run
            themselves, so the command is the content and carries no preamble.
            Shown at a rail's width, because that is where it clips — and it
            wraps instead. */}
        <div style={{ width: 264 }}>
          <CopyCommand command="kubectl --context prod-eu get servicemonitors.monitoring.coreos.com -A -o wide" />
        </div>
      </section>

      <section>
        <h2>KubectlPreview</h2>
        {/* Not `CopyCommand`: this sits inside a confirm dialog, beside an
            action the app is about to perform, and says so. */}
        <KubectlPreview command="kubectl delete pod web-1 -n default" onCopy={() => {}} />
        {/* Not every action has a faithful one-liner; the note says so in the
            same place rather than leaving the dialog silent. */}
        <KubectlPreview note="Eviction is an API call with no kubectl verb of its own." />
      </section>

      <section>
        <h2>CodeEditor</h2>
        <div style={{ height: 200 }}>
          <CodeEditor
            value={manifest}
            onChange={setManifest}
            ariaLabel="Manifest YAML"
            fill
          />
        </div>
        {/* Completions are the caller's: the kit knows CodeMirror, not what an
            apiVersion is. */}
      </section>
      <section>
        <h2>Table</h2>
        {/* The states worth seeing: an active sort (only that column gets a
            caret — the design correction this table exists to show, #319
            follow-up), a column that opted into a filter funnel, bulk
            selection, and a row that is clickable. Virtualisation only
            engages past a threshold, so a gallery-sized list renders whole.
            Restarts is end-aligned — the design correction that right-aligns
            every numeric column (READY, RESTARTS, CPU, MEMORY, AGE) so their
            digits line up down the column. */}
        <Table
          columns={
            [
              { key: "name", header: "Name", sortable: true, filterable: true },
              { key: "phase", header: "Phase", sortable: true },
              { key: "restarts", header: "Restarts", sortable: true, align: "end" },
            ] as Column<{ name: string; phase: string; restarts: number }>[]
          }
          data={[
            { name: "web-1", phase: "Running", restarts: 0 },
            { name: "web-2", phase: "Pending", restarts: 3 },
            { name: "api-0", phase: "CrashLoopBackOff", restarts: 17 },
          ]}
          getRowKey={(r) => r.name}
          sort={sort}
          onSortChange={setSort}
          activeFilterKey={tableFilterKey}
          onActiveFilterKeyChange={setTableFilterKey}
          selection={{ selected: picked2, onChange: setPicked2 }}
          onRowClick={() => {}}
        />
        <p className="text-[0.75rem] text-muted">
          sorted: {sort ? `${sort.key} ${sort.direction}` : "(unsorted)"} · filter:{" "}
          {tableFilterKey ?? "(all columns)"} · selected:{" "}
          {picked2.size === 0 ? "(none)" : [...picked2].join(", ")}
        </p>
        {/* Empty is a state, not an absence: it says what would be here. */}
        <Table
          columns={[{ key: "name", header: "Name" }] as Column<{ name: string }>[]}
          data={[]}
          getRowKey={(r) => r.name}
          emptyText="No pods"
          emptyHint="Nothing is scheduled in this namespace."
        />
        {/* The row gestures a resource list needs: double-click or Enter on
            the focused row opens it, right-click (or Shift+F10) opens a menu
            built from that row. One tab stop for the table; the arrows move
            it. */}
        <Table
          columns={
            [
              { key: "name", header: "Name", sortable: true, filterable: true },
              { key: "phase", header: "Phase", sortable: true },
            ] as Column<{ name: string; phase: string }>[]
          }
          data={[
            { name: "web-1", phase: "Running" },
            { name: "web-2", phase: "Pending" },
            { name: "api-0", phase: "CrashLoopBackOff" },
          ]}
          getRowKey={(r) => r.name}
          onRowActivate={(row) => alert(`open ${row.name}`)}
          rowMenu={(row) => [
            { label: "Open logs", icon: DotIcon, onPick: () => alert(`logs for ${row.name}`) },
            { kind: "sep" },
            { label: "Delete pod", danger: true, onPick: () => alert(`delete ${row.name}`) },
          ]}
          rowMenuLabel="Pod actions"
        />
      </section>
      <section>
        <h2>Checkbox</h2>
        <div className="kit-gallery__row">
          <Checkbox checked={boxes.a} onChange={(v) => setBoxes((b) => ({ ...b, a: v }))} label="Include system namespaces" />
          <Checkbox checked={boxes.b} onChange={(v) => setBoxes((b) => ({ ...b, b: v }))} label="Watch for changes" />
          {/* The third state, for a header box over a partial selection — and
              the one the mock lost on the first click. */}
          <Checkbox checked={false} indeterminate onChange={() => {}} label="Select all" />
          <Checkbox checked disabled onChange={() => {}} label="Locked on" />
          <Checkbox checked={boxes.a} onChange={(v) => setBoxes((b) => ({ ...b, a: v }))} ariaLabel="Unlabelled" />
        </div>
      </section>

      <section>
        <h2>Radio</h2>
        {/* One tab stop for the group; arrows move within it, which is the
            browser's doing and the reason these are native inputs. */}
        {[
          { value: "10", label: "Every 10 seconds", hint: "Heaviest on the API server." },
          { value: "30", label: "Every 30 seconds" },
          { value: "off", label: "Never", hint: "Refresh by hand." },
        ].map((option) => (
          <Radio
            key={option.value}
            name="kit-gallery-refresh"
            checked={refresh === option.value}
            onChange={() => setRefresh(option.value)}
            label={option.label}
            hint={option.hint}
          />
        ))}
      </section>

      <section>
        <h2>Switch</h2>
        <Switch on={live} onChange={setLive} label="Live updates" hint="Stream changes as they happen." />
        <Switch on={!live} onChange={() => setLive((v) => !v)} label="Pause on error" danger />
        <Switch on={false} onChange={() => {}} label="Unavailable here" disabled />
        {/* Unlabelled still needs a name; the caller supplies one. */}
        <Switch on={live} onChange={setLive} ariaLabel="Live updates, compact" />
      </section>

      <section>
        <h2>Eyebrow</h2>
        <div className="kit-gallery__row">
          <Eyebrow>since</Eyebrow>
          <Eyebrow tone="warn">degraded</Eyebrow>
          <Eyebrow tone="sev">failing</Eyebrow>
        </div>
      </section>

      <section>
        <h2>SubHead</h2>
        {/* A heading, so the pane it labels has an outline. */}
        <SubHead>Containers</SubHead>
        <p className="text-[0.75rem] text-muted">the group this labels</p>
      </section>

      <section>
        <h2>Stat</h2>
        {/* A divided row: `.stat + .stat` rules between them. */}
        <div className="card flex">
          <Stat label="Nodes" value={12} delta="all ready" tone="ok" className="flex-1" />
          <Stat label="Pods" value="1 284" delta="3 not ready" tone="sev" className="flex-1" />
          <Stat label="Age" value="84d" className="flex-1" />
        </div>
      </section>

      <section>
        <h2>KV</h2>
        {/* A row carries its own name/value group, so one on its own is valid
            markup anywhere — which is why it is used standalone as often as
            through a list. */}
        <KV k="Status" v="Running" />
        <KV k="Image" v="nginx:1.25" mono />
      </section>

      <section>
        <h2>KVList</h2>
        <Panel title="Pod · web-1">
          <KVList
            rows={[
              ["Kind", "Pod"],
              ["Namespace", "kube-system"],
              ["Image", "nginx:1.25"],
              ["Node", "ip-10-0-1-23"],
            ]}
            mono={(v) => v.includes(":") || v.startsWith("ip-")}
          />
        </Panel>
      </section>

      <section>
        <h2>PairList</h2>
        <SubHead>Labels</SubHead>
        <PairList
          pairs={[
            ["app", "web"],
            ["app.kubernetes.io/managed-by", "Helm"],
          ]}
        />
        {/* Wide enough to read one in full. */}
        <SubHead>Annotations</SubHead>
        <PairList
          breakValues
          pairs={[["kubectl.kubernetes.io/last-applied-configuration", '{"apiVersion":"v1","kind":"Pod"}']]}
        />
      </section>

      <section>
        <h2>Mark</h2>
        <div className="kit-gallery__row">
          <Mark name="prod-eu" short="PE" size="sm" />
          <Mark name="prod-us" short="PU" color="var(--ok)" />
          <Mark name="staging" short="ST" color="var(--info)" size="lg" active />
          {/* No short text: the initials come off the name, split on the
              separators a context name actually uses. */}
          <Mark name="gke_acme-prod_europe-west1" color="var(--warn)" />
          {/* A glyph says nothing about which cluster this is, so the short text
              rides under it. */}
          <Mark name="sandbox" short="SBX" icon={DotIcon} color="var(--sev)" />
          <Mark name="lab" short="LAB" icon={DotIcon} color="var(--sev)" withBadge={false} />
          {/* A moved file or a truncated data URL: it falls through to what is
              underneath rather than leaving a broken-image glyph in the rail. */}
          <Mark name="edge-1" short="E1" imageSrc="/no-such-logo.png" />
          {/* Named by the row around it instead, so it is not announced twice. */}
          <Mark decorative name="prod-eu" short="PE" />
        </div>
      </section>

      <section>
        <h2>AgentMark</h2>
        <div className="kit-gallery__row">
          <AgentMark />
          <AgentMark size={28} />
          {/* Standing alone it names itself; beside the word "Agent" it would
              say the same thing twice. */}
          <AgentMark size={44} label="srelens agent" />
          {/* A collapsed layout hands over 0 and arithmetic upstream hands over
              a four-digit number; both are clamped to what can be drawn. */}
          <AgentMark size={0} />
          <AgentMark size={4000} />
        </div>
      </section>

      <section>
        <h2>Avatar</h2>
        <div className="kit-gallery__row">
          <Avatar name="Devesh Kumar" />
          <Avatar name="ci-bot" tone="muted" />
          {/* A double space used to yield an undefined initial, an astral
              character half a surrogate pair. */}
          <Avatar name="Ana  Maria  Ruiz" tone="ok" />
          <Avatar name="🐙 Octo Cat" tone="warn" />
          {/* Nothing to be named by, so it leaves the accessibility tree rather
              than announcing an anonymous picture. */}
          <Avatar name="   " tone="sev" />
        </div>
      </section>

      <section>
        <h2>CustomizeMark</h2>
        {/* The palette is the caller's, and every swatch is named: a hex read
            aloud names nothing. Tokens are perfectly good mark colours and no
            value at all for the native picker, which shows its own default. */}
        <div className="card overflow-hidden">
          <CustomizeMark
            value={appearance}
            onChange={setAppearance}
            colors={[
              { value: "var(--accent)", label: "Accent" },
              { value: "var(--ok)", label: "Green" },
              { value: "var(--warn)", label: "Amber" },
              { value: "var(--sev)", label: "Red" },
              { value: "var(--info)", label: "Blue" },
            ]}
            icons={[{ id: "dot", label: "Dot", icon: DotIcon }]}
            onReset={() => setAppearance(CLUSTER_MARK)}
          />
        </div>
        {/* The name emptied, which the rail cannot show. With no symbols passed
            the Symbol choice is not offered either — the kit ships none. */}
        <div className="card overflow-hidden">
          <CustomizeMark
            value={{ name: "", short: "", color: "var(--sev)", mark: "text", withText: false }}
            onChange={() => {}}
          />
        </div>
      </section>

      <section>
        <h2>Alert</h2>
        <Alert title="ip-10-0-1-23 is cordoned">Nothing new will be scheduled onto it.</Alert>
        <Alert tone="warn" title="3 pods are pending" onDismiss={() => {}}>
          Insufficient cpu on every node in prod-eu.
        </Alert>
        {/* Only sev interrupts: it is an alert region, everything else waits. */}
        <Alert tone="sev" title="Apply failed" onDismiss={() => {}}>
          admission webhook "vpa.kb.io" denied the request.
        </Alert>
        {/* Title alone — the detail is optional and its wrapper goes with it. */}
        <Alert tone="ok" title="Rollout complete" />
      </section>

      <section>
        <h2>Toast</h2>
        <div className="kit-gallery__row">
          <Toast title="Scaled checkout-api to 6" hint="prod-eu · default" onClose={() => {}} />
          <Toast tone="warn" title="Rollout is slower than usual" hint="4 of 12 pods updated" />
          {/* The glyph follows the tone: a failure drawn with a tick says one
              thing in shape and another in colour. */}
          <Toast
            tone="sev"
            title="Delete refused"
            hint={'pods "checkout-api-7d9f4" is forbidden'}
            onClose={() => {}}
          />
          <Toast tone="info" title="Nothing to apply" />
          {/* Built from state, and state is empty on the first render. */}
          <Toast title="" hint="" />
        </div>
      </section>

      <section>
        <h2>SurfaceToast</h2>
        {/* Pinned to the nearest positioned ancestor — this card — so a message
            about one pane stays with the pane the action happened in. The
            `window` anchor is the same card in the corner of the viewport. */}
        <div className="card relative overflow-hidden" style={{ height: 150 }}>
          <p className="p-3 text-[0.75rem] text-muted">the pane the scale was triggered in</p>
          <SurfaceToast title="Scaled checkout-api to 6" hint="prod-eu · default" onClose={() => {}} />
        </div>
        {/* Nothing to say draws nothing: an empty positioned box is invisible
            and still takes the clicks meant for what is under it. */}
        <SurfaceToast title="" hint="" />
      </section>

      <section>
        <h2>LiveSignal</h2>
        <div className="kit-gallery__row">
          <LiveSignal label="Streaming" tone="ok" />
          <LiveSignal label="Reconnecting" tone="warn" />
          <LiveSignal label="Stream lost" />
          {/* `label={connected && "Streaming"}` is how a caller makes it
              conditional; a bare coloured dot would say nothing at all. */}
          <LiveSignal label={false} tone="muted" />
        </div>
      </section>

      <section>
        <h2>Progress</h2>
        {/* A progressbar rather than a Meter: this only goes forward, and then
            it ends. Hence no tone — a rollout at 95% is not bad news. */}
        <Progress value={0} ariaLabel="Rollout of checkout-api, not started" />
        <Progress value={100 / 3} label="Rollout" ariaLabel="Rollout of checkout-api" />
        <Progress value={100} label="Drain" ariaLabel="Drain of ip-10-0-1-23" />
        {/* The caption is not the accessible name, and a figure past 100 clamps
            the bar while the printed number keeps the truth. */}
        <Progress value={140} label="Upload" ariaLabel="Upload of the support bundle" />
        <Progress value={-20} ariaLabel="A figure nobody designed for" />
        {/* Named by something already on screen instead of by a string of its
            own. */}
        <span id="kit-gallery-pull" className="eyebrow">
          Image pull
        </span>
        <Progress value={62} ariaLabelledBy="kit-gallery-pull" />
      </section>

      <section>
        <h2>Popover</h2>
        {/* Closed, which is how it spends most of its life: this is the trigger
            and nothing else. */}
        <div className="kit-gallery__row">
          <Popover label="Namespace filter" trigger={<span className="btn">Namespaces</span>}>
            <div className="p-2">
              <MultiSelect
                options={[{ value: "default" }, { value: "kube-system" }, { value: "monitoring" }]}
                selection={picked}
                onChange={setPicked}
                allLabel="All namespaces"
                ariaLabel="Namespaces"
              />
            </div>
          </Popover>
          {/* A panel that ends on Apply has to be able to dismiss itself, which
              is what the render prop is for. */}
          <Popover
            label="Rollout detail"
            side="top"
            align="end"
            trigger={<span className="btn">Rollout</span>}
          >
            {(close) => (
              <div className="space-y-2 p-2">
                <Progress value={45} label="Rollout" ariaLabel="Rollout of checkout-api" />
                <Button size="xs" onClick={close}>
                  Done
                </Button>
              </div>
            )}
          </Popover>
        </div>
      </section>

      <section>
        <h2>Tooltip</h2>
        <div className="kit-gallery__row">
          <Tooltip label="Restart the deployment">
            <Button size="xs" variant="secondary">
              Restart
            </Button>
          </Tooltip>
          {/* Raw content is not a tab stop, so it is given a wrapper that is
              one — a hint a keyboard cannot reach is the gap this closes. */}
          <Tooltip label="app.kubernetes.io/managed-by=Helm" side="right">
            helm
          </Tooltip>
          {/* An empty label shows no bubble rather than a bare padded
              rectangle, and does not remount the control it wraps. */}
          <Tooltip label="">
            <Button size="xs" variant="ghost">
              no hint
            </Button>
          </Tooltip>
        </div>
      </section>

      <section>
        <h2>ContextMenu</h2>
        {/* Right-click the row — or Shift+F10, which is the same gesture
            without a pointer. Closed, it is only the region it wraps. */}
        <ContextMenu
          label="Pod actions"
          items={[
            { label: "Open logs", icon: DotIcon, hint: "⌘L", onPick: () => {} },
            { label: "Port-forward", icon: DotIcon, onPick: () => {} },
            { kind: "sep" },
            { label: "Delete pod", danger: true, hint: "⌫", onPick: () => {} },
          ]}
        >
          <div className="card p-3 text-[0.8125rem]">checkout-api-7d9f4-x2k9 · prod-eu</div>
        </ContextMenu>
      </section>

      <section>
        <h2>ActionBar</h2>
        {/* Past `max` the rest fold into the menu. A blocked action is dimmed
            but still focusable, so the reason is reachable without a pointer —
            and in the menu it is written out, because opacity is not a
            message. */}
        <ActionBar
          label="Actions for checkout-api-7d9f4-x2k9"
          max={3}
          actions={[
            { id: "logs", label: "Logs", icon: DotIcon, onSelect: () => {} },
            { id: "shell", label: "Shell", icon: DotIcon, onSelect: () => {} },
            { id: "restart", label: "Restart", onSelect: () => {} },
            { id: "forward", label: "Port-forward", onSelect: () => {} },
            { id: "evict", label: "Evict", onSelect: () => {} },
            {
              id: "delete",
              label: "Delete",
              danger: true,
              disabledReason: "No delete on pods in prod-eu",
              onSelect: () => {},
            },
          ]}
          menuFooter={
            <KubectlPreview command="kubectl -n prod-eu get pod checkout-api-7d9f4-x2k9" onCopy={() => {}} />
          }
        />
        {/* An unusable `max` still leaves one action on the bar rather than an
            empty row with everything behind a menu. */}
        <ActionBar
          label="Actions for a suspended cronjob"
          max={0}
          actions={[
            { id: "resume", label: "Resume", onSelect: () => {} },
            { id: "run", label: "Run now", disabledReason: "Suspended", onSelect: () => {} },
          ]}
        />
      </section>

      <section>
        <h2>Breadcrumb</h2>
        <Breadcrumb parts={["prod-eu", "kube-system", "coredns-5d78c9869d-l8x2p"]} />
        {/* One part is still the current one, and says so with more than a
            colour. */}
        <Breadcrumb parts={["prod-eu"]} />
        {/* An empty trail renders no landmark: one that leads nowhere is worse
            than an absent one. */}
        <Breadcrumb parts={[]} />
      </section>

      <section>
        <h2>AskChip</h2>
        {/* One per row, so the question is the accessible name — otherwise a
            screen reader hears "Ask, Ask, Ask" and cannot tell the rows apart. */}
        <div className="kit-gallery__row">
          <AskChip question="why is checkout-api-7d9f4-x2k9 restarting?" onAsk={() => {}} />
          <AskChip question="explain CrashLoopBackOff on checkout-api" label="Explain" onAsk={() => {}} />
          {/* While an earlier question is still in flight. */}
          <AskChip question="what changed in prod-eu in the last hour?" onAsk={() => {}} disabled />
          {/* Nothing to ask renders nothing, rather than a tab stop that does
              nothing when it is used. */}
          <AskChip question="   " onAsk={() => {}} />
        </div>
      </section>

      <section>
        <h2>LogLine</h2>
        <div className="card overflow-hidden py-1">
          <LogLine ts="14:02:11.204" source="checkout-api" level="info" message="listening on :8080" />
          <LogLine
            ts="14:02:12.881"
            source="checkout-api"
            level="warn"
            message="upstream inventory-svc slow: 812ms"
          />
          <LogLine
            ts="14:02:13.002"
            source="istio-proxy"
            sourceTone="info"
            level="error"
            message="upstream connect error or disconnect/reset before headers"
          >
            <AskChip question="what does this istio-proxy error mean?" onAsk={() => {}} />
          </LogLine>
          {/* A level the stream spells its own way falls back rather than
              guessing; `tone` is the override for a line singled out by
              something other than its level. */}
          <LogLine ts="14:02:13.140" source="checkout-api" level="audit" message="draining connections" />
          <LogLine ts="14:02:13.900" source="kubelet" level="info" tone="sev" message="killing container: OOMKilled" />
          {/* The columns keep their boxes when they are empty, or the messages
              stop lining up with the thousand above them. */}
          <LogLine ts="14:02:14.900" message="no source, no level" />
          {/* A blank row in a stream is indistinguishable from a fault. */}
          <LogLine ts="14:02:15.001" source="checkout-api" level="debug" message="" />
        </div>
      </section>

      <section>
        <h2>DiffLines</h2>
        {/* §16's fixture: two lines changed between revision 118 and 119. Each
            is one "replace" row carrying both sides — not a delete row and an
            insert row rendered separately, which would double the hunk. */}
        <div className="card overflow-hidden">
          <DiffLines
            rows={
              [
                { tag: "same", left: "replicaCount: 12", right: "replicaCount: 12" },
                { tag: "same", left: "image:", right: "image:" },
                {
                  tag: "same",
                  left: "  repository: acme/checkout-api",
                  right: "  repository: acme/checkout-api",
                },
                { tag: "replace", left: '  tag: "118a7e"', right: '  tag: "4f2a1c"' },
                { tag: "same", left: "env:", right: "env:" },
                { tag: "replace", left: '  DB_POOL_MAX: "40"', right: '  DB_POOL_MAX: "5"' },
                { tag: "same", left: '  DB_POOL_TIMEOUT: "30s"', right: '  DB_POOL_TIMEOUT: "30s"' },
              ] satisfies DiffRow[]
            }
          />
        </div>
        {/* An empty list renders nothing — no empty frame left behind. */}
        <DiffLines rows={[]} />
      </section>

      <section>
        <h2>FilterBar</h2>
        <div className="card overflow-hidden">
          <FilterBar
            value={podFilter}
            onValueChange={setPodFilter}
            label="Filter pods"
            placeholder="name, label, node"
          >
            <Select
              value={ns}
              onValueChange={setNs}
              options={[{ value: "default" }, { value: "kube-system" }, { value: "argocd" }]}
              aria-label="Namespace to filter by"
            />
            <Checkbox
              checked={boxes.b}
              onChange={(v) => setBoxes((b) => ({ ...b, b: v }))}
              label="Only failing"
            />
          </FilterBar>
          <p className="p-3 text-[0.75rem] text-muted">the list this filters</p>
        </div>
        {/* Empty, so no clear button; and disabled while the list behind it is
            still loading. */}
        <div className="card overflow-hidden">
          <FilterBar value="" onValueChange={() => {}} label="Filter releases" placeholder="release" disabled />
        </div>
      </section>

      <section>
        <h2>ResourceTree</h2>
        {/* One Tab stop for the whole tree: the arrows walk it, Right opens a
            section and steps in, Left closes it and climbs out. While a query
            is running the folds step aside. */}
        <div className="card overflow-hidden" style={{ width: 280 }}>
          <FilterBar
            value={treeQuery}
            onValueChange={setTreeQuery}
            label="Filter the resource tree"
            placeholder="pods, jobs"
          />
          <ResourceTree
            label="Cluster resources"
            nodes={RESOURCE_NODES}
            active={resource}
            onActivate={setResource}
            query={treeQuery}
          />
        </div>
        {/* A query that matches nothing says so, with the query in it. */}
        <div className="card overflow-hidden" style={{ width: 280 }}>
          <ResourceTree
            label="Cluster resources, filtered to nothing"
            nodes={RESOURCE_NODES}
            onActivate={() => {}}
            query="ingressroutes"
          />
        </div>
        {/* The failure replaces the tree: a stale tree is worse than none. */}
        <div className="card overflow-hidden" style={{ width: 280 }}>
          <ResourceTree
            label="Cluster resources, failed"
            nodes={RESOURCE_NODES}
            onActivate={() => {}}
            error={{
              title: "Could not list resources",
              detail: "the server could not find the requested resource",
              onRetry: () => {},
            }}
          />
        </div>
        <div className="card overflow-hidden" style={{ width: 280 }}>
          <ResourceTree
            label="Cluster resources, empty"
            nodes={[]}
            onActivate={() => {}}
            emptyTitle="No resources"
            emptyHint="Nothing in this cluster answers to an API group this build knows."
          />
        </div>
      </section>

      <section>
        <h2>WorkspaceTree</h2>
        {/* Deliberately a list rather than a tree: each row is three controls,
            and Tab reaches every one of them. */}
        <div className="card overflow-hidden" style={{ width: 320 }}>
          <WorkspaceTree
            name="Acme platform"
            active={cluster}
            onActivate={setCluster}
            onConnect={() => {}}
            onDrillIn={() => {}}
            renderExpanded={(c) => (
              <ResourceTree
                label={`${c.name} resources`}
                nodes={RESOURCE_NODES}
                active={resource}
                onActivate={setResource}
              />
            )}
            clusters={[
              {
                id: "prod-eu",
                name: "prod-eu",
                chip: <Mark decorative name="prod-eu" short="PE" size="sm" />,
                detail: "eks · eu-west-1",
                count: 412,
              },
              {
                id: "prod-us",
                name: "prod-us",
                chip: <Mark decorative name="prod-us" short="PU" size="sm" color="var(--ok)" />,
                link: "connecting",
              },
              {
                id: "staging",
                name: "staging",
                chip: <Mark decorative name="staging" short="ST" size="sm" color="var(--info)" />,
                link: "disconnected",
              },
              {
                // The reason sits under the row and is tied to it, rather than
                // shouted through a live region while something else is read.
                id: "sandbox",
                name: "sandbox",
                chip: <Mark decorative name="sandbox" short="SB" size="sm" color="var(--warn)" />,
                link: "error",
                error: "dial tcp 10.0.4.7:6443: i/o timeout",
              },
            ]}
          />
        </div>
        {/* An empty workspace keeps the way out of being empty. */}
        <div className="card overflow-hidden" style={{ width: 320 }}>
          <WorkspaceTree
            name="New workspace"
            clusters={[]}
            onActivate={() => {}}
            emptyTitle="No clusters"
            emptyHint="Connect one to see what is in it."
            emptyAction={<Button size="xs">Connect a cluster</Button>}
          />
        </div>
      </section>

      <section>
        <h2>ClusterRail</h2>
        {/* Three things the design tells in colour are also told in words: a
            marker dot names itself into the mark's name, and a cluster out of
            reach is dimmed only because `unavailable` says why. */}
        <div className="card flex overflow-hidden" style={{ height: 240 }}>
          <ClusterRail
            showNames
            activeId={railCluster}
            onSelect={setRailCluster}
            onAdd={() => {}}
            /* Right-click a mark: the items are the app's vocabulary, the
               anchoring, the arrow keys and the dismissal are ContextMenu's. */
            menuFor={(item) => [
              { label: `Open ${item.name}`, onPick: () => setRailCluster(item.id) },
              { label: "Customise…", onPick: () => {} },
              { kind: "sep" },
              { label: "Remove from workspace", danger: true, onPick: () => {} },
            ]}
            items={[
              {
                id: "prod-eu",
                name: "prod-eu",
                mark: <Mark decorative name="prod-eu" short="PE" size="sm" />,
                detail: "eks 1.29 · eu-west-1",
                group: "prod",
                markers: [{ label: "Team connection", tone: "info" }],
              },
              {
                id: "prod-us",
                name: "prod-us",
                mark: <Mark decorative name="prod-us" short="PU" size="sm" color="var(--ok)" />,
                detail: "eks 1.29 · us-east-1",
                group: "prod",
                color: "var(--ok)",
                markers: [{ label: "Degraded", tone: "warn" }],
              },
              {
                id: "staging",
                name: "staging",
                mark: <Mark decorative name="staging" short="ST" size="sm" color="var(--info)" />,
                detail: "gke 1.30",
                group: "nonprod",
              },
              {
                id: "sandbox",
                name: "sandbox",
                mark: <Mark decorative name="sandbox" short="SB" size="sm" color="var(--warn)" />,
                group: "nonprod",
                unavailable: "Disconnected",
              },
            ]}
          />
          <p className="flex-1 p-3 text-[0.75rem] text-muted">looking at {railCluster}</p>
        </div>
        {/* Nothing to show, and something went wrong finding out. A stored
            width of 0 used to leave the rail invisible; it is clamped. */}
        <div className="card flex overflow-hidden" style={{ height: 120 }}>
          <ClusterRail
            label="Clusters, unreadable"
            items={[]}
            onSelect={() => {}}
            markSize={0}
            error="kubeconfig could not be read"
          />
          <p className="flex-1 p-3 text-[0.75rem] text-muted">no workspace yet</p>
        </div>
      </section>

      <section>
        <h2>ResizeHandle</h2>
        {/* The grip on its own, on the edge a docked-right pane wears it: drag
            it LEFT to widen, or focus it and press ArrowLeft. The arrows
            follow the edge, not the other handle. */}
        <div className="card flex overflow-hidden" style={{ height: 160 }}>
          <p className="flex-1 p-3 text-[0.75rem] text-muted">the view this sits beside</p>
          <div className="relative flex shrink-0 flex-col" style={{ width: peekWidth, background: "var(--surface-sunk)" }}>
            <ResizeHandle
              label="the details"
              width={peekWidth}
              minWidth={180}
              maxWidth={420}
              edge="left"
              onResize={setPeekWidth}
            />
            <p className="p-3 text-[0.75rem] text-muted">{peekWidth}px</p>
          </div>
        </div>
      </section>

      <section>
        <h2>Sidebar</h2>
        {/* Drag the right edge, or focus the handle and use the arrow keys —
            a resize a pointer can do and a keyboard cannot is not a resize. */}
        <div className="card flex overflow-hidden" style={{ height: 340 }}>
          <Sidebar
            label="Cluster navigation"
            back={{ label: "All clusters", count: 4, onClick: () => {} }}
            header={
              <div className="flex items-center gap-2">
                <Mark decorative name="prod-eu" short="PE" size="sm" />
                <div className="min-w-0">
                  <div className="truncate text-[0.8125rem] font-semibold">prod-eu</div>
                  <p className="path truncate">eks · eu-west-1</p>
                </div>
              </div>
            }
            query={sidebarQuery}
            onQueryChange={setSidebarQuery}
            footer={<StatusPill status="Connected" kind="success" />}
          >
            <ResourceTree
              label="prod-eu resources"
              nodes={RESOURCE_NODES}
              active={resource}
              onActivate={setResource}
              query={sidebarQuery}
            />
          </Sidebar>
          <p className="flex-1 p-3 text-[0.75rem] text-muted">the view this navigates — showing {resource}</p>
        </div>
        {/* Nothing to navigate: the frame stays and the middle says so. */}
        <div className="card flex overflow-hidden" style={{ height: 160 }}>
          <Sidebar
            label="Cluster navigation, empty"
            emptyTitle="No resources"
            emptyHint="Connect a cluster to see what is in it."
          />
          <p className="flex-1 p-3 text-[0.75rem] text-muted">nothing connected</p>
        </div>
      </section>

      <section>
        <h2>SideRail</h2>
        {/* The other half of the pair above: the sidebar resizes, this does
            not. No grip on the hairline, nothing to drag, one number per
            screen. The wide line in the middle is the whole reason the main
            region carries `min-w-0` — it scrolls inside its own box instead of
            shoving the rail off the edge. */}
        <div className="card flex overflow-hidden" style={{ height: 220 }}>
          <SideRail
            head="About this kind"
            width={264}
            rail={
              <>
                <Section title="Definition">
                  <KV k="Kind" v="ServiceMonitor" />
                  <KV k="Scope" v="Namespaced" />
                  <KV k="Served versions" v="v1, v1beta1" />
                  <KV k="Storage version" v="v1" />
                  <KV k="Objects" v="42" />
                </Section>
                <Section title="Fetch it yourself">
                  <KubectlPreview command="kubectl --context prod-eu get servicemonitors.monitoring.coreos.com -A -o wide" />
                </Section>
              </>
            }
          >
            <div className="scroll min-h-0 min-w-0 flex-1 p-3">
              <p className="whitespace-nowrap text-[0.75rem] text-muted">
                the main region — a table this wide scrolls here rather than widening the row
              </p>
            </div>
          </SideRail>
        </div>
      </section>

      <section>
        <h2>Inspector</h2>
        <div className="card overflow-hidden" style={{ height: 320 }}>
          <Inspector
            name="checkout-api-7d9f4-x2k9"
            subtitle="Deployment · checkout-api · prod-eu"
            flagged
            status="CrashLoopBackOff"
            statusKind="danger"
            /* The figures are drawn bare, so a value that wants a word on
               screen carries its own — the `label` is the `sr-only` term and
               is never rendered. Written the other way round this line read
               `CrashLoopBackOff  9/12  17  6m`, which is the mistake, not the
               example. `tone` mutes the age; the rest take ordinary ink. */
            facts={[
              { label: "Ready", value: "9/12 ready" },
              { label: "Restarts", value: "17 restarts", tone: "sev" },
              { label: "Age", value: "6m old", tone: "muted" },
            ]}
            actions={<IconButton icon={DotIcon} label="Open the full view" />}
            onClose={() => {}}
            tabs={[
              { id: "overview", label: "Overview" },
              { id: "containers", label: "Containers" },
              { id: "events", label: "Events" },
            ]}
            activeTab={inspectorTab}
            onTabChange={setInspectorTab}
            tabsLabel="Pod views"
            emptyLabel="No events in the last hour"
            footer={
              <>
                <Button size="xs" variant="secondary">
                  Restart
                </Button>
                <Button size="xs" variant="danger">
                  Delete
                </Button>
              </>
            }
          >
            {/* Events is deliberately empty: a pane with nothing in it is the
                state, and the panel says so rather than going blank. */}
            {inspectorTab === "overview" && (
              <KVList
                rows={[
                  ["Node", "ip-10-0-1-23"],
                  ["Image", "ghcr.io/acme/checkout-api:1.9.2"],
                  ["QoS", "Burstable"],
                  ["Last exit", "OOMKilled (137)"],
                ]}
                mono={(v) => v.includes(":") || v.startsWith("ip-")}
              />
            )}
            {inspectorTab === "containers" && (
              <PairList
                pairs={[
                  ["checkout-api", "OOMKilled · exit 137"],
                  ["istio-proxy", "Running"],
                ]}
              />
            )}
          </Inspector>
        </div>
        {/* Nothing picked yet, and no Close — a Drawer around it would own the
            only way out, and two of them is one too many. */}
        <div className="card overflow-hidden" style={{ height: 150 }}>
          <Inspector name="No pod selected" emptyLabel="Pick a row to peek at it" />
        </div>
      </section>

      <section>
        <h2>DrillCard</h2>
        <DrillCard
          title="Latency spike · checkout-api"
          headerAction={<LiveSignal label="Live" />}
          railLabel="Investigation steps"
          active={drillStep}
          onActiveChange={setDrillStep}
          steps={[
            {
              id: "signal",
              label: "Signal",
              content: <p className="text-[0.8125rem]">p99 crossed 800ms at 14:02 and has not come back.</p>,
              actions: (
                <>
                  <Button size="xs" variant="secondary">
                    Open the dashboard
                  </Button>
                  <Button size="xs" onClick={() => setDrillStep("diagnose")}>
                    Diagnose
                  </Button>
                </>
              ),
            },
            {
              id: "diagnose",
              label: "Diagnose",
              content: (
                <KVList
                  rows={[
                    ["Restarts", "17 in 6m"],
                    ["Last exit", "OOMKilled (137)"],
                    ["Limit", "512Mi"],
                  ]}
                />
              ),
              actions: (
                <>
                  <Button size="xs" variant="secondary">
                    Show the logs
                  </Button>
                  <Button size="xs" onClick={() => setDrillStep("act")}>
                    Propose a fix
                  </Button>
                </>
              ),
            },
            {
              id: "act",
              label: "Act",
              content: <KubectlPreview command="kubectl -n prod-eu set resources deploy/checkout-api --limits=memory=1Gi" onCopy={() => {}} />,
            },
          ]}
        />
        {/* No steps: the rail goes with them rather than heading an empty
            body. An `active` naming a step that is gone falls back to the
            first. */}
        <DrillCard steps={[]} active="signal" onActiveChange={() => {}} title="Nothing selected" />
      </section>

      <section>
        <h2>ConsoleDock</h2>
        {/* Controlled all the way down: the query, the open state and the
            output are the caller's. ⌘K opens it from anywhere on the page. */}
        <div className="card overflow-hidden">
          <ConsoleDock
            open={consoleOpen}
            onOpenChange={setConsoleOpen}
            value={ask}
            onValueChange={setAsk}
            onSubmit={() => setAsk("")}
            onClear={() => setAsk("")}
            mode="Agent"
            context="prod-eu / checkout-api"
            status="3 exchanges"
            placeholder="Ask about what you are looking at"
          >
            <p className="text-[0.8125rem]">
              checkout-api-7d9f4-x2k9 has restarted 17 times; the last exit was OOMKilled at 512Mi.
            </p>
          </ConsoleDock>
        </div>
        {/* A question in flight: send is withdrawn and Enter does nothing, so
            holding it down cannot queue three copies. */}
        <div className="card overflow-hidden">
          <ConsoleDock
            label="Console, busy"
            open
            busy
            value="scale checkout-api to 6"
            onOpenChange={() => {}}
            onValueChange={() => {}}
            onSubmit={() => {}}
            mode="Agent"
            context="prod-eu"
            emptyLabel="Nothing yet"
          />
        </div>
      </section>

      <section>
        <h2>Titlebar</h2>
        {/* The traffic lights are a picture of the window controls, not the
            controls: this component has no handle on the window. Whether they
            are drawn is the shell's answer, never a detection. */}
        <div className="card overflow-hidden">
          <Titlebar
            controls="macos"
            label="Window chrome, macOS"
            leading={<Breadcrumb parts={["Acme platform", "prod-eu"]} />}
            title="srelens"
            actions={
              <>
                <IconButton icon={DotIcon} label="Theme" />
                <IconButton icon={DotIcon} label="Zoom" />
              </>
            }
          />
        </div>
        <div className="card overflow-hidden">
          <Titlebar label="Window chrome, no controls" title="srelens" actions={<IconButton icon={DotIcon} label="Menu" />} />
        </div>
      </section>

      <section>
        <h2>StatusBar</h2>
        {/* Not a live region: a strip that announced every count, reconnect and
            indexing tick would be read out all day. */}
        <StatusBar
          segments={[
            { id: "ctx", label: "prod-eu", icon: DotIcon, tone: "accent", onSelect: () => {} },
            { id: "ns", label: "kube-system", onSelect: () => {} },
            { id: "watch", label: "Watching", dot: true, pulse: true, tone: "ok" },
            { id: "index", label: "Indexing", detail: "1 284", busy: true, title: "Building the resource index" },
          ]}
          end={[
            { id: "pf", label: "2 port-forwards", dot: true, tone: "info", onSelect: () => {} },
            { id: "pending", label: "3 pending", dot: true, tone: "warn", onSelect: () => {} },
            { id: "ver", label: "v0.7.2" },
          ]}
        />
        {/* Nothing pinned to the trailing edge, so no stretched hole either. */}
        <StatusBar
          label="Status, nothing connected"
          segments={[{ id: "ctx", label: "No cluster", dot: true, tone: "muted" }]}
        />
      </section>

      <section>
        <h2>ArcField</h2>
        {/* Scoped to a positioned ancestor: left alone it is `fixed inset-0`,
            the field behind the whole workspace. Two on one page each get their
            own pattern id, which is the fault the mock's `id="grid"` hid. */}
        <div className="kit-gallery__row">
          <div className="card relative overflow-hidden" style={{ height: 140, width: 260 }}>
            <ArcField className="absolute inset-0 h-full w-full" />
            <p className="relative p-3 text-[0.75rem] text-muted">one</p>
          </div>
          <div className="card relative overflow-hidden" style={{ height: 140, width: 260 }}>
            <ArcField className="absolute inset-0 h-full w-full" />
            <p className="relative p-3 text-[0.75rem] text-muted">and another</p>
          </div>
        </div>
      </section>
      <section>
        <h2>WorkspaceSwitcher</h2>
        {/* The chip is a button, not the mock's span: a workspace switch
            changes which clusters are in reach and which tabs are open, so the
            control for it has to be reachable. */}
        <WorkspaceSwitcher
          workspaces={[
            { id: "prod", name: "Production", clusters: 2, tabs: 11 },
            { id: "local", name: "Local & staging", clusters: 5, tabs: 2 },
            { id: "platform", name: "Platform", clusters: 4, tabs: 3 },
          ]}
          activeId={workspace}
          onSelect={setWorkspace}
          onRemove={() => {}}
          onCreate={() => {}}
        />
        <p className="text-[0.75rem] text-muted">in: {workspace}</p>
        {/* The last workspace offers no remove — there would be nothing left
            to be in — and one of each counts in the singular. */}
        <WorkspaceSwitcher
          workspaces={[{ id: "solo", name: "Solo", clusters: 1, tabs: 1 }]}
          activeId="solo"
          onSelect={() => {}}
          onRemove={() => {}}
        />
        {/* Nothing yet: the chip falls back to the empty label. */}
        <WorkspaceSwitcher workspaces={[]} activeId="" onSelect={() => {}} />
      </section>
      <section>
        <h2>TabStrip</h2>
        {/* The app's document tabs, not the view switcher above. One tab stop
            for the strip; arrows move focus without opening anything, because
            a tab here can be a live log stream; Enter opens; Delete closes the
            focused one, which is what gives the × a keyboard equivalent. */}
        <TabStrip
          tabs={openTabs}
          activeId={openTab}
          onSelect={setOpenTab}
          onClose={(id) => setOpenTabs((t) => t.filter((tab) => tab.id !== id))}
          onNew={() =>
            setOpenTabs((t) => [...t, { id: `tab-${t.length}`, title: "Pods", sub: "prod-eu" }])
          }
          newHint="⌘T"
          menuFor={(tab) => [
            { label: "Duplicate tab", onPick: () => {} },
            { label: tab.pinned ? "Unpin tab" : "Pin tab", onPick: () => {} },
            { kind: "sep" },
            { label: "Close tab", hint: "⌘W", danger: true, onPick: () => {} },
          ]}
        />
        <p className="text-[0.75rem] text-muted">
          open: {openTab} · {openTabs.length} tab{openTabs.length === 1 ? "" : "s"}
        </p>
        {/* The pinned tab shows a pin rather than a close, and Delete refuses
            it. Nothing open is still a strip. */}
        <TabStrip tabs={[]} activeId="" onSelect={() => {}} onNew={() => {}} />
      </section>
    </div>
  );
}
