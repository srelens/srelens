import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { filtersKind, matchCollectionNames, type Collection, type ListKind, type NameMatches } from "@srelens/core";
import { useCollectionScope } from "./collectionScope";
import type { ListRow } from "./kinds/types";
import { useNamespaceCollections } from "./workspace";

/** How long the rows have to settle before their names are matched again. */
const SETTLE_MS = 200;

export type NameFilterState =
  /** No collection in force narrows this kind: the list is as it was. */
  | { status: "off" }
  | { status: "matching" }
  | { status: "ready"; shown: ReadonlySet<string>; problems: string[] }
  | { status: "failed"; error: string };

export interface CollectionNameFilter {
  /** The collections in force that narrow this kind's list. */
  narrowing: Collection[];
  state: NameFilterState;
  /** The rows the collections' name expressions keep, or every row when none narrows this kind. */
  apply: <Row extends ListRow>(rows: Row[]) => Row[];
}

/**
 * A list of one kind narrowed by the namespace collections its tab is scoped
 * to (#885): a collection whose resource filter names this kind shows only
 * the resources whose name matches its expression. Several are a union, and
 * one in force that does not filter this kind allows every name.
 *
 * The matching is the backend's (`collections.matchNames`), over the names
 * the list already has, worked out again whenever its rows change. While the
 * first answer is on its way the list shows nothing rather than every row;
 * after that the last answer stays on screen until the next one lands.
 */
export function useCollectionNameFilter(clusterId: string | undefined, kind: ListKind | undefined, rows: ListRow[]): CollectionNameFilter {
  const names = useNamespaceCollections(clusterId);
  const scope = useCollectionScope(clusterId, names);
  const inForce = useMemo(() => (scope && scope.status !== "gone" ? scope.collections : []), [scope]);
  const narrowing = useMemo(() => (kind ? inForce.filter((c) => filtersKind(c, kind)) : []), [inForce, kind?.kind, kind?.group]); // eslint-disable-line react-hooks/exhaustive-deps
  // A collection in force that does not filter this kind allows every name.
  const narrows = kind !== undefined && narrowing.length > 0 && narrowing.length === inForce.length;

  const rowNames = useMemo(() => [...new Set(rows.map((r) => r.name))].sort(), [rows]);
  const namesKey = rowNames.join("\n");
  const rulesKey = JSON.stringify(inForce.map((c) => [c.name, c.resources]));
  const [state, setState] = useState<NameFilterState>({ status: "off" });
  const asked = useRef("");

  useEffect(() => {
    if (!narrows || !kind) {
      asked.current = "";
      setState({ status: "off" });
      return;
    }
    let live = true;
    // A different rule is a different answer, shown only once known; the
    // same rule over new rows keeps the last answer on screen meanwhile.
    const question = `${kind.group ?? ""}/${kind.kind}|${rulesKey}`;
    if (question !== asked.current) {
      asked.current = question;
      setState({ status: "matching" });
    }
    const timer = setTimeout(() => {
      matchCollectionNames(inForce, kind, rowNames).then(
        (answer: NameMatches) => {
          if (!live) return;
          const shown = new Set(rowNames.filter((_, i) => answer.matches[i]));
          setState({ status: "ready", shown, problems: answer.problems });
        },
        (error: unknown) => live && setState({ status: "failed", error: error instanceof Error ? error.message : String(error) }),
      );
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [narrows, kind?.kind, kind?.group, rulesKey, namesKey]);

  const apply = useCallback(
    <Row extends ListRow>(list: Row[]): Row[] => {
      if (state.status === "off") return list;
      if (state.status !== "ready") return [];
      return list.filter((row) => state.shown.has(row.name));
    },
    [state],
  );

  return { narrowing: narrows ? narrowing : [], state, apply };
}
