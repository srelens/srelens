import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { CrdRef } from "@srelens/core";
import { AboutKind } from "./AboutKind";

const WIDGETS: CrdRef = {
  name: "widgets.example.com",
  group: "example.com",
  version: "v1",
  kind: "Widget",
  plural: "widgets",
  namespaced: true,
  versions: ["v1", "v1beta1"],
  storageVersion: "v1",
};

function about(crd: CrdRef) {
  const { container } = render(<AboutKind crd={crd} context="prod-eu" />);
  return container;
}

/** Every key/value pair the rail drew, in the order it drew them. */
const definition = (container: HTMLElement) =>
  Array.from(container.querySelectorAll("dl.kv")).map(
    (kv) =>
      [kv.querySelector(".kv-k")?.textContent ?? "", kv.querySelector(".kv-v")?.textContent ?? ""] as const,
  );

const keys = (container: HTMLElement) => definition(container).map(([k]) => k);

const valueOf = (container: HTMLElement, key: string) =>
  definition(container).find(([k]) => k === key)?.[1];

describe("AboutKind", () => {
  it("reads a kind's definition off the CRD, in the design's own order", () => {
    const container = about(WIDGETS);

    expect(keys(container)).toEqual(["Kind", "Scope", "Served versions", "Storage version"]);
    expect(definition(container)).toEqual([
      ["Kind", "Widget"],
      ["Scope", "Namespaced"],
      ["Served versions", "v1, v1beta1"],
      ["Storage version", "v1"],
    ]);
  });

  it("says Cluster for a cluster-scoped CRD, which the design never draws", () => {
    // The design hard-codes `Namespaced` and never renders a ClusterIssuer or a
    // PriorityClass, though its own tree lists both. The CRD knows.
    expect(valueOf(about({ ...WIDGETS, namespaced: false }), "Scope")).toBe("Cluster");
  });

  it("omits a version row it has nothing to put in, rather than drawing it blank", () => {
    // Both fields are optional on `CrdRef` — an older backend and a hand-built
    // ref in a test alike arrive without them.
    const container = about({ ...WIDGETS, versions: undefined, storageVersion: undefined });

    expect(keys(container)).toEqual(["Kind", "Scope"]);
    expect(container.textContent).not.toContain("Served versions");
    expect(container.textContent).not.toContain("Storage version");
  });

  it("keeps the served versions when only the storage version is missing", () => {
    const container = about({ ...WIDGETS, storageVersion: undefined });

    expect(keys(container)).toEqual(["Kind", "Scope", "Served versions"]);
  });

  it("drops a served-versions row for a CRD that serves an empty list", () => {
    expect(keys(about({ ...WIDGETS, versions: [] }))).toEqual(["Kind", "Scope", "Storage version"]);
  });

  it("says nothing about how many objects there are — that is the list header's (#402)", () => {
    // It was here, for custom resources alone. The count is in the header
    // now, for every kind, and the same figure in two places on one screen is
    // two things to keep agreeing.
    expect(about(WIDGETS).textContent).not.toContain("Objects");
  });

  it("names the real kind, not the slug with its first letter upper-cased", () => {
    // The design titles `servicemonitors` as `Servicemonitors`. The CRD says
    // `ServiceMonitor`, and that is the kind anyone types at kubectl.
    const container = about({
      ...WIDGETS,
      name: "servicemonitors.monitoring.coreos.com",
      kind: "ServiceMonitor",
      plural: "servicemonitors",
    });

    expect(valueOf(container, "Kind")).toBe("ServiceMonitor");
    expect(container.textContent).not.toContain("Servicemonitors");
  });

  it("hands the reader the command for this cluster and this CRD", () => {
    const container = about({
      ...WIDGETS,
      name: "servicemonitors.monitoring.coreos.com",
      kind: "ServiceMonitor",
    });

    expect(container.querySelector("code.code")?.textContent).toBe(
      "kubectl --context prod-eu get servicemonitors.monitoring.coreos.com -A -o wide",
    );
  });

  it("does not announce the command as an equivalent to anything", () => {
    // `KubectlPreview` says "Equivalent kubectl:" ahead of the command, which
    // is right beside an action the app is about to perform and wrong under
    // "Fetch it yourself", where the command is the content and there is no
    // action for it to be equivalent to.
    expect(about(WIDGETS).textContent).not.toContain("Equivalent");
  });

  it("offers the command through the kit's copy control", () => {
    // The clipboard write, the Copy/Copied flip and the no-clipboard case are
    // `CopyCommand`'s own, and its suite owns them. What is this screen's is
    // that the control is there at all — a command with no way to take it away
    // is a command the reader retypes by eye.
    about(WIDGETS);
    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined();
  });

  it("renders its sections as siblings, so the rail rules between them", () => {
    // `SideRail` drops its `rail` straight into one box and `.section +
    // .section` is what draws the hairline. A wrapper of our own around either
    // block breaks that adjacency and the rail reads as one undivided slab.
    const container = about(WIDGETS);
    const kids = Array.from(container.children);

    expect(kids.map((el) => el.tagName)).toEqual(["SECTION", "SECTION"]);
    expect(kids.map((el) => el.querySelector(".section-title")?.textContent)).toEqual([
      "Definition",
      "Fetch it yourself",
    ]);
  });
});
