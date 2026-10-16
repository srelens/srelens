import { describe, it, expect, vi } from "vitest";
import type { CrdRef } from "@srelens/core";

const { listCrds } = vi.hoisted(() => ({ listCrds: vi.fn() }));
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  listCrds,
}));

import { isCustomKind, resolveCrdGvk } from "./crdGvk";

const WIDGET: CrdRef = {
  name: "widgets.example.com",
  group: "example.com",
  version: "v1",
  kind: "Widget",
  plural: "widgets",
  namespaced: true,
  printerColumns: [{ name: "Phase", jsonPath: ".status.phase", type: "string" }],
};

describe("resolveCrdGvk", () => {
  it("hands back the matched CRD whole, beside the group, version and plural a read needs", async () => {
    listCrds.mockResolvedValue({ crds: [WIDGET] });
    const out = await resolveCrdGvk("ctx", "Widget");
    expect(out.crd).toEqual({ group: "example.com", version: "v1", plural: "widgets" });
    expect(out.ref).toEqual(WIDGET);
  });
});

describe("isCustomKind", () => {
  it("is custom with a group, or outside the built-in table", () => {
    expect(isCustomKind("Widget")).toBe(true);
    expect(isCustomKind("Pod")).toBe(false);
    // A CRD may reuse a built-in's name in its own group.
    expect(isCustomKind("Deployment", "acme.io")).toBe(true);
  });
});
