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
  type BadgeTone,
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
import { getProbe, probeCluster, useProbes, type Probe, type ProbeState } from "../lib/probe";
import { describe } from "../lib/routes";
import { openTab, setActiveCluster, setWorkspaceClusters, useTabs } from "../lib/tabsStore";
import { glyph } from "../lib/tree";
import { latencyLabel, viaOf } from "./connections/clusterText";

/** §24's copy, verbatim, and in one place so the page and the suite quote one string. */
const HEADLINE_ONE = "Pick a cluster.";
const HEADLINE_TWO = "The room is already reading it.";
const LEDE =
  "srelens uses the credentials already in your kubeconfig and talks to the API server directly. Nothing about your clusters leaves this machine.";
const FOOTER =
  "Once a cluster is connected, ask the console about it in plain language. srelens reads the cluster to answer and sends your kubeconfig nowhere.";

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
 * The three words a probe can put on a row, and the tone each is worth.
 *
 * **No cluster is ever `healthy` or `degraded`** (decision 3). `connectCluster`
 * reports whether the API server answered; calling that a health verdict claims
 * a check nothing ran. `unread` is the absence, named as an absence — not
 * "pending" or "idle", which read as things the cluster is.
 *
 * This table is the same three pairs as `ClusterTable`'s own `STATUS`, which is
 * private to that file. It is duplicated rather than shared because promoting
 * it means editing a file under review in a parallel task; **it belongs beside
 * `latencyLabel` in `connections/clusterText.ts`** and should move there the
 * next time that file is opened. The latency formatter is imported for exactly
 * this reason — two formatters for one reading is how two surfaces start
 * disagreeing about the same number.
 */
const STATUS: Record<ProbeState, { word: string; tone: BadgeTone }> = {
  reachable: { word: "reachable", tone: "ok" },
  unreachable: { word: "unreachable", tone: "sev" },
  unread: { word: "no reading", tone: "muted" },
};

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
        <p className="text-[0.75rem] leading-relaxed text-muted">
          Written to a file of its own beside your other kubeconfigs. srelens reads it in place from
          then on.
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
  const { workspace } = useTabs();

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

  /**
   * Open a cluster: put it in this workspace, focus it, and open its overview.
   *
   * The workspace step is not a flourish. This screen lists every context on
   * the machine, including ones no workspace holds, and `setActiveCluster`
   * refuses an id the workspace does not have — so without it `Open` on exactly
   * those rows would do nothing at all, silently.
   */
  function open(context: ClusterContext) {
    const id = context.stableId;
    if (!workspace.clusters.includes(id)) {
      setWorkspaceClusters(workspace.id, [...workspace.clusters, id]);
    }
    setActiveCluster(id);
    openTab("/overview", { clusterName: context.name });
  }

  const desktop = isTauri();
  const count =
    contexts.length > 0
      ? `${contexts.length} in ${plural(fileCount(contexts), "file")}`
      : undefined;

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
          <Eyebrow className="normal-case">srelens · local-first</Eyebrow>
          <h1 className="mt-2.5 flex flex-col text-[2.75rem] font-semibold leading-[1.06] tracking-[-0.02em]">
            <span>{HEADLINE_ONE}</span>
            <span className="text-muted">{HEADLINE_TWO}</span>
          </h1>
          <p className="mt-4 max-w-[62ch] text-[0.9375rem] leading-relaxed text-muted">{LEDE}</p>
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

            {contexts.map((context) => (
              <ContextRow
                key={context.stableId}
                context={context}
                // `no reading` until the store has an answer for this cluster,
                // which is what lets every row paint before any probe lands.
                probe={probes[context.stableId] ?? UNREAD}
                onOpen={open}
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
          <p className="text-[0.8125rem] leading-relaxed text-muted">{FOOTER}</p>
        </div>
      </div>

      {pasteOpen && (
        <PasteDialog onClose={() => setPasteOpen(false)} onSaved={savedPaste} />
      )}
    </section>
  );
}
