/**
 * What the BUTTON says for each action the host offers. Only that.
 *
 * One copy, shared by the single-resource detail and the bulk bar. A second
 * copy is how the two surfaces end up saying "Force reconcile" and "Force",
 * and the sentence a reader approves has to be the same question whichever
 * screen asked it (#552). The descriptions that used to sit beside each label
 * are gone: they come from the host's own confirmation template now, per
 * action, in `ExtensionResourceDetail.actionMeta` (#548).
 */
export const ACTION_LABELS: Record<string, string> = {
  suspend: "Suspend",
  resume: "Resume",
  reconcile: "Reconcile",
  force: "Force reconcile",
  reset: "Reset retries",
  refresh: "Refresh status",
  "hard-refresh": "Hard refresh",
  sync: "Sync",
};

/** True when the host named an action this build knows how to label. */
export const isKnownAction = (action: string) => Object.hasOwn(ACTION_LABELS, action);
