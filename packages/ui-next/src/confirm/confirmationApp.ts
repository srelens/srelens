import type { InstalledExtension } from "@srelens/core";
import { extensionLabel, useExtensions } from "../extensions/inventoryStore";
import type { ConfirmationApp } from "./HostConfirmation";

/**
 * Which app a call was made through, as the HOST knows it — an ID and the
 * revision it was installed at, and nothing else.
 *
 * The NAME and the PUBLISHER are deliberately not here and never cross a wire.
 * They are looked up from the host's own installed inventory by
 * {@link appIdentity}, so a caller cannot name itself in the sentence a person
 * is asked to approve, cannot claim a publisher, and cannot claim to be an app
 * at all — an ID that resolves to nothing draws no requester line.
 *
 * **This comes from host context, never from a request.** An app's own screen
 * is rendered by the host from its own inventory, so the selection there names
 * the app authentically. A `mcp://confirm-request` does not: an MCP client is
 * a bearer token and nothing authenticates it as the app its arguments name,
 * which is why `ConfirmTarget` carries no app and `RequestConfirmation` never
 * builds one of these.
 */
export interface ConfirmationAppRef {
  id: string;
  revision: number;
}

/**
 * The app behind `ref`, as the host will name it — or `null` when the host
 * cannot find it.
 *
 * `null` rather than a line built from the ID: "Requested by app x (unsigned)"
 * is a claim about an app, and the host has no grounds for it when the app is
 * not installed. That happens when an app is removed between raising a
 * confirmation and answering it, and inventing a requester there would be the
 * one sentence on this surface that nothing backs.
 *
 * A QUARANTINED app is named by its ID and reported unsigned whatever proof it
 * carries. Its stored name is one the host stopped accepting — the rule
 * `extensionLabel` already applies in the app list, applied here because a
 * confirmation is where being taken for another app pays best — and its
 * signature is precisely what failed to re-verify.
 *
 * **The REVISION is part of the identity**, not decoration carried for its own
 * sake. A review raised under revision 4 and answered after the app was
 * replaced by revision 5 is not a review of revision 5: its name and its
 * signature state may both differ, and vouching for the installed one would
 * name an app that is not the one that asked. `resolve`
 * (`crates/registry/src/extensions/resource.rs`) matches on `id` AND
 * `revision` before it will run anything, so an action whose revision has
 * moved on is one the host would refuse in any case — the line is dropped for
 * the same reason an unknown ID drops it, and the reader is not told a name
 * the host is about to disown.
 */
export function appIdentity(
  plugins: readonly InstalledExtension[],
  ref: ConfirmationAppRef | null | undefined,
): ConfirmationApp | null {
  if (!ref) return null;
  const plugin = plugins.find((p) => p.manifest.id === ref.id && p.revision === ref.revision);
  if (!plugin) return null;
  return {
    name: extensionLabel(plugin),
    publisher: plugin.signatureProof && !plugin.quarantined ? "srelens" : null,
  };
}

/**
 * {@link appIdentity} against the window's live inventory.
 *
 * One poll per window whatever the number of consumers (`inventoryStore`), and
 * an empty list off the desktop — so a surface that has no app to name, or no
 * inventory to name it from, simply draws no requester line.
 */
export function useConfirmationApp(ref: ConfirmationAppRef | null | undefined): ConfirmationApp | null {
  const inventory = useExtensions();
  return appIdentity(inventory.data?.plugins ?? [], ref);
}
