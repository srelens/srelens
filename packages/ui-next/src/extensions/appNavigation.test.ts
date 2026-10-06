import { expect, it } from "vitest";
import type { InstalledExtension } from "@srelens/core";
import { appNavigation } from "./appNavigation";

const app = {
  enabled: true, revision: 3, settings: {}, grants: [], source: "local", installedAt: 1, history: [],
  manifest: {
    id: "org.srelens.trivy", name: "Trivy", version: "0.1.0", srelensApiVersion: "^0.7",
    kind: "executable", permissions: [], capabilities: [],
    sidecar: { binaries: {}, operations: [{ name: "source-status", title: "Overview" }, { name: "list-images", title: "Images" }] },
    contributions: { pages: [], detailTabs: [], detailLinks: [] },
  },
} as InstalledExtension;

it("opens executable operations on the cluster and installed revision they belong to", () => {
  const navigation = appNavigation([app], "config#a/b");
  expect(navigation?.children?.[0].label).toBe("Trivy");
  expect(navigation?.children?.[0].children?.map(({ id, label }) => ({ id, label }))).toEqual([
    { id: "route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/source-status", label: "Overview" },
    { id: "route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/list-images", label: "Images" },
  ]);
  expect(appNavigation([{ ...app, enabled: false }], "config#a/b")).toBeNull();
  expect(appNavigation([{ ...app, contexts: ["other"] }], "config#a/b")).toBeNull();
});
