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
  /**
   * `--reuse-values`, and upgrade's alone.
   *
   * **Helm's default is not "keep what is there".** `helm upgrade <rel>
   * <chart>` with no values body applies the CHART's defaults over the release,
   * which silently throws away every value it was installed with. Whenever
   * srelens could not read those values back, this is what stops that: helm
   * keeps the release's own values and merges anything sent with `--values`
   * over them. It rides WITH a body rather than instead of one.
   */
  reuseValues: boolean;
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
      return [
        "install",
        plan.release,
        plan.chart.trim(),
        ...scope,
        // Classic has always sent this, and helm's own default is to fail an
        // install into a namespace that does not exist yet — which is the
        // ordinary case for a first install. Install only: the other three
        // operate on a release that is already somewhere.
        "--create-namespace",
        ...flags,
      ];
    case "upgrade":
      return [
        "upgrade",
        plan.release,
        plan.chart.trim(),
        ...scope,
        // Only upgrade: an install has no earlier values to reuse, and helm
        // rejects the flag outright.
        ...(plan.reuseValues ? ["--reuse-values"] : []),
        ...flags,
      ];
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

/**
 * helm's own release-name rule, copied from `ValidateReleaseName`: a DNS-1123
 * name, lower case, at most 53 characters.
 *
 * Checked here rather than left to helm because the reader is looking at the
 * field now, and a rejected name comes back as a failed operation in the strip
 * a minute later — by which time the dialog, and the values typed into it, are
 * gone. Nothing about it is a srelens house rule: every clause is helm's, and
 * the 53 is helm's own `maxReleaseNameLen`.
 */
const RELEASE_NAME = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$/;
const MAX_RELEASE_NAME = 53;

