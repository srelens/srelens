import { useId } from "react";
import { contextDisplayName, plural } from "@srelens/core";
import { Badge, Button, Section, cx } from "@srelens/ui-kit";
import type { ClusterRow } from "./ClusterTable";
import { joined, latencyLabel, viaOf } from "./clusterText";

/** §6's rail width, in px — see `SideRail`'s note on why a width is a number. */
const WIDTH = 292;

export interface SourcesRailProps {
  /** Every cluster the screen is drawing, the same rows the table gets. */
  rows: readonly ClusterRow[];
  /** The stored kubeconfig paths, from `loadKubeconfigFiles()`. */
  files: readonly string[];
  /**
   * Browse for a kubeconfig file to add.
   *
   * **Absent means there is no filesystem to browse, and the rail says so.**
   * A callback rather than an `isTauri()` call of its own (which is what
   * `Toolbox.tsx:188` does): the screen already knows which platform it is on,
   * and a component that reads the platform itself cannot be rendered both
   * ways from a fixture. The reason it is absent is printed once — see
   * {@link SourcesRail}.
   */
  onAddFile?: () => void;
  /**
   * Whether the reader is looking at the desktop app, so the two section
   * headings can say WHOSE machine they are about.
   *
   * **A fact, passed in like {@link SourcesRailProps.onAddFile} and for the
   * same reason** — a component that calls `isTauri()` itself cannot be drawn
   * both ways from a fixture. Deliberately NOT derived from `onAddFile`: this
   * file's own note refuses to read a platform claim out of a UI callback, and
   * the two can legitimately diverge (a caller that has a file picker but no
   * local clusters, or the reverse).
   *
   * Absent means the browser, which is the conservative default: the headings
   * then say "the server's host", which is true of a shared server and merely
   * pedantic if a reader happens to be running one on their own machine — the
   * other way round the rail states a falsehood about where somebody's
   * cluster runs.
   */
  desktop?: boolean;
  className?: string;
}

/** One line of §6's first section: a stored kubeconfig, and what came out of it. */
interface FileRow {
  path: string;
  /** How many contexts in `rows` name this file. */
  contexts: number;
  /** How many of them are the kubeconfig's current context. */
  inUse: number;
}

/**
 * The files this rail lists: every stored path, plus every path a context
 * actually came from.
 *
 * **Both directions matter, and each covers a hole the other leaves.**
 *
 * A stored path with no contexts is KEPT, because a path can be deleted, moved
 * or emptied between runs while `saveKubeconfigFiles` still holds it — and a
 * stored path that silently vanishes from this list is how a reader concludes
 * they never added it.
 *
 * A path that no one stored is ADDED, because `files` is not the whole truth
 * about where clusters are read from: `Window.tsx` reads
 * `loadKubeconfigFiles()` only on the desktop and hands web mode `[]`, so
 * without this the web build would draw an empty "where your clusters come
 * from" section beside a table listing twelve of them. It exposes nothing new
 * — the table's own `Via` column already prints the same path for every one of
 * those rows.
 *
 * A context with no `sourceFile` at all (a synthesized cluster) contributes no
 * row: an empty path drawn as a file is a line the reader cannot act on.
 *
 * Order is `files`' own, then first appearance in `rows`, so the list a reader
 * curated keeps the order they curated it in. Dedupe is by exact path, which
 * is what both sides are keyed on; two spellings of one file (a symlink, a
 * trailing slash) are two entries, because this file has no filesystem to ask
 * and inventing a normalisation would merge two paths that may genuinely
 * differ.
 */
function fileRows(files: readonly string[], rows: readonly ClusterRow[]): FileRow[] {
  const byPath = new Map<string, FileRow>();
  const seed = (path: string) => {
    if (path.trim() === "" || byPath.has(path)) return;
    byPath.set(path, { path, contexts: 0, inUse: 0 });
  };

  for (const file of files) seed(file);
  for (const { context } of rows) seed(context.sourceFile);

  for (const { context } of rows) {
    const entry = byPath.get(context.sourceFile);
    if (!entry) continue;
    entry.contexts += 1;
    if (context.isCurrent) entry.inUse += 1;
  }

  return [...byPath.values()];
}

/**
 * §6's `<n> contexts · <n> in use`.
 *
 * **`in use` is the kubeconfig's `current-context`** — the one context every
 * other tool on this machine will pick up if nobody says otherwise. Core marks
 * at most one context current across every merged file
 * (`crates/kube/src/context_resolve.rs:141-145` takes the first match and
 * stops), so this number is 0 or 1 per file and that IS the fact, not a
 * miscount.
 *
 * It is deliberately NOT "how many probes came back reachable". That is a
 * reading, the table already words it (`reachable`), and calling the same fact
 * "in use" here would be two words for one thing — which is how two panes
 * start disagreeing.
 *
 * `0 in use` is dropped for a file with no contexts: "0 contexts · 0 in use"
 * says the second half twice, and the sentence under it is what explains the
 * first.
 */
