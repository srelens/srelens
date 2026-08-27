import { useEffect, useRef, useState } from "react";
import {
  contextDisplayName,
  isTauri,
  listContexts,
  pickKubeconfigFiles,
  plural,
  saveKubeconfigFiles,
  savePastedKubeconfig,
  type ClusterContext,
} from "@srelens/core";
import {
  Badge,
  Button,
  Dialog,
  EmptyState,
  Eyebrow,
  Field,
  LoadingState,
  Mark,
  type IconComponent,
} from "@srelens/ui-kit";
import {
  getContexts,
  getKubeconfigFiles,
  setContexts,
  setKubeconfigFiles,
  useContexts,
  useContextsError,
  useContextsStatus,
} from "../lib/clusters";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { useMark } from "../lib/marks";
import { openCluster } from "../lib/openCluster";
import { getProbe, probeCluster, useProbes, type Probe } from "../lib/probe";
import { describe } from "../lib/routes";
import { glyph } from "../lib/tree";
import { STATUS, bySource, latencyLabel, viaOf } from "./connections/clusterText";

/**
 * §24's copy, in one place so the page and the suite quote one string.
 *
 * **Two deliberate departures from §24, recorded here because the file used to
 * claim "verbatim" and a future reviewer would otherwise "restore" the thing
 * that was wrong.**
 *
 * 1. **The lede and the footer branch on the platform.** §24 wrote one string
 *    each, and both are claims about WHOSE machine the kubeconfig is on —
 *    true of the desktop and false of the web build, where the kubeconfig is
 *    the server's, the clusters are read on the server's host, and web mode
 *    over-listing the server's contexts is a filed bug (#347). §24's own
 *    words appeared on the same card as {@link WEB_ONLY}'s "This build talks
 *    to a shared server". The empty-state hint below already branched for
 *    exactly this reason; these now do too.
 * 2. **§24's footer is replaced, not quoted.** Its version offered a
 *    read-only cluster, and there is no such thing in srelens — every write
 *    the agent can issue stops at a confirmation prompt whatever the cluster
 *    is, which is what `SourcesRail`'s own section says one click away. The
 *    verbatim footer would have contradicted its sibling rail.
 *
 * The replacement carries no promise of its own either. It used to say "ask
 * the console about it in plain language. srelens reads the cluster to
 * answer", and the console answers nothing in this design —
 * `shell/Console.tsx` renders "The agent is not in the new design yet". So it
 * says what srelens does do, and says plainly that the asking is not here yet.
 */
const HEADLINE_ONE = "Pick a cluster.";
const HEADLINE_TWO = "The room is already reading it.";
const LEDE_DESKTOP =
  "srelens uses the credentials already in your kubeconfig and talks to the API server directly. Nothing about your clusters leaves this machine.";
const LEDE_WEB =
  "srelens uses the credentials in the kubeconfig this server was started with, and talks to the API server directly from the server's host. The clusters listed below are the ones that server can see, not the ones on the machine you are reading this on.";
/**
 * `local-first` is the desktop's claim and only the desktop's. A build served
 * from a shared host is not local-first in any sense the word carries, and an
 * eyebrow is exactly where a reader takes a brand line as a fact about the
 * software in front of them.
 */
const EYEBROW_DESKTOP = "srelens · local-first";
const EYEBROW_WEB = "srelens · shared server";
/** Said once, in both footers: the console is not wired up in this design. */
const CONSOLE_NOT_YET =
  "Asking the console about a cluster in plain language is not in this design yet.";
const FOOTER_DESKTOP = `srelens reads each cluster directly, with the credentials already in your kubeconfig, and sends that file nowhere. ${CONSOLE_NOT_YET}`;
const FOOTER_WEB = `srelens reads each cluster directly from this server, with the credentials in the kubeconfig it was started with. ${CONSOLE_NOT_YET}`;

/**
 * Why neither door is drawn in the browser, said once — decision 2.
 *
 * A control that is simply absent teaches a reader nothing; this says what is
 * missing, where it exists, and the reason it cannot exist here. The reason is
 * not squeamishness: `savePastedKubeconfig` and the file picker both write to
 * the machine the backend runs on, which in web mode is a shared host, under
 * the server's own uid rather than the reader's.
 *
 * Deliberately worded around the two control labels: the suite proves the
 * doors are absent by looking for their words, and a sentence that quoted them
 * would make that proof unable to tell an explanation from a button.
 */
