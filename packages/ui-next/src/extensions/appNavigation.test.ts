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
  expect(navigation.map(({ id, label }) => ({ id, label }))).toEqual([{ id: "extension:org.srelens.trivy", label: "Trivy" }]);
  expect(navigation[0].children?.map(({ id, label }) => ({ id, label }))).toEqual([
    { id: "route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/source-status", label: "Overview" },
    { id: "route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/list-images", label: "Images" },
  ]);
  expect(appNavigation([{ ...app, enabled: false }], "config#a/b")).toEqual([]);
  expect(appNavigation([{ ...app, contexts: ["other"] }], "config#a/b")).toEqual([]);
});

it("omits an app whose only operations are hidden", () => {
  const hidden = structuredClone(app);
  hidden.manifest.contributions.pages = [];
  hidden.manifest.sidecar!.operations = [{name:"findings",title:"Findings",view:{hidden:true}}];
  expect(appNavigation([hidden], "config#demo")).toEqual([]);
});

/** #1001: an app with one place to go is a row, not a heading with one row under it. */
it("draws an app with one destination as a leaf, named after the app", () => {
  const single = structuredClone(app);
  single.manifest.sidecar!.operations = [{ name: "list-images", title: "Images" }];
  const [entry] = appNavigation([single], "config#a/b");
  expect(entry.children).toBeUndefined();
  expect(entry.id).toBe("route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/list-images");
  expect(entry.label).toBe("Trivy");
  expect(entry.icon).toBeTruthy();
});
