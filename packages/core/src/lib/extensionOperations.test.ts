import { expect, it } from "vitest";
import { extensionOperationRoute, parseExtensionOperationRoute } from "./extensions";

it("keeps executable operation tabs separate by context, app revision and operation", () => {
  const route = extensionOperationRoute("config#a/b", "org.srelens.trivy", 3, "source-status");
  expect(route).toBe("/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/source-status");
  expect(parseExtensionOperationRoute(route)).toEqual({ contextKey: "config#a/b", id: "org.srelens.trivy", revision: 3, operation: "source-status" });
  expect(extensionOperationRoute("other", "org.srelens.trivy", 3, "source-status")).not.toBe(route);
  expect(extensionOperationRoute("config#a/b", "org.srelens.trivy", 4, "source-status")).not.toBe(route);
  for (const bad of ["/extension-operation-contexts/%zz/app/3/op", "/extension-operation-contexts/a/app/0/op", "/extension-operation-contexts/a/app/3/op/extra", "/extension-operation-contexts/a/app/3/op?context=other"]) {
    expect(parseExtensionOperationRoute(bad)).toBeNull();
  }
});

it("pins a selected report in the operation route and rejects nonscalar route inputs", () => {
  const a = extensionOperationRoute("config#a", "org.srelens.trivy", 3, "findings", { reportId: "report-a", namespace: "team" });
  const b = extensionOperationRoute("config#a", "org.srelens.trivy", 3, "findings", { reportId: "report-b", namespace: "team" });
  expect(a).not.toBe(b);
  expect(parseExtensionOperationRoute(a)?.params).toEqual({ reportId: "report-a", namespace: "team" });
  expect(parseExtensionOperationRoute("/extension-operation-contexts/a/app/3/op/" + encodeURIComponent('{"reportId":{}}'))).toBeNull();
});
