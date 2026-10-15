import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  collectionCoversKind,
  resolveResourceCollections,
  selectsNothing,
  type Collection,
  type KindRef,
  type ResourceResolution,
} from "@srelens/core";
import { byRecentUse, markCollectionsUsed, useCollections } from "./collections";
import { rowKey, type ListRow } from "./kinds/types";
import { useTabScope } from "./tabScope";
import { currentWorkspace, setTabView, useTabView } from "./tabsStore";

/** How long the rows have to settle before the collections are worked out again. */
const SETTLE_MS = 400;
/** How often a narrowed list is worked out again with nothing changing on screen. */
export const RESOLVE_AGAIN_MS = 30_000;

export type ResourceFilterState =
  /** No collection chosen: the list is as it was. */
  | { status: "off" }
  | { status: "resolving" }
  | { status: "ready"; keys: ReadonlySet<string>; resolution: ResourceResolution }
  | { status: "failed"; error: string };

export interface ResourceCollectionFilter {
  /** The resource collections that cover this kind, most recently used first. */
  offered: Collection[];
  /** The chosen ones that still exist. */
  chosen: Collection[];
  /** Choose collections for this list; none goes back to every row. */
  choose: (names: string[]) => void;
  state: ResourceFilterState;
  /** The rows the collections keep, or every row when none is chosen. */
  apply: <Row extends ListRow>(rows: Row[]) => Row[];
}

/**
 * The resource collections (#885) a list of one kind is narrowed to.
 *
 * Chosen on the tab, so a list keeps them across a restart the way it keeps
 * its sort and filter. Worked out by the backend for the kind and the list's
 * namespace scope — it reads the kind's names and labels, which the rows do
 * not carry — and again whenever the rows themselves change (an object
 * created or deleted may now match a pattern), and every half a minute.
 *
 * While that is being worked out the list shows nothing rather than every
 * row: a list narrowed to a release must not show the whole kind meanwhile.
 */
export function useResourceCollectionFilter(
  context: string,
  kind: KindRef | undefined,
  namespaces: string[],
  rows: ListRow[],
): ResourceCollectionFilter {
  const tabId = useTabScope() ?? currentWorkspace().activeId;
  const view = useTabView(tabId);
  const all = useCollections();
  const kindKey = kind ? `${kind.group ?? ""}/${kind.kind}` : "";
  const offered = useMemo(
    () => (kind ? byRecentUse(all, "resource").filter((c) => collectionCoversKind(c, kind)) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [all, kindKey],
  );
  const names = view.collections ?? [];
  const chosen = useMemo(() => offered.filter((c) => names.includes(c.name)), [offered, names.join("\u0000")]); // eslint-disable-line react-hooks/exhaustive-deps

  const choose = useCallback(
    (next: string[]) => {
      setTabView(tabId, { collections: next });
      markCollectionsUsed("resource", next);
    },
    [tabId],
  );

  // What decides the answer: the collections' rules, the kind, the scope and
  // which rows there are — not their contents, which change on every status.
  const rulesKey = JSON.stringify(chosen.map((c) => [c.name, c.kinds, c.members, c.patterns, c.labelSelector]));
  const scopeKey = namespaces.join(",");
  const rowsKey = useMemo(() => rows.map(rowKey).sort().join("\n"), [rows]);
  const [state, setState] = useState<ResourceFilterState>({ status: "off" });
  const [tick, setTick] = useState(0);
  // A change of collection, kind or scope is a different answer, shown only
  // once it is known; the same question asked again — new rows, the timer —
  // keeps the last answer on screen meanwhile, so the list does not blink.
  const asked = useRef("");

  useEffect(() => {
    if (chosen.length === 0 || !kind) {
      asked.current = "";
      setState({ status: "off" });
      return;
    }
    if (selectsNothing(namespaces)) {
      setState({ status: "ready", keys: new Set(), resolution: { keys: [], collections: [], partial: false } });
      return;
    }
    let live = true;
    const question = `${context}|${kindKey}|${scopeKey}|${rulesKey}`;
    if (question !== asked.current) {
      asked.current = question;
      setState({ status: "resolving" });
    }
    const timer = setTimeout(() => {
      resolveResourceCollections(context, kind, namespaces, chosen).then(
        (resolution) => live && setState({ status: "ready", keys: new Set(resolution.keys), resolution }),
        (error: unknown) => live && setState({ status: "failed", error: error instanceof Error ? error.message : String(error) }),
      );
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context, kindKey, scopeKey, rulesKey, rowsKey, tick]);

  useEffect(() => {
    if (chosen.length === 0) return;
    const timer = setInterval(() => setTick((t) => t + 1), RESOLVE_AGAIN_MS);
    return () => clearInterval(timer);
  }, [chosen.length]);

  const apply = useCallback(
    <Row extends ListRow>(list: Row[]): Row[] => {
      if (state.status === "off") return list;
      if (state.status !== "ready") return [];
      return list.filter((row) => state.keys.has(rowKey(row)));
    },
    [state],
  );

  return { offered, chosen, choose, state, apply };
}