function countLine({ contexts, inUse }: FileRow): string {
  return joined([plural(contexts, "context"), contexts > 0 ? `${inUse} in use` : undefined]);
}

/**
 * §6's right-hand rail: where srelens reads each cluster from.
 *
 * **Driven entirely by props**, like the table beside it — no fetch, no store,
 * no `isTauri()`. The screen owns the contexts, the probes, the stored file
 * list AND the platform, so every state of this rail is reachable from a
 * fixture; see {@link SourcesRailProps.desktop} for why the platform is a prop
 * rather than a call.
 *
 * Three sections, and the third is not the one the mock draws.
 *
 * **1. The kubeconfig files.** A path, and how many contexts came out of it.
 * A file that yields none is still listed — see {@link fileRows}.
 *
 * **2. The local clusters.** Name, `via` and the round trip, from the same two
 * helpers the table uses (`./clusterText`), so a reading cannot read one way
 * here and another way six inches to the left. No reading means no badge; it
 * never means `0 ms`.
 *
 * **3. What the agent may reach — ONE SENTENCE, AND NO BADGE PER CLUSTER.**
 * §6's mock draws `read + write` / `read only` per cluster. There is no such
 * distinction anywhere in srelens, and this was established rather than
 * assumed: every mutating capability carries `requires_confirm`, enforced as a
 * build-time invariant against the real registry
 * (`crates/mcp/src/completeness.rs:36-45`), and `tools/call` blocks on that
 * confirmation before invoking anything (`crates/mcp/src/stdio.rs:156-183`).
 * `isLocal` appears nowhere in that chain — not in `policy.rs`, not in
 * `consent_kind`, not in the desktop's `PromptUser::confirm`. No cluster is
 * read-only in srelens's own logic: the only outright refusals are the
 * cluster's own RBAC (per verb, not per cluster) and `WEB_DENIED_CAPABILITIES`
 * (per platform, not per cluster). Two badges that differ by nothing would
 * imply a setting the reader can change, and there is none — so the section
 * says the true thing once. The suite asserts the section draws no `.badge`
 * and names no cluster, so restoring the mock's shape fails a test rather than
 * quietly contradicting a comment.
 *
 * The web build's capability denylist IS a real distinction, and it is
 * deliberately not mentioned here even though `desktop` now says which build
 * this is: the denylist is the SERVER's configuration, not a consequence of
 * being served over HTTP, and the Toolbox says it where it is checked. What
 * `desktop` may be spent on is whose machine a thing is on — nothing more. It
 * is also why the platform is its own prop and not read off `onAddFile`, which
 * says only "this caller can open a file picker": deriving "and therefore the
 * server refuses nine capabilities" from a UI callback would be inventing a
 * platform claim out of a button.
 */
