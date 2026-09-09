import { PALETTE, SYMBOLS, symbolFor } from "../lib/markSymbols";
import { useOrderedContexts } from "../lib/contextOrder";
import { useEffect, useState } from "react";
import type { ClusterContext } from "@srelens/core";
import {
  Button,
  ClusterRail,
  CustomizeMark,
  Dialog,
  Mark,
  NavIcon,
  type ClusterRailItem,
  type ContextMenuItem,
} from "@srelens/ui-kit";
import { friendly } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { getMark, resetMark, setMark, useEditableMark } from "../lib/marks";
import { useInfos } from "../lib/probe";
import { openTab, setActiveCluster, setWorkspaceClusters, useActiveCluster, useTabs } from "../lib/tabsStore";
import { useWorkspaceView } from "../lib/workspace";

export interface RailProps {
  contexts: ClusterContext[];
  /** Opens /connect. The rail knows the gesture, the window knows the route. */
  onConnect: () => void;
  /**
   * Why the cluster list could not be read — a kubeconfig that failed to
   * parse, usually. The saved cluster ids are kept and drawn once their
   * contexts come back, but until then the rail would otherwise be silently
   * empty with no way to tell "nothing configured" from "couldn't read it".
   */
  error?: string;
}

/** An image mark is inlined into the settings file, so it has to stay small. */
const MAX_IMAGE_BYTES = 64 * 1024;

/**
 * The strip of cluster marks down the edge of the window, the menu each one
 * answers, and the dialog that edits how it looks.
 *
 * Everything drawn is the kit's; what lives here is the four stores the kit is
 * not allowed to see. The workspace says which clusters and in what order, the
 * contexts resolve those ids to something with a name and a server, the link
 * states say which are reachable, and the marks say what each one looks like.
 * Items are built from all four and handed over already ordered — a rail that
 * fetched any of it for itself would be a rail with one call site (#320).
 *
 * A cluster id with no matching context is skipped rather than drawn as a
 * placeholder. `reconcile` already drops ids whose context has gone, so this
 * only covers the window between a kubeconfig changing and the store catching
 * up, and a mark for a cluster that is not there is worse than one mark fewer.
 *
 * What the menu offers, and what it deliberately does not. The design draws a
 * `Disconnect` in the destructive slot, and there is nothing behind that verb:
 * core connects (`connectCluster`, which is a probe) and deletes a context out
 * of the kubeconfig on disk (`deleteContext`, which is a far larger act than
 * this menu implies), and the `disconnected` link state is derived from a
 * probe's answer and re-derived by the next one — writing it by hand would be
 * a claim the next probe erases. So the item keeps the name of what it really
 * does, which is to drop the cluster from this workspace; a red row wired to
 * the nearest available verb is worse than an honest one. `Connection details`
 * opens `/connections`, which is a route this shell already knows and titles,
 * though the screen behind it is still the Placeholder.
 *
 * The marks and the probes are read once for the whole list rather than once
 * per cluster: the number of clusters changes between renders, so a hook per
 * item would be a hook count that changes with the list, which React refuses.
 * `useInfos` is the probe store's whole-record snapshot, which exists for this.
 * The marks have no such hook, so the subscription rides on the `useEditableMark` call
 * the dialog's editor needs anyway — that hook subscribes whatever id it is
 * asked about, so it re-renders this rail on any mark change and the items then
 * read the plain `getMark` beside it.
 */
