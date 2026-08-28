// First-run guidance and the empty states that carry it (#161).
//
// The rules live here rather than inline in the components so they can be
// tested without a DOM, and so "have they been onboarded" has one answer.

import { settingsStorage } from "./settingsStorage";

const ONBOARDED_KEY = "srelens.onboarded";

/** Whether the user has already been shown (or outgrown) the first-run help. */
export function loadOnboarded(): boolean {
  try {
    return settingsStorage.getItem(ONBOARDED_KEY) === "true";
  } catch {
    // A storage failure must not mean an onboarding card on every launch
    // forever; treat an unreadable flag as "seen".
    return true;
  }
}

export function saveOnboarded(): void {
  try {
    settingsStorage.setItem(ONBOARDED_KEY, "true");
  } catch {
    /* non-fatal: the card reappears next launch */
  }
}

/**
 * Whether to show the first-run card.
 *
 * Only before the flag is set, and only while contexts are still loading or
 * present — someone with no kubeconfig at all has a more specific problem, and
 * gets the connect-a-cluster call to action instead of tips about the command
 * palette.
 */
export function shouldShowFirstRun(onboarded: boolean, contextCount: number | null): boolean {
  if (onboarded) return false;
  return contextCount === null || contextCount > 0;
}

/** A list's empty state: what is missing, and what the user can do about it. */
export interface EmptyListMessage {
  title: string;
  hint: string;
}

/**
 * The empty state for a resource list.
 *
 * "No pods" alone leaves the user unsure whether the cluster is empty or they
 * are looking at the wrong slice of it, so each case names what is narrowing
 * the view rather than repeating that nothing was found.
 *
 * Only two things narrow it: the search box and the namespace scope. Picking a
 * column narrows WHERE a search looks, not which rows survive, so with an empty
 * query it is not a filter and must not be blamed for an empty list.
 */
export function emptyListMessage(opts: {
  /** Plural resource label as shown in the UI, e.g. "pods". */
  kind: string;
  /** The active search text, if any. */
  query?: string;
  /** Selected namespaces; empty means every namespace. */
  namespaces?: readonly string[];
  /** False for cluster-scoped kinds, where namespaces are meaningless. */
  namespaced?: boolean;
}): EmptyListMessage {
  const { kind, namespaces = [], namespaced = true } = opts;
  // Trimmed, to match filterTableData: it ignores a whitespace-only query and
  // returns every row, so treating one as an active search would blame a
  // filter that is not filtering.
  const query = opts.query?.trim();
  if (query) {
    return {
      title: `No ${kind} match “${query}”`,
      hint: "Clear the search to see everything in scope.",
    };
  }
  const scope =
    !namespaced || namespaces.length === 0
      ? ""
      : namespaces.length === 1
        ? ` in ${namespaces[0]}`
        : ` in ${namespaces.length} namespaces`;
  return {
    title: `No ${kind}${scope}`,
    hint: scope
      ? "Switch namespace, or select all namespaces to look wider."
      : `This cluster has no ${kind} you can see.`,
  };
}
