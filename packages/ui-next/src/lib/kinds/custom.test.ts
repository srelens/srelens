import { describe, it, expect } from "vitest";
import type { CrdRef } from "@srelens/core";
import { customDescriptor, customColumns } from "./custom";

const crd = (over: Partial<CrdRef> = {}): CrdRef => ({
  name: "widgets.example.com", group: "example.com", version: "v1", plural: "widgets",
  kind: "Widget", namespaced: true,
  printerColumns: [
    { name: "Phase", type: "string", jsonPath: ".status.phase" },
    { name: "Since", type: "date", jsonPath: ".status.since" },
  ],
  ...over,
});

describe("custom columns", () => {
  // Whole-branch review (FIX 4): this used to assert the opposite — written
  // before the "Name" rule existed. All 23 typed sets and the generic three
  // title their identifier column "Name", never the kind; a custom resource
  // is no exception.
  it("titles the identifier column Name, not the CRD's kind", () => {
    expect(customColumns(crd())[0].header).toBe("Name");
  });

  it("gives each printer column a cell from the row's positional values", () => {
    const cols = customColumns(crd());
    const phase = cols.find((c) => c.header === "Phase")!;
    expect(phase.render!({ name: "w", namespace: "d", age: "1d", columns: ["Ready", "2026-01-01"] })).toBe("Ready");
  });

  it("drops the namespace column for a cluster-scoped CRD", () => {
    expect(customColumns(crd({ namespaced: false })).some((c) => c.key === "namespace")).toBe(false);
  });

  it("falls back to the generic set when the CRD declares no printer columns", () => {
    const cols = customColumns(crd({ printerColumns: [] }));
    expect(cols.map((c) => c.key)).toEqual(["name", "namespace", "age"]);
  });

  // `printerSortValue(type, value, sortKey)` takes three `string` parameters,
  // so a future argument swap at the call site in customColumns still
  // compiles. These pin the order by giving `columns` and `sortKeys` values
  // that disagree, so a swap changes the result rather than passing by luck.
  describe("printer column sort values", () => {
    it("sorts a string-typed column by its rendered value, not the raw sort key", () => {
      const cols = customColumns(crd());
      const phase = cols.find((c) => c.header === "Phase")!;
      const row = { name: "w", namespace: "d", age: "1d", columns: ["Ready"], sortKeys: ["NotReady"] };
      expect(phase.getSortValue!(row)).toBe("Ready");
    });

    it("sorts a date-typed column by its raw timestamp, not its rendered age text", () => {
      const cols = customColumns(crd());
      const since = cols.find((c) => c.header === "Since")!;
      const now = Date.now();
      // Rendered text says the opposite of the truth: "fresh" looks old
      // ("10d") and "stale" looks recent ("2h"). Only the raw sort key —
      // the third argument — carries the real age.
      // "Since" is the second printer column (index 1); index 0 is filler
      // for "Phase" so the positional lookup lands on the right entry.
      const fresh = {
        name: "a", namespace: "d", age: "1d",
        columns: ["Ready", "10d"], sortKeys: ["", new Date(now - 60_000).toISOString()],
      };
      const stale = {
        name: "b", namespace: "d", age: "1d",
        columns: ["Ready", "2h"], sortKeys: ["", new Date(now - 30 * 86_400_000).toISOString()],
      };
      expect(since.getSortValue!(fresh) as number).toBeLessThan(since.getSortValue!(stale) as number);
    });
  });
});

describe("customDescriptor", () => {
  // Whole-branch review (FIX 3): the backend resolves kind→GVR through a
  // closed match with no CRD path, so Delete on a custom resource always
  // fails — offering it is worse than not offering it.
  it("withholds Delete, since the backend has no kind→GVR path for a CRD's kind", () => {
    expect(customDescriptor(crd()).actions.delete).toBe(false);
  });
});

describe("customDescriptor", () => {
  // `customDescriptorFor(slug, crds)` used to live here and wrap this in a
  // `crds.find`. It was deleted rather than kept as a seam once `Resources`
  // stopped calling it: that screen needs the `CrdRef` itself, not only the
  // descriptor built from it, because the "About this kind" rail reads the
  // kind, the scope and the versions straight off it. Two finds over the same
  // list — one for the columns and one for the rail — is two chances for the
  // table and the rail to describe different definitions, and an exported
  // function whose only caller is its own test is a seam nobody is holding.
  it("names the kind by its CRD, not by the slug that routed to it", () => {
    expect(customDescriptor(crd()).k8sKind).toBe("Widget");
  });
});