export function Rail({ contexts, onConnect, error }: RailProps) {
  const { workspace } = useTabs();
  const active = useActiveCluster();
  const { links } = useWorkspaceView();
  const [editing, setEditing] = useState<string | null>(null);

  const byId = new Map(contexts.map((c) => [c.stableId, c]));
  const target = editing === null ? null : (byId.get(editing) ?? null);

  // One subscription each, standing in for the per-item hooks — see above.
  const value = useEditableMark(target?.stableId ?? "", target?.name ?? "");
  const infos = useInfos();

  // A context can leave while its dialog is open — a kubeconfig rewritten under
  // the app. The dialog is already gone by then, since `target` cannot resolve;
  // this forgets which cluster it was about, so a context that comes back does
  // not bring a dialog nobody asked for back with it.
  const stale = editing !== null && !byId.has(editing);
  useEffect(() => {
    if (stale) setEditing(null);
  }, [stale]);

  const ordered = useOrderedContexts(workspace.clusters.flatMap(id => byId.get(id) ? [byId.get(id)!] : []));
  const items: ClusterRailItem[] = [];
  for (const ctx of ordered) {
    const id = ctx.stableId;
    const mark = getMark(id, ctx.name);
    const info = infos[id];
    const link = links[id];
    items.push({
      id,
      name: mark.name,
      mark: (
        <Mark
          decorative
          name={mark.name}
          short={mark.short}
          color={mark.color}
          // A symbol this build does not have is `undefined` rather than a
          // blank: the mark then draws the initials, which say more than an
          // empty square does.
          icon={mark.mark === "icon" ? symbolFor(mark.icon) : undefined}
          imageSrc={mark.mark === "image" ? mark.imageSrc : undefined}
          withBadge={mark.withText}
          size="sm"
        />
      ),
      // The version first, because the server is the long half and the hint
      // truncates from the end.
      detail: [info?.version, ctx.server].filter(Boolean).join(" · "),
      // A reason rather than a flag: the kit dims the mark and says the word,
      // so the state is never told in opacity alone. An error with no message
      // still has to say something.
      //
      // The CLASSIFICATION, not the cluster's own words. This string is not
      // drawn — the rail is 46px wide — it joins the mark's accessible name,
      // and what was reaching a screen reader was three hundred characters of
      // `Status { metadata: Some(ListMeta { … })` read out as the name of a
      // button. "Not authorized" is the same fact in two words. The original
      // is not offered here because there is nowhere in a 46px strip to offer
      // it from; the overview's Fleet row for the same cluster has it.
      unavailable:
        link?.state === "error"
          ? link.error
            ? friendly(link.error).title
            : "Unreachable"
          : link?.state === "disconnected"
            ? "Disconnected"
            : undefined,
      markers: link?.state === "connecting" ? [{ label: "Connecting", tone: "info" }] : [],
      color: mark.color,
    });
  }

  /**
   * Switch the rail's cluster, and hand the store the name the strip needs.
   *
   * `setActiveCluster` takes a stableId — that is what a workspace holds (#265)
   * — but a tab's label is the context's NAME, and the store cannot translate
   * between the two (`lib/clusters` imports it, not the other way round). The
   * rail already has both, in `byId`, so the name is passed from here. Without
   * it every cluster-scoped tab kept the label of the cluster switched away
   * from while the mounted screen rendered the new one.
   *
   * Both ways in go through this: the click on a mark and the menu's `Open`.
   * They are one gesture, and fixing one is how the two start disagreeing.
   */
  function select(id: string) {
    setActiveCluster(id, byId.get(id)?.name);
  }

  function remove(id: string) {
    setWorkspaceClusters(
      workspace.id,
      workspace.clusters.filter((c) => c !== id),
    );
    setEditing(null);
  }

  function menuFor(item: ClusterRailItem): ContextMenuItem[] {
    return [
      { label: `Open ${item.name}`, onPick: () => select(item.id) },
      // The ellipsis is the promise that this one asks something more before
      // anything happens — it opens the dialog below.
      { label: "Customise…", icon: Icons.edit, onPick: () => setEditing(item.id) },
      { kind: "sep" },
      { label: "Connection details", onPick: () => openTab("/connections") },
      // Named for what it does. See the note above on the design's Disconnect.
      { label: "Remove from workspace", icon: Icons.trash, danger: true, onPick: () => remove(item.id) },
    ];
  }

  const close = () => setEditing(null);

  return (
    <>
      <ClusterRail
        items={items}
        activeId={active ?? undefined}
        onSelect={select}
        menuFor={menuFor}
        onAdd={onConnect}
        error={error}
      />
      {target && (
        <Dialog
          title={`Customise ${target.name}`}
          onClose={close}
          footer={
            <>
              {/* Reset is the editor's own, but it belongs beside Done rather
                  than in a rule-topped row of its own above it: the design
                  draws one row of controls, and CustomizeMark draws its own
                  only when it is handed an `onReset` to draw it for. */}
              <Button variant="secondary" size="sm" onClick={() => resetMark(target.stableId)}>
                {/* The kit draws the glyph — NavIcon is what it offers for a
                    decorative one — so nothing here renders an icon set's own
                    element. */}
                <NavIcon icon={Icons.revert} /> Reset
              </Button>
              {/* Every edit is already kept — the editor writes through on each
                  keystroke — so this closes rather than commits. It is here
                  because a dialog with no way out but its own × reads as one
                  that has not been answered. */}
              <Button variant="primary" size="sm" onClick={close}>
                Done
              </Button>
            </>
          }
        >
          <CustomizeMark maxNameLength={Infinity}
            value={value}
            onChange={(next) => setMark(target.stableId, next)}
            colors={PALETTE}
            icons={SYMBOLS}
            maxImageBytes={MAX_IMAGE_BYTES}
          />
        </Dialog>
      )}
    </>
  );
}