const WEB_ONLY =
  "Adding and pasting kubeconfigs happens on the desktop app. This build talks to a shared server, so a file written from here would land in that server's home directory as its own user rather than yours — it reads the kubeconfigs it was started with instead.";

/**
 * A cluster nothing has read yet.
 *
 * Module-level so the identity is stable: it stands in for every row whose
 * probe the store has no entry for, and a fresh object per render would make
 * each of those rows re-render on every notification about any other one.
 */
const UNREAD: Probe = { state: "unread" };

/**
 * How many files the rows came out of.
 *
 * **Counted from the rows, never from the stored file list** — §24's own mock
 * says `5 in 2 files` over eight rows, and the spec resolves that ambiguity in
 * favour of what is on the page. The stored list is the other candidate and it
 * is wrong twice over: web mode stores none at all (`Window.tsx` hands it
 * `[]`), and a stored path that yielded no context is not a file any row came
 * from.
 *
 * The empty string is not a file. A synthesized kubeconfig carries
 * `sourceFile: ""`, and counting that would offer the reader a file they cannot
 * go and look at.
 */
function fileCount(contexts: readonly ClusterContext[]): number {
  const paths = new Set<string>();
  for (const context of contexts) {
    if (context.sourceFile.trim() !== "") paths.add(context.sourceFile);
  }
  return paths.size;
}

/**
 * One context, as §24 draws it: the mark, the name, where it is reached
 * through, its reading and a way in.
 *
 * A component per row rather than inline JSX because it reads the cluster's
 * mark from the marks store and subscribes to it — `ClusterTable`'s
 * `ClusterCell` makes the same split for the same reason.
 */
function ContextRow({
  context,
  probe,
  onOpen,
}: {
  context: ClusterContext;
  probe: Probe;
  onOpen: (context: ClusterContext) => void;
}) {
  const id = context.stableId;
  const mark = useMark(id, context.name);
  /**
   * The name the reader gave this context, or the context's own.
   *
   * `contextDisplayName` takes a profile and ui-next has no profiles store yet,
   * so this resolves to the context name today. It is called anyway: it is the
   * one place a profile has to be handed over when that store arrives, and
   * `context.name` inlined here would hide it.
   */
  const name = contextDisplayName(context.name);
  /**
   * What the cluster is reached THROUGH: the kubeconfig it was declared in, or
   * — for a local cluster, where the file is beside the point — the tool that
   * made it and the endpoint it listens on. `viaOf` is the connections table's
   * own answer to that question, imported rather than restated.
   *
   * **A deviation from §24, recorded rather than left for a reviewer to
   * "restore".** §24's second line is the RAW CONTEXT STRING — the kubeconfig's
   * own `contexts[].name`, on the reasoning that a reader recognises it. That
   * needs a profiles store to be worth anything: without one, `name` above is
   * already the raw context string, so the row would print it twice. `viaOf`
   * answers the question the second line is actually for — where this cluster
   * comes from — and when a profiles store arrives the two lines diverge
   * naturally, the first becoming the reader's own name for the cluster.
   */
  const via = viaOf(context);
  const status = STATUS[probe.state];
  const latency = latencyLabel(probe);

  return (
    // `min-w-0` on the row itself, not only inside it. A flex item's implicit
    // `min-width: auto` refuses to shrink below its content, so without this
    // the caps below never engage: a 70-character kubeconfig path inside an
    // 860px card pushes the badge and the control off the end of the card.
    // Eight defects on this migration, and jsdom sees none of them — hence the
    // class assertions in the suite.
    <div
      data-testid="connect-context"
      className="flex min-w-0 items-center gap-3 rounded-tile border border-rule bg-sunk px-3 py-2.5"
    >
      <Mark
        name={mark.name}
        short={mark.short}
        color={mark.color}
        size="lg"
        // The row names the cluster twice already — the line beside the mark
        // and the control at the end of the row.
        decorative
        withBadge={mark.withText}
        icon={mark.mark === "icon" && mark.icon ? glyph(mark.icon) : undefined}
        imageSrc={mark.mark === "image" ? mark.imageSrc : undefined}
      />
      <div data-testid={`connect-text-${id}`} className="flex min-w-0 flex-1 flex-col">
        {/* `block` is what makes `truncate`'s `overflow: hidden` apply at all —
            it does nothing to an inline box — and `max-w-` is what caps the
            intrinsic contribution that `white-space: nowrap` would otherwise
            set to the whole string. */}
        <span
          data-testid={`connect-name-${id}`}
          className="block max-w-[420px] truncate text-[0.9375rem] font-medium"
        >
          {name}
        </span>
        {via !== "" && (
          <span
            data-testid={`connect-detail-${id}`}
            className="path block max-w-[420px] truncate"
            // The whole string for the row whose line is clipped. The same
            // string that is on screen, so the hover hides nothing.
            title={via}
          >
            {via}
          </span>
        )}
      </div>
      {/* The reading and the control keep their own width whatever the name
          does: a clipped path is recoverable by widening the window, a clipped
          badge or a clipped button is not. */}
      <div className="flex shrink-0 items-center gap-2.5">
        {latency !== null && (
          <span data-testid={`connect-latency-${id}`} className="path num">
            {latency}
          </span>
        )}
        <span data-testid={`connect-status-${id}`}>
          <Badge tone={status.tone}>{status.word}</Badge>
        </span>
        <Button
          type="button"
          size="sm"
          variant="secondary"
          // Named for the cluster, unlike the connections table's `Open` — that
          // control sits in a row whose first cell a screen reader announces
          // around it, and this one stands at the end of a card row with three
          // neighbours that say the same word.
          aria-label={`Open ${name}`}
          onClick={() => onOpen(context)}
        >
          Open
        </Button>
      </div>
    </div>
  );
}

