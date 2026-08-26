import { useMemo, useState } from "react";
import { type DiffRow, type HelmRevision, helmStatus } from "@srelens/core";
import {
  Alert,
  Button,
  Checkbox,
  CodeEditor,
  CopyCommand,
  DiffLines,
  Dialog,
  Field,
  SubHead,
  Switch,
  Tabs,
  TextInput,
} from "@srelens/ui-kit";
import { type HelmOpKind, startHelmOperation } from "../../lib/helmOps";

/** §A.5's width, in px. */
const WIDTH = 620;

/**
 * Everything the four operations can be told to do, as one value.
 *
 * The dialog's fields collapse to this and nothing else reads the fields, so
 * there is exactly one description of the operation in flight — see
 * {@link helmArgv}.
 */
export interface HelmPlan {
  kind: HelmOpKind;
  release: string;
  namespace: string;
  /** The chart ref, for the two operations that take one. */
  chart: string;
  /** `--version`, blank for whatever the repo calls latest. */
  chartVersion: string;
  /** Rollback's target revision; `null` until there is one. */
  revision: number | null;
  atomic: boolean;
  wait: boolean;
}

/**
 * The helm argv this plan runs as — the ONE derivation.
 *
 * **The store composes no commands.** `startHelmOperation` hands `args`
 * straight to `helm`, and nothing on either side of the wire checks that
 * `args[0]` agrees with `kind`; this array is the whole of what the cluster
 * will be told to do. The `Equivalent command` the dialog prints comes from
 * {@link helmCommand}, which calls this function rather than restating it — two
 * hand-maintained copies of a destructive command is how a screen ends up
 * showing one thing and running another.
 *
 * **No `--kube-context`.** The backend runs helm against a kubeconfig written
 * for this context alone, with `current-context` pinned to it, and that file
 * carries the context's IN-FILE name — which is not always the name srelens
 * shows, because duplicate context names across merged kubeconfigs are
 * disambiguated for display. A `--kube-context` built from the display name
 * would fail to resolve on exactly the machines that need it most. The cluster
 * is named in words beside the command instead.
 *
 * **No `--values` either.** The backend writes the values body to a temporary
 * file and appends `--values <path>` itself, and a path that does not exist
 * yet is not a flag this side can honestly print.
 */
export function helmArgv(plan: HelmPlan): string[] {
  const scope = ["--namespace", plan.namespace];
  const version = plan.chartVersion.trim();
  const flags = [
    ...(version ? ["--version", version] : []),
    ...(plan.atomic ? ["--atomic"] : []),
    ...(plan.wait ? ["--wait"] : []),
  ];
  switch (plan.kind) {
    case "install":
      return ["install", plan.release, plan.chart.trim(), ...scope, ...flags];
    case "upgrade":
      return ["upgrade", plan.release, plan.chart.trim(), ...scope, ...flags];
    case "rollback":
      return [
        "rollback",
        plan.release,
        ...(plan.revision === null ? [] : [String(plan.revision)]),
        ...scope,
      ];
    case "uninstall":
      return ["uninstall", plan.release, ...scope];
  }
}

/** Characters a shell leaves alone; anything else earns quotes. */
const BARE = /^[A-Za-z0-9_@%+=:,./-]+$/;

