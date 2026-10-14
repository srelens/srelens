import { CopyIconButton } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";

/**
 * Put a resource's name on the clipboard. Reports whether it worked, which is
 * the whole of what lets the button say so — see `copyKubectlCommand` for the
 * same guard and the same reason: with no clipboard at all (web mode on a
 * plain-http origin) an optional-chained write resolves and reads as success.
 */
async function copyName(name: string): Promise<boolean> {
  if (!navigator.clipboard) return false;
  try {
    await navigator.clipboard.writeText(name);
    return true;
  } catch {
    return false;
  }
}

/**
 * The copy control beside a resource's name, in the peek's heading and the
 * full tab's (#827).
 *
 * A resource name is the thing a reader takes somewhere else — a terminal, a
 * search, a ticket — more than anything else on the pane, and the only way to
 * take it was to select it by hand: `kps-kube-prometheus-stack-operator-…`
 * with its trailing hash, without clipping a character. `Copy as kubectl`
 * copies a whole command, which is a different thing.
 *
 * The bare name and nothing else: no kind, no namespace. Both hosts print
 * those beside it, and a name is what pastes into `kubectl get pod <here>`.
 *
 * The kit's `CopyIconButton`, so the check, the "Copied"/"Copy failed" tooltip
 * and the live-region announcement are the ones every other copy control in
 * the app gives. It is named for what it copies — "Copy name web-0" — because
 * a pane can hold several copy buttons and "Copy" alone says nothing to a
 * reader who cannot see which one they are on.
 */
export function CopyNameButton({ name }: { name: string }) {
  // Keyed by the name, so the confirmation belongs to the name it was given
  // for. The peek stays mounted while the reader walks from row to row, and
  // an un-keyed button carried its check across: copy `web-1`, click `web-2`,
  // and the control beside `web-2` said "Copied" over a clipboard holding
  // `web-1`. A new name is a new button, at rest.
  return (
    <CopyIconButton key={name} icon={Icons.copy} label={`Copy name ${name}`} onCopy={() => copyName(name)} />
  );
}