/** One of §24's two doors: a full-width tile with a glyph, a label and an arrow. */
function GhostRow({
  label,
  icon: Glyph,
  busy,
  onClick,
}: {
  label: string;
  icon: IconComponent;
  busy?: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className="ghost" aria-busy={busy || undefined} onClick={onClick}>
      <span className="flex min-w-0 items-center gap-2">
        <Glyph size={14} aria-hidden="true" />
        {label}
      </span>
      {/* Decorative: the label already says where the row goes, and the kit's
          own `.ghost .arrow` rule is what animates it. */}
      <span className="arrow" aria-hidden="true">
        →
      </span>
    </button>
  );
}

/**
 * §24's second door: a kubeconfig pasted rather than browsed for.
 *
 * Its own component so the paste, the write in flight and the reason a write
 * refused live and die with the dialog rather than lingering in the screen's
 * state after it closes.
 */
function PasteDialog({
  onClose,
  onSaved,
}: {
  onClose: () => void;
  onSaved: (path: string) => Promise<void>;
}) {
  const [yaml, setYaml] = useState("");
  const [busy, setBusy] = useState(false);
  /** Why the paste could not be written, when it could not. */
  const [failure, setFailure] = useState<unknown>(null);

  const ready = yaml.trim() !== "";

  async function save() {
    // Guarded here as well as on the control: a disabled button is not a
    // contract, and an empty file written into the kubeconfig list is a path
    // every later `listContexts` carries around for nothing.
    if (!ready || busy) return;
    setBusy(true);
    setFailure(null);
    let path: string;
    try {
      // The text exactly as it was pasted — not trimmed. Trailing whitespace is
      // the pasting reader's business and YAML's, not this dialog's.
      //
      // `undefined` for the name, explicitly: core names the file itself, and
      // §24's door asks for no name. Passing the argument keeps the seam
      // visible for the day the design wants one.
      path = await savePastedKubeconfig(yaml, undefined);
    } catch (error) {
      // The dialog stays open with the paste still in it. A reader who lost a
      // pasted kubeconfig to a failed write has to go and find it again.
      setFailure(error);
      setBusy(false);
      return;
    }
    await onSaved(path);
  }

  return (
    <Dialog
      title="Paste a context"
      maxWidth={560}
      onClose={onClose}
      footer={
        <>
          <Button type="button" variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            disabled={!ready || busy}
            onClick={() => void save()}
          >
            Save
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 p-3">
        {failure !== null && (
          <FailureAlert tone="sev" title="Could not save that context" error={failure} />
        )}
        {/* The hint is a sibling of the field rather than its `hint` prop.
            `Field` renders that prop INSIDE the `<label>` it wraps the control
            in, so a hint there joins the control's accessible name: the
            textarea would announce as "Kubeconfig YAML Written to a file of its
            own beside…" — the label plus a paragraph. The label is the name;
            this is a remark beside it. */}
        <Field label="Kubeconfig YAML">
          <textarea
            value={yaml}
            onChange={(e) => setYaml(e.target.value)}
            rows={12}
            spellCheck={false}
            className="w-full rounded-md border px-2 py-1 font-mono text-[0.75rem] leading-relaxed outline-none"
            style={{ background: "var(--surface-sunk)", borderColor: "var(--rule)" }}
          />
        </Field>
        {/* **Where the file actually goes.** This said "beside your other
            kubeconfigs" and it does not go there: `savePastedKubeconfig` ->
            `files.rs`'s `save_pasted_kubeconfig` writes to
            `app_config_dir()/kubeconfigs/<stem>-<ts>.yaml` — srelens's own
            folder — while the reader's kubeconfig is `~/.kube/config`. A
            reader who wanted to find, edit or delete that context went to
            `~/.kube` and found nothing. The frozen classic app words the same
            operation correctly (`SettingsView.tsx`: "Saved securely in the
            srelens app configuration directory"); the migration turned a true
            sentence into a false one. */}
        <p className="text-[0.75rem] leading-relaxed text-muted">
          Written to a file of its own in the srelens app configuration folder, not into your own
          kubeconfig. srelens reads it in place from then on.
        </p>
      </div>
    </Dialog>
  );
}

/**
 * `/connect` — §24's screen: the door a reader with no clusters lands on, and
 * what `Add connection` opens.
 *
 * **Full-bleed, with no `Screen` around it.** Every other screen in this
 * package opens with the kit's toolbar; this one's headline IS its head, and a
 * 44px "Pick a cluster." under a 32px strip repeating the route's title would
 * be the page introducing itself twice. The route's title is still what names
 * the region, so the tab strip and the landmark cannot drift.
 *
 * **Nothing is listed on arrival.** `Window` lists the contexts at boot and
 * every screen reads that one answer, so this draws immediately rather than
 * re-asking for a list the window already has — and, more to the point, a
 * listing of its own would paint over the reader's own picked file a moment
 * after they picked it. The two doors and `Try again` are the only things that
 * list, and each writes its answer back through `setContexts` so the rail, the
 * status bar and every other screen see the list this one is drawing.
 *
 * **Three properties this screen is careful about**, each of which has shipped
 * as a defect on some screen in this migration:
 *
 * 1. The count is of what is listed — see {@link fileCount}. `5 in 2 files`
 *    over eight rows is §24's own mock, and both halves here come out of the
 *    same array the rows do. It is absent whenever there are no rows: a count
 *    of nothing asserted as a fact is the same fault as a count that disagrees
 *    with its rows.
 * 2. A probe never blocks a row. Every cluster is drawn `no reading` on the
 *    first paint and each reading arrives on its own, so twenty clusters do not
 *    queue and one that never answers holds up none of the others.
 * 3. Neither door is drawn where it cannot work, and the reason is said once —
 *    see {@link WEB_ONLY}. `Toolbox.tsx:188` asks the same platform question
 *    the same way.
 */
export function Connect({ route }: { route: string }) {
  /** The routes table's own title, so the landmark and the tab strip agree. */
  const title = describe(route).title;

  const contexts = useContexts();
  const status = useContextsStatus();
  const listError = useContextsError();
  const probes = useProbes();

  /** A listing asked for by the reader, still out. */
  const [busy, setBusy] = useState(false);
  /** Why a kubeconfig file could not be added, when one could not. */
  const [addError, setAddError] = useState<unknown>(null);
  const [pasteOpen, setPasteOpen] = useState(false);

  /**
   * Which listing is the current one — `Helm`'s and `Connections`' `listSeq`,
   * for the same reason. A reader who picks a file and then pastes a context
   * must be left looking at the second answer whatever order the two come back
   * in; without this the first listing's late answer paints over the second's.
   */
  const listSeq = useRef(0);

  /**
   * Read every cluster on the list, each on its own.
   *
   * **Nothing is awaited in series and nothing gates the render.** The rows are
   * drawn the moment the contexts exist, every one of them `no reading`, and
   * each reading arrives as its own store notification. A cluster that never
   * answers leaves its own row saying so and delays no other row — which is
   * also why the probes are not started one after another: on a laptop
   * kubeconfig holding an old kind cluster and a VPN-only production one, a
   * serial read would spend a timeout per unreachable cluster before the
   * reachable ones said anything at all.
   *
   * Only the unread ones. Arriving here respects what the store already knows —
   * `Window` probes the workspace's clusters at launch, and re-reading a
   * cluster whose answer is already in hand is a round trip for nothing.
   *
   * **No in-flight guard of this screen's own.** `probeCluster` holds one, and
   * it has to be the one that does: `Window` probes the same clusters and
   * cannot see a ref of ours, so a second read started while the first is still
   * out is two unordered writes to a module store neither caller can un-write.
   * A second call for a cluster already being read joins that read.
   */
  useEffect(() => {
    for (const context of contexts) {
      if (getProbe(context.stableId).state !== "unread") continue;
      // `probeCluster` never rejects — it folds a transport failure into an
      // unreachable reading — so there is nothing here to catch.
      void probeCluster(context);
    }
  }, [contexts]);

  /**
   * List the contexts again, and write the answer back to the store.
   *
   * A fresh array deliberately, so the effect above re-runs even when the
   * listing came back with the same contexts. A listing that FAILED keeps the
   * rows already on screen: a refresh that could not be made took nothing away
   * from the reader, and the store installs the list and the reason in one
   * write so no render catches one without the other.
   */
  async function reload() {
    const seq = ++listSeq.current;
    setBusy(true);
    const outcome = await listContexts(getKubeconfigFiles());
    // Superseded. Dropped whole — the list and the reason it is short are one
    // fact, and installing half of a stale answer is worse than none of it.
    if (seq !== listSeq.current) return;
    setBusy(false);
    setContexts([...(outcome.contexts ?? getContexts())], outcome.error ?? "");
  }

  /**
   * Remember a file the backend must know about, and list again.
   *
   * Three writes, and all three are needed: `saveKubeconfigFiles` is what makes
   * the file survive a restart, `setKubeconfigFiles` is what every core call in
   * this window reads (the backend cannot build a client for a context from a
   * file it has not been told about), and the listing is what puts that file's
   * contexts on the screen.
   */
  async function remember(paths: readonly string[]) {
    const next = [...new Set([...getKubeconfigFiles(), ...paths])];
    saveKubeconfigFiles(next);
    setKubeconfigFiles(next);
    await reload();
  }

  async function addFile() {
    setAddError(null);
    let picked: string[];
    try {
      picked = await pickKubeconfigFiles();
    } catch (error) {
      // Through `describeError` where it is shown, like every other failure
      // here. Nothing is written when the picker itself refused.
      setAddError(error);
      return;
    }
    // A cancelled picker is not a failure and has nothing to say.
    if (picked.length === 0) return;
    await remember(picked);
  }

  async function savedPaste(path: string) {
    // Closed before the listing rather than after it: the dialog has done its
    // work, and leaving it over the card until a slow listing returns hides the
    // rows the reader pasted the context to see.
    setPasteOpen(false);
    await remember([path]);
  }

  const desktop = isTauri();
  const count =
    contexts.length > 0
      ? `${contexts.length} in ${plural(fileCount(contexts), "file")}`
      : undefined;

  /**
   * The rows, in the order `/connections` puts the same clusters in.
   *
   * **Not `listContexts`' own order, and that is the whole change.** This card
   * listed raw, so a laptop whose kubeconfig declares a kind cluster between two
   * remote contexts drew them interleaved here and grouped one click away on
   * §6's table — two orders for one set, on two screens a reader moves between
   * in a single gesture. `bySource` is the table's own grouping, imported rather
   * than restated: a second sort written here is exactly how the two screens
   * would drift apart again.
   *
   * No group HEADING goes with it. §6's table draws one per group because it has
   * a `Source` column to head; this card is one flat list of eight rows at most
   * in the ordinary case, and a label over each half of it would be chrome the
   * first screen a reader ever sees does not need. The order is what the two
   * screens have to agree on.
   */
  const listed = bySource(contexts, (context) => context.isLocal);

  const Sparkle = Icons.ask;

  return (
    // A landmark with the route's own name on it: this screen draws no `h1` in
    // a toolbar, so the region is what a screen reader has to navigate by.
    <section
      aria-label={title}
      className="scroll h-full min-h-0"
      // §24's decorative background, which is the treatment the classic connect
      // surface already wears (`styles.css`'s `.fl-landing`): one accent bloom
      // off the top-left corner over the canvas, in the new design's tokens.
      style={{
        background:
          "radial-gradient(circle at 15% 8%, color-mix(in srgb, var(--accent) 7%, transparent), transparent 30%), var(--canvas)",
      }}
    >
      <div
        data-testid="connect-column"
        className="rise mx-auto flex min-w-0 max-w-[860px] flex-col gap-6 px-6 py-12"
      >
        <header className="min-w-0">
          {/* `normal-case`: the label voice is uppercase, and the brand is
              lowercase everywhere — "SRELENS" is a spelling srelens does not
              have. The tracking and the mono face are the design's; only the
              transform gives way. */}
          <Eyebrow className="normal-case">{desktop ? EYEBROW_DESKTOP : EYEBROW_WEB}</Eyebrow>
          <h1 className="mt-2.5 flex flex-col text-[2.75rem] font-semibold leading-[1.06] tracking-[-0.02em]">
            <span>{HEADLINE_ONE}</span>
            <span className="text-muted">{HEADLINE_TWO}</span>
          </h1>
          <p className="mt-4 max-w-[62ch] text-[0.9375rem] leading-relaxed text-muted">
            {desktop ? LEDE_DESKTOP : LEDE_WEB}
          </p>
        </header>

        <div className="card min-w-0">
          <div className="card-head">
            <span className="card-title">Contexts found</span>
            {count !== undefined && (
              <span data-testid="connect-count" className="eyebrow shrink-0">
                {count}
              </span>
            )}
          </div>

          <div className="flex min-w-0 flex-col gap-2 p-2.5">
            {addError !== null && (
              // `sev`: the reader asked for a file to be added and it was not.
              // This is not a remark about the rows under it.
              <FailureAlert
                tone="sev"
                title="Could not add that kubeconfig file"
                error={addError}
              />
            )}

            {contexts.length === 0 && status === "loading" && (
              <LoadingState label="Listing your contexts" />
            )}

            {contexts.length === 0 && status === "failed" && (
              <FailureState
                title="Could not read your kubeconfig"
                error={listError}
                retryLabel="Try again"
                onRetry={() => void reload()}
              />
            )}

            {contexts.length === 0 && status === "loaded" && (
              <EmptyState
                title="No contexts in your kubeconfig"
                hint={
                  desktop
                    ? "srelens read your kubeconfig and found nothing to connect to. Point it at another file, or paste one below."
                    : "srelens read the kubeconfig this server was started with and found nothing to connect to."
                }
              />
            )}

            {listed.map((context) => (
              <ContextRow
                key={context.stableId}
                context={context}
                // `no reading` until the store has an answer for this cluster,
                // which is what lets every row paint before any probe lands.
                probe={probes[context.stableId] ?? UNREAD}
                onOpen={openCluster}
              />
            ))}

            {desktop ? (
              <>
                <GhostRow
                  label="Add a kubeconfig file"
                  icon={Icons.add}
                  busy={busy}
                  onClick={() => void addFile()}
                />
                <GhostRow
                  label="Paste a context"
                  icon={Icons.copy}
                  onClick={() => setPasteOpen(true)}
                />
              </>
            ) : (
              <p className="px-1 py-1 text-[0.8125rem] leading-relaxed text-muted">{WEB_ONLY}</p>
            )}
          </div>
        </div>

        <div
          data-testid="connect-footer"
          className="flex min-w-0 items-start gap-2.5 rounded-card border border-accent-line bg-accent-wash px-3.5 py-3"
        >
          <span className="mt-px shrink-0 text-accent">
            <Sparkle size={14} aria-hidden="true" />
          </span>
          <p className="text-[0.8125rem] leading-relaxed text-muted">
            {desktop ? FOOTER_DESKTOP : FOOTER_WEB}
          </p>
        </div>
      </div>

      {pasteOpen && (
        <PasteDialog onClose={() => setPasteOpen(false)} onSaved={savedPaste} />
      )}
    </section>
  );
}