/** One argv element as a shell would have to be given it. */
function quoted(arg: string): string {
  if (arg !== "" && BARE.test(arg)) return arg;
  return `'${arg.replace(/'/g, `'\\''`)}'`;
}

/**
 * The plan as a line the reader can run themselves — {@link helmArgv}, quoted.
 *
 * Derived, never written twice. Break the argv and this line breaks with it,
 * which is the point.
 */
export function helmCommand(plan: HelmPlan): string {
  return ["helm", ...helmArgv(plan)].map(quoted).join(" ");
}

/**
 * The revision a rollback should land on by default: the newest one, other
 * than the one running now, that helm does not report as failed or as still
 * in flight.
 *
 * `deployed` is read from core: `helmStatus` classifies helm's own vocabulary,
 * and `success` is the one health that means this revision is up right now.
 *
 * `superseded` is named here, and ONLY `superseded`, because core's `neutral`
 * cannot be trusted as a positive answer. That bucket holds `superseded`,
 * `uninstalled` AND every status word this build has never heard of —
 * `helmStatus` documents the fallback as "it might be a failure this table has
 * no name for". Accepting the bucket wholesale would make a helm status added
 * tomorrow, or an `uninstalled` revision left behind by
 * `helm uninstall --keep-history`, the DEFAULT target of a rollback, under a
 * hint promising it is the newest one helm does not report failed. So the one
 * neutral word that actually means "this was deployed, then replaced" is named,
 * and the unknown ones are left out. That is not a status-word table: nothing
 * here renames or tones anything, and the health still comes from core.
 */
export function lastGoodRevision(
  history: readonly HelmRevision[],
  current: number | undefined,
): HelmRevision | null {
  const candidates = history
    .filter((r) => r.revision !== current)
    .filter((r) => helmStatus(r.status).health === "success" || r.status === "superseded");
  if (candidates.length === 0) return null;
  return candidates.reduce((best, r) => (r.revision > best.revision ? r : best));
}

export interface HelmOpDialogProps {
  /** Which of the four this is. */
  kind: HelmOpKind;
  /** The cluster it runs in — a kubeconfig context NAME. */
  context: string;
  namespace: string;
  /** The release being installed, upgraded, rolled back or removed. */
  release: string;
  /** Where the chart field starts, for install and upgrade. */
  chart?: string;
  /** Where the chart version field starts; blank means whatever is latest. */
  chartVersion?: string;
  /** The values body the editor opens on. Sent only for install and upgrade. */
  values?: string;
  /** The release's revisions — rollback's target and the hint under it. */
  history?: readonly HelmRevision[];
  /** The revision running now: the one a rollback moves away from. */
  revision?: number;
  /**
   * The rendered-manifest diff, when the caller has one. Rendering a chart is
   * a round trip this dialog does not make; the screen that opens it does.
   */
  diff?: readonly DiffRow[];
  /** Extra kubeconfigs to put on helm's KUBECONFIG. */
  extraKubeconfigs?: string[];
  /** Cancel, escape, the header's control, and a submitted operation. */
  onClose: () => void;
  /** The store's id for the operation just started, for a caller that follows it. */
  onStarted?: (id: number) => void;
}

/** §A.5's title, and the word on its button. */
const TITLE: Record<HelmOpKind, string> = {
  install: "Install",
  upgrade: "Upgrade",
  rollback: "Roll back",
  uninstall: "Uninstall",
};

/**
 * §A.5's `<Op> <release>` — the one place srelens changes a Helm release.
 *
 * **Two gates, and they are deliberately different sizes.** Uninstall demands
 * the release name typed out, character for character: it is the only
 * irreversible thing in this migration, and a dialog whose destructive button
 * is one click from the button that opened it is a dialog that gets clicked
 * through. Rollback asks once, with a checkbox, and asks for no typing at all —
 * it is recoverable (roll forward again), and it is reached when something is
 * already broken and the reader is in a hurry. Classic gated neither.
 *
 * **The uninstall alert states the general truth and fetches nothing.** §A.5
 * writes "Twelve pods, one Service, one Ingress and two ConfigMaps", which is
 * the design's fixture: nothing here has counted this release's objects, and
 * this build does not count them. What it says instead is true of every
 * release — every object in it goes, and PVCs survive unless the chart marks
 * them for deletion. A made-up count on the one irreversible screen in the app
 * is the worst possible place to be confidently wrong.
 *
 * **Submitting closes the dialog.** `helm upgrade --wait` runs for minutes, and
 * a modal held open for the duration stops the reader watching the pods it is
 * restarting. The operation goes to `helmOps`, which owns the output, the
 * status and the failure from there — which is why nothing here catches
 * anything: `startHelmOperation` never throws, and a refused start is a failed
 * row carrying its own reason, already through `describeError`.
 */
export function HelmOpDialog({
  kind,
  context,
  namespace,
  release,
  chart: initialChart = "",
  chartVersion: initialVersion = "",
  values: initialValues = "",
  history = [],
  revision: current,
  diff,
  extraKubeconfigs,
  onClose,
  onStarted,
}: HelmOpDialogProps) {
  const takesChart = kind === "install" || kind === "upgrade";

  const suggested = useMemo(() => lastGoodRevision(history, current), [history, current]);

  const [chart, setChart] = useState(initialChart);
  const [chartVersion, setChartVersion] = useState(initialVersion);
  const [values, setValues] = useState(initialValues);
  const [panel, setPanel] = useState("values");
  const [atomic, setAtomic] = useState(false);
  const [wait, setWait] = useState(false);
  const [targetText, setTargetText] = useState(suggested ? String(suggested.revision) : "");
  const [acknowledged, setAcknowledged] = useState(false);
  /**
   * The typed confirmation, compared to the release name EXACTLY.
   *
   * Not trimmed, not lower-cased. A trailing space is what a paste leaves
   * behind, and a reader who pasted the name is precisely the one who has not
   * read it; the whole value of this gate is that it takes a moment of
   * attention, and every softening of the comparison gives that moment back.
   */
  const [typed, setTyped] = useState("");

  const target = revisionOf(targetText);

  const plan: HelmPlan = {
    kind,
    release,
    namespace,
    chart,
    chartVersion,
    revision: target,
    atomic,
    wait,
  };

  /** Does the argv have everything it needs to be a command at all? */
  const complete = takesChart ? chart.trim() !== "" : kind === "rollback" ? target !== null : true;
  /** Has the reader passed this operation's gate? */
  const confirmed =
    kind === "uninstall" ? typed === release && release !== "" : kind === "rollback" ? acknowledged : true;
  const ready = complete && confirmed;

  function submit() {
    if (!ready) return;
    const body = takesChart && values.trim() !== "" ? values : undefined;
    // Fired and left to run: the store owns what happens next, and the reader
    // gets the screen back. `startHelmOperation` never throws.
    void startHelmOperation({
      kind,
      release,
      namespace,
      context,
      args: helmArgv(plan),
      ...(extraKubeconfigs ? { extraKubeconfigs } : {}),
      ...(body === undefined ? {} : { values: body }),
    }).then((id) => onStarted?.(id));
    onClose();
  }

  return (
    <Dialog
      title={`${TITLE[kind]} ${release}`}
      maxWidth={WIDTH}
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant={kind === "uninstall" ? "danger" : "primary"}
            size="sm"
            disabled={!ready}
            onClick={submit}
          >
            {kind === "rollback" && target !== null ? `Roll back to ${target}` : TITLE[kind]}
          </Button>
        </>
      }
    >
      <div className="flex min-w-0 flex-col gap-3 p-3">
        {kind === "uninstall" && (
          <Alert tone="sev" title="This removes every object in the release">
            {/* No counts. See this component's note: srelens has not asked the
                cluster what is in this release, so it says what is true of any
                release rather than what would sound specific. */}
            Everything helm created for it goes, and nothing here can undo it.
            Persistent volume claims are kept unless the chart marks them for
            deletion.
          </Alert>
        )}

        {kind === "rollback" && (
          <Alert tone="warn" title="Rolling back replaces what is running now">
            The target revision's chart and values are applied over the current
            one, which stays in the release's history — so you can roll forward
            again.
          </Alert>
        )}

        {takesChart && (
          <div className="grid min-w-0 grid-cols-2 gap-x-3">
            <Field label="Chart" className="min-w-0">
              <TextInput value={chart} onValueChange={setChart} placeholder="bitnami/nginx" />
            </Field>
            {/* No hint here: the kit's `Field` renders one INSIDE the label,
                so a hint becomes part of the control's accessible name — the
                placeholder says the same thing without renaming the field. */}
            <Field label="Chart version" className="min-w-0">
              <TextInput value={chartVersion} onValueChange={setChartVersion} placeholder="18.3.0" />
            </Field>
          </div>
        )}

        {takesChart && (
          <div className="min-w-0">
            <Tabs
              variant="segmented"
              label="Panel"
              active={panel}
              onChange={setPanel}
              tabs={[
                { id: "values", label: "Values" },
                { id: "diff", label: "Rendered diff" },
              ]}
            />
            <div className="mt-2 min-w-0">
              {panel === "values" ? (
                <CodeEditor
                  value={values}
                  onChange={setValues}
                  language="yaml"
                  ariaLabel="Values"
                  minHeight={140}
                  maxHeight={220}
                />
              ) : diff && diff.length > 0 ? (
                <DiffLines rows={[...diff]} />
              ) : (
                <span className="text-[0.75rem] text-muted">
                  srelens has not rendered this chart, so there is nothing to
                  compare yet.
                </span>
              )}
            </div>
          </div>
        )}

        {takesChart && (
          <div className="flex min-w-0 flex-col gap-2">
            <Switch
              on={atomic}
              onChange={setAtomic}
              label="atomic"
              hint="Roll the release back itself if this one does not come up."
            />
            <Switch
              on={wait}
              onChange={setWait}
              label="wait"
              hint="Hold the operation open until every object reports ready."
            />
          </div>
        )}

        {kind === "rollback" && (
          <>
            <Field
              label="Target revision"
              className="min-w-0"
              hint={
                suggested
                  ? `Revision ${suggested.revision} is the newest one helm does not report failed; it reads ${helmStatus(suggested.status).word}.`
                  : "srelens has no history for this release, so name the revision yourself."
              }
            >
              <TextInput value={targetText} onValueChange={setTargetText} type="number" />
            </Field>
            {/* Asked once, and never typed out. See this component's note. */}
            <Checkbox
              checked={acknowledged}
              onChange={setAcknowledged}
              label={
                target === null
                  ? `Yes, roll ${release} back.`
                  : `Yes, roll ${release} back to revision ${target}.`
              }
            />
          </>
        )}

        {kind === "uninstall" && (
          <Field label={`Type ${release} to confirm`} className="min-w-0">
            <TextInput value={typed} onValueChange={setTyped} autoFocus />
          </Field>
        )}

        <div className="min-w-0">
          <SubHead variant="caps">Equivalent command</SubHead>
          <div className="mt-1 min-w-0">
            {complete ? (
              <CopyCommand command={helmCommand(plan)} />
            ) : (
              <span className="text-[0.75rem] text-muted">
                {takesChart ? "Name a chart to see it." : "Name a revision to see it."}
              </span>
            )}
          </div>
          {/* The context is not a flag on the command — see `helmArgv` — so the
              cluster it runs against is said in words instead. */}
          <div className="mt-1 break-words text-[0.75rem] text-muted">
            srelens runs this against {context}.
          </div>
        </div>
      </div>
    </Dialog>
  );
}

/** A revision the field holds, or null when it holds nothing helm could use. */
function revisionOf(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  return Number.isInteger(n) && n > 0 ? n : null;
}