/** Why helm would refuse this release name, or null when it would take it. */
export function releaseNameError(name: string): string | null {
  if (name === "") return "A release needs a name.";
  if (name.length > MAX_RELEASE_NAME) {
    return `helm allows ${MAX_RELEASE_NAME} characters; this is ${name.length}.`;
  }
  if (!RELEASE_NAME.test(name)) {
    return "Lower-case letters, digits, - and . only, starting and ending with a letter or digit.";
  }
  return null;
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
  /** Where the release lives — and, for install, where its own field starts. */
  namespace: string;
  /**
   * The release being upgraded, rolled back or removed.
   *
   * In install mode this is only where the `Release name` FIELD starts — the
   * reader names the release, and everything downstream reads the field. Empty
   * is the ordinary case there and nowhere else: uninstall's gate compares
   * what was typed to this prop and guards `release !== ""`, so an empty name
   * is a gate that can never be passed.
   */
  release: string;
  /** Where the chart field starts, for install and upgrade. */
  chart?: string;
  /** Where the chart version field starts; blank means whatever is latest. */
  chartVersion?: string;
  /** The values body the editor opens on. Sent only for install and upgrade. */
  values?: string;
  /**
   * Why the caller has no current values to open on — already through
   * `describeError`, because it is shown to the reader as it arrives.
   *
   * Present means "this release HAS values and srelens could not read them",
   * which is a different fact from an empty body and must never be rendered as
   * one: an upgrade with no values body applies the chart's defaults over
   * whatever the release was installed with. Set, an upgrade says so and adds
   * `--reuse-values` so helm keeps them. Absent — the ordinary case — the
   * editor's contents are the whole of what will be applied.
   */
  valuesUnavailable?: string;
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
  valuesUnavailable,
  history = [],
  revision: current,
  diff,
  extraKubeconfigs,
  onClose,
  onStarted,
}: HelmOpDialogProps) {
  const takesChart = kind === "install" || kind === "upgrade";
  /**
   * **Install names its own release, which §A.5 does not draw.**
   *
   * The design lists Chart and Chart version and nothing else, because the
   * mock only ever depicts one install — of a fixture called `new-release`.
   * Built as drawn, srelens could create exactly one release per cluster: the
   * second `helm install new-release` is refused, "cannot re-use a name that
   * is still in use". The namespace is here for the same reason — a fixed one
   * installs everything into `default` — and `--create-namespace` with it,
   * which classic has always sent, because helm's own default is to fail an
   * install into a namespace that does not exist yet.
   */
  const takesName = kind === "install";

  const suggested = useMemo(() => lastGoodRevision(history, current), [history, current]);
  /**
   * What the hint says when {@link lastGoodRevision} offers nothing.
   *
   * Three different facts, and only one of them is "srelens has no history".
   * Saying that one for all three was false in the two cases that matter most:
   * a release whose history is entirely failed revisions HAS a history, and
   * telling the reader otherwise hides the very thing they came to find out.
   * The rest of the degrade path is already honest and stays — no target is
   * filled in, and the command area says there is nothing to show rather than
   * printing a command nobody asked for.
   */
  const noDefault = useMemo(() => {
    if (history.length === 0) {
      return "srelens has no history for this release, so name the revision yourself.";
    }
    if (history.every((r) => r.revision === current)) {
      return "This release has only the revision running now, so there is nothing earlier to return to.";
    }
    return "No earlier revision is safe to offer: each one is failed, unfinished, gone, or in a state this build does not recognise. Name the revision yourself.";
  }, [history, current]);

  // Install's own two fields. Seeded from the props, and read by everything
  // downstream from there: the title, the argv, and what the store is told.
  const [name, setName] = useState(release);
  const [ns, setNs] = useState(namespace);
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

  /**
   * Ask helm to keep what the release already has.
   *
   * Upgrade only, and only when the caller says the current values could not
   * be read: with them in hand the editor IS the answer, and adding the flag
   * would merge the release's values under an editor the reader can see and
   * trust — including a line they deliberately deleted, which would then not
   * go away.
   */
  const reuseValues = kind === "upgrade" && valuesUnavailable !== undefined;

  /** Why helm would refuse the name in the field, or null. Install only. */
  const nameError = takesName ? releaseNameError(name) : null;

  const plan: HelmPlan = {
    kind,
    release: takesName ? name : release,
    namespace: takesName ? ns.trim() : namespace,
    chart,
    chartVersion,
    revision: target,
    atomic,
    wait,
    reuseValues,
  };

  /**
   * Why the argv is not a command helm could run yet, or null when it is one.
   *
   * One derivation for two jobs — what the command area says instead of a
   * command, and whether the button is dead — so the two can never disagree
   * about whether this dialog is ready. The name's own reason is under its
   * field rather than here: that is where the reader is looking.
   */
  const incomplete: string | null =
    takesName && nameError !== null
      ? "Name the release to see it."
      : takesName && ns.trim() === ""
        ? // Not "helm will use the current namespace": the field says which
          // namespace this goes to, and an empty one makes it say nothing.
          "Name a namespace to see it."
        : takesChart && chart.trim() === ""
          ? "Name a chart to see it."
          : kind === "rollback" && target === null
            ? "Name a revision to see it."
            : null;
  const complete = incomplete === null;
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
      // The plan's, not the props': install's release and namespace are the
      // reader's fields, and the store must follow the same pair the argv did.
      release: plan.release,
      namespace: plan.namespace,
      context,
      args: helmArgv(plan),
      ...(extraKubeconfigs ? { extraKubeconfigs } : {}),
      ...(body === undefined ? {} : { values: body }),
    }).then((id) => onStarted?.(id));
    onClose();
  }

  /**
   * §A.5's `<Op> <release>` — and, before an install has been named, the word
   * on the control that opened it. "Install " with nothing after it reads as a
   * rendering fault.
   */
  const heading = takesName
    ? name.trim() === ""
      ? "Install chart"
      : `Install ${name}`
    : `${TITLE[kind]} ${release}`;

  return (
    <Dialog
      title={heading}
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

        {reuseValues && (
          <Alert tone="warn" title="srelens could not read this release's current values">
            {/* The one thing that must not happen here is a blank editor with
                no explanation: an upgrade sending no values applies the
                CHART's defaults over the release. `--reuse-values` is on the
                command below, so helm keeps what the release has. */}
            {valuesUnavailable} The editor below therefore starts empty and
            helm is told <code className="code">--reuse-values</code>: the
            release keeps the values it has, and anything typed here is merged
            over them.
          </Alert>
        )}

        {takesName && (
          /* `min-w-0` on the cells as well as the grid: a grid item's implicit
             `min-width: auto` refuses to shrink below its content, and a long
             release name would push this 620px dialog wider than it says it
             is. Seven defects on this migration, none of them visible in
             jsdom. */
          <div className="grid min-w-0 grid-cols-2 gap-x-3">
            <Field
              label="Release name"
              className="min-w-0"
              // Only once there is something to be wrong about: an error under
              // an untouched empty field is a scolding for not having typed
              // yet. The button is dead either way.
              error={name !== "" && nameError !== null ? nameError : undefined}
            >
              <TextInput
                value={name}
                onValueChange={setName}
                placeholder="checkout-api"
                invalid={name !== "" && nameError !== null}
                autoFocus
              />
            </Field>
            <Field label="Namespace" className="min-w-0">
              <TextInput value={ns} onValueChange={setNs} placeholder="default" />
            </Field>
          </div>
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
                  : noDefault
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
              <span className="text-[0.75rem] text-muted">{incomplete}</span>
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
