import { afterEach, expect, it, vi } from "vitest";
import ReactDom from "react-dom";
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { mount } from "../src/runtime.jsx";
const source = execFileSync(
  "tar",
  [
    "-xOzf",
    resolve(
      dirname(fileURLToPath(import.meta.url)),
      "../../../crates/plugin-host/tests/fixtures/freelens-flux-5.3.1.tgz",
    ),
    "package/out/renderer/index.js",
  ],
  { maxBuffer: 2e6 },
).toString();
const settle = () => new Promise((r) => setTimeout(r, 30));
afterEach(() => {
  ReactDom.unmountComponentAtNode(document.getElementById("root"));
  document.body.innerHTML = "";
});
const descriptors = [
  ["source", "GitRepository", "gitrepositories", "v1"],
  ["source", "HelmRepository", "helmrepositories", "v1"],
  ["source", "HelmChart", "helmcharts", "v1"],
  ["source", "Bucket", "buckets", "v1"],
  ["source", "OCIRepository", "ocirepositories", "v1"],
  ["kustomize", "Kustomization", "kustomizations", "v1"],
  ["helm", "HelmRelease", "helmreleases", "v2"],
  ["image", "ImageRepository", "imagerepositories", "v1"],
  ["image", "ImagePolicy", "imagepolicies", "v1"],
  ["image", "ImageUpdateAutomation", "imageupdateautomations", "v1"],
  ["notification", "Alert", "alerts", "v1beta3"],
  ["notification", "Provider", "providers", "v1beta3"],
  ["notification", "Receiver", "receivers", "v1"],
].map(([prefix, kind, plural, version]) => ({
  group: `${prefix}.toolkit.fluxcd.io`,
  kind,
  plural,
  version,
  versions: [version],
}));
function start(options = {}) {
  document.body.innerHTML = '<div id="root"></div>';
  const request = vi.fn(async ({ operation, kind, group, version }) =>
    operation === "events"
      ? { events: [] }
      : {
          objects: [
            {
              apiVersion: `${group}/${version}`,
              kind,
              metadata: {
                name: "fixture",
                namespace: "flux-system",
                creationTimestamp: "2026-01-01T00:00:00Z",
              },
              spec: {
                url: "https://example.test/repo",
                interval: "1m",
                ref: { branch: "main" },
                chart:
                  kind === "HelmChart"
                    ? "test"
                    : {
                        spec: {
                          chart: "test",
                          sourceRef: {
                            kind: "HelmRepository",
                            name: "fixture",
                          },
                        },
                      },
                sourceRef: { kind: "GitRepository", name: "fixture" },
                imageRepositoryRef: { name: "fixture" },
                providerRef: { name: "fixture" },
                eventSources: [],
                resources: [],
                secretRef: { name: "webhook-token" },
                repository: "test",
                policy: {},
              },
              status: {
                conditions: [{ type: "Ready", status: "True" }],
                artifact: { revision: "main@sha1:0123456789abcdef" },
              },
            },
          ],
        },
  );
  mount({
    source,
    crds: descriptors,
    namespaces: ["flux-system"],
    request,
    ...options,
  });
  return request;
}
it("runs every registered resource list and its details with the host adapters", async () => {
  start({ page: "gitrepository" });
  await settle();
  for (const page of [
    "Git Repositories",
    "Helm Repositories",
    "Helm Charts",
    "Buckets",
    "OCI Repositories",
    "Kustomizations",
    "Helm Releases",
    "Image Repositories",
    "Image Policies",
    "Image Update Automations",
    "Alerts",
    "Providers",
    "Receivers",
  ]) {
    const button = [...document.querySelectorAll("nav button")].find(
      (b) => b.textContent === page,
    );
    expect(button, page).toBeTruthy();
    button.click();
    await settle();
    expect(document.body.textContent, page).not.toContain(
      "Extension rendering failed",
    );
    const row = document.querySelector('[data-resource-name="fixture"]');
    expect(row, page).toBeTruthy();
    row.click();
    await settle();
    expect(document.querySelector("[role=dialog]"), page).toBeTruthy();
    expect(
      document.querySelector("[role=dialog]").textContent,
      page,
    ).not.toContain("Extension rendering failed");
    document.querySelector('[aria-label="Close details"]').click();
  }
});
it("mounts upstream dashboard charts and supports navigation from them", async () => {
  start();
  await settle();
  await settle();
  expect(document.body.textContent).toContain("FluxCD Overview");
  expect(document.querySelectorAll(".donut").length).toBeGreaterThan(0);
  const link = [...document.querySelectorAll("a")].find((a) =>
    a.textContent.includes("Git Repositories"),
  );
  expect(link).toBeTruthy();
  link.click();
  await settle();
  expect(document.querySelector('[data-resource-name="fixture"]')).toBeTruthy();
});