export function SourcesRail({ rows, files, onAddFile, desktop, className }: SourcesRailProps) {
  const headId = useId();
  const sources = fileRows(files, rows);
  const local = rows.filter((row) => row.context.isLocal);
  /**
   * Whose machine the two headings are about.
   *
   * Both said "this machine" / "this laptop" unconditionally, and in web mode
   * neither is the reader's: the kubeconfig is the one the server was started
   * with, and a kind or minikube cluster it declares runs on the SERVER's host
   * — possibly a shared box in another building. The `Kubeconfig` heading sat
   * directly above the paragraph telling that same reader files are added "on
   * the desktop, where srelens has a filesystem to browse", so the rail
   * contradicted itself two lines apart.
   *
   * The agent section's sentence is left alone: "whether the cluster runs on
   * this laptop or across the internet" is a claim about the GATE being the
   * same near and far, not about where any cluster runs.
   */
  const filesHead = desktop ? "Kubeconfig · on this machine" : "Kubeconfig · on the server's host";
  const localHead = desktop ? "Local · runs on this laptop" : "Local · runs on the server's host";

  return (
    // The frame is fixed and does not scroll; the body does, exactly as
    // `ReleasePane` splits them. A `scroll` on the aside would put the head out
    // of reach of a reader half way down a long file list.
    <aside
      aria-labelledby={headId}
      className={cx("side-rail", className)}
      style={{ width: WIDTH }}
    >
      <div id={headId} className="pane-head">
        Sources
      </div>

      {/* `min-w-0` here as well as on every row below: this body is the flex
          child that a 70-character filesystem path would otherwise refuse to
          shrink below, and `min-width: auto` is what makes the truncation in
          each row inert. Eight defects on this migration, none of them visible
          in jsdom — hence the class assertions in the suite. */}
      <div data-slot="rail-body" className="side-rail-body min-w-0">
        <Section title={filesHead}>
          <div className="flex min-w-0 flex-col gap-2.5">
            {sources.length === 0 ? (
              <p className="text-[0.8125rem] leading-snug text-muted">No kubeconfig files.</p>
            ) : (
              sources.map((source) => (
                <div
                  key={source.path}
                  data-testid="source-file"
                  className="flex min-w-0 flex-col gap-0.5"
                >
                  {/* `block` is what makes `truncate`'s `overflow: hidden`
                      apply at all, and `max-w-` caps the intrinsic width that
                      `white-space: nowrap` would otherwise set to the whole
                      path. 252px is the rail's 292 less the section's inset.
                      The `title` carries the full path, which is the same
                      string, so nothing is hidden behind the hover. */}
                  <span
                    className="block max-w-[252px] truncate font-mono text-[0.75rem]"
                    title={source.path}
                  >
                    {source.path}
                  </span>
                  <span className="path text-faint">{countLine(source)}</span>
                  {source.contexts === 0 && (
                    /* **Four causes, and the likeliest one first.** This named
                       three and left out the one a reader hits right after
                       `Add a kubeconfig file`: a file srelens could not parse.
                       `resolve_contexts` skips an unreadable or invalid
                       kubeconfig silently (`Kubeconfig::read_from(path).ok()`),
                       so the path stays on the stored list and still yields
                       nothing. Telling someone who has just picked a file that
                       it may have been deleted sends them after the wrong
                       problem. */
                    <p className="text-[0.8125rem] leading-snug text-muted">
                      No contexts came from this file. srelens may not have been able to read it as
                      a kubeconfig, or it may have been moved, deleted or emptied since it was
                      added.
                    </p>
                  )}
                </div>
              ))
            )}

            {onAddFile ? (
              <div className="flex">
                <Button size="xs" variant="secondary" onClick={onAddFile}>
                  Add
                </Button>
              </div>
            ) : (
              /* Said once, for the section, rather than as a disabled button —
                 the Toolbox's absent install column is the precedent. A
                 control that is not there needs its reason on the screen; a
                 reader who cannot see one concludes srelens lost the feature. */
              <p className="text-[0.8125rem] leading-snug text-muted">
                Kubeconfig files are added on the desktop, where srelens has a filesystem to
                browse.
              </p>
            )}

            {/* §6's footnote, verbatim. It is the whole answer to "what does
                srelens do with my credentials", and it is answered before the
                reader has to ask. */}
            <p className="text-[0.8125rem] leading-snug text-muted">
              srelens reads these files in place and connects to the API server directly. Nothing is
              copied anywhere.
            </p>
          </div>
        </Section>

        {/* Nothing is drawn when no cluster runs here: a heading over an empty
            box is a thing the reader has to look at and dismiss, and §6's own
            rail has no "you have no local clusters" state. */}
        {local.length > 0 && (
          <Section title={localHead}>
            <div data-testid="sources-local" className="flex min-w-0 flex-col gap-2.5">
              {local.map(({ context, probe }) => {
                const via = viaOf(context);
                const reading = latencyLabel(probe);
                return (
                  <div key={context.stableId} className="flex min-w-0 items-start gap-2">
                    {/* No `max-w-` on these two, unlike the path above: this
                        column is `flex-1 min-w-0` inside a frame of a FIXED
                        width, so its content is already bounded and a cap
                        would clip a name early whenever the reading beside it
                        is absent. The table needed caps because an auto-layout
                        table cell sizes to its content instead. */}
                    <div className="flex min-w-0 flex-1 flex-col">
                      <span className="block truncate font-medium" title={context.name}>
                        {contextDisplayName(context.name)}
                      </span>
                      {via !== "" && (
                        <span className="path block truncate font-mono" title={via}>
                          {via}
                        </span>
                      )}
                    </div>
                    {reading !== null && (
                      // `shrink-0`: the reading is short and fixed, and the
                      // name and endpoint are what absorb a long line. Toned
                      // `muted` deliberately — a round trip is a network
                      // duration, not a health verdict, and the table puts no
                      // colour on it either.
                      <span data-slot="local-reading" className="shrink-0">
                        <Badge tone="muted">{reading}</Badge>
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          </Section>
        )}

        <Section title="What the agent may reach">
          <div data-testid="sources-agent">
            <p className="text-[0.8125rem] leading-snug text-muted">
              Every change the agent makes to any cluster on this list stops at a confirmation
              prompt first — the same gate whether the cluster runs on this laptop or across the
              internet.
            </p>
          </div>
        </Section>
      </div>
    </aside>
  );
}
