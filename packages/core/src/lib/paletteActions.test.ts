import { describe, it, expect, vi, beforeEach } from "vitest";

const {
  deleteResourceMock,
  rolloutRestartMock,
  cordonNodeMock,
  drainNodeMock,
  cronjobSetSuspendMock,
  cronjobTriggerNowMock,
  createNodeDebugPodMock,
} = vi.hoisted(() => ({
  deleteResourceMock: vi.fn(),
  rolloutRestartMock: vi.fn(),
  cordonNodeMock: vi.fn(),
  drainNodeMock: vi.fn(),
  cronjobSetSuspendMock: vi.fn(),
  cronjobTriggerNowMock: vi.fn(),
  createNodeDebugPodMock: vi.fn(),
}));
vi.mock("../lib/actions", () => ({
  deleteResource: deleteResourceMock,
  rolloutRestart: rolloutRestartMock,
  cordonNode: cordonNodeMock,
  drainNode: drainNodeMock,
  cronjobSetSuspend: cronjobSetSuspendMock,
  cronjobTriggerNow: cronjobTriggerNowMock,
  createNodeDebugPod: createNodeDebugPodMock,
}));

const { deletePodMock, evictPodMock } = vi.hoisted(() => ({
  deletePodMock: vi.fn(),
  evictPodMock: vi.fn(),
}));
vi.mock("../lib/workloads", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/workloads")>();
  return { ...actual, deletePod: deletePodMock, evictPod: evictPodMock };
});

import { PALETTE_ACTIONS, actionsForKind, appPaletteCommands, paletteActionCapabilityIds, type PaletteActionCtx } from "./paletteActions";
import type { ExtensionManifest } from "./extensions";

function ctx(overrides: Partial<PaletteActionCtx> = {}): PaletteActionCtx {
  return {
    context: "kind-dev",
    kind: "deployments",
    namespace: "default",
    name: "web",
    ...overrides,
  };
}

function actionByCapability(capabilityId: string, label?: string) {
  const action = PALETTE_ACTIONS.find((a) => a.capabilityId === capabilityId && (label === undefined || a.label === label));
  if (!action) throw new Error(`no action found for ${capabilityId}${label ? ` (${label})` : ""}`);
  return action;
}

beforeEach(() => {
  deleteResourceMock.mockReset();
  rolloutRestartMock.mockReset();
  cordonNodeMock.mockReset();
  drainNodeMock.mockReset();
  cronjobSetSuspendMock.mockReset();
  cronjobTriggerNowMock.mockReset();
  createNodeDebugPodMock.mockReset();
  deletePodMock.mockReset();
  evictPodMock.mockReset();
});

describe("paletteActions", () => {
  it("exposes actions with non-empty capability ids and labels", () => {
    expect(PALETTE_ACTIONS.length).toBeGreaterThan(0);
    for (const a of PALETTE_ACTIONS) {
      expect(a.capabilityId).toMatch(/^[a-z0-9]+\./);
      expect(a.label.length).toBeGreaterThan(0);
    }
  });

  it("filters actions applicable to a kind", () => {
    const forDeploy = actionsForKind("deployments").map((a) => a.capabilityId);
    expect(forDeploy).toContain("k8s.scale");
    expect(forDeploy).toContain("k8s.rolloutRestart");
    const forCm = actionsForKind("configmaps").map((a) => a.capabilityId);
    expect(forCm).not.toContain("k8s.scale");
  });

  it("returns no actions for UI-only pseudo-kinds", () => {
    expect(actionsForKind("settings")).toEqual([]);
  });

  it("reports its covered capability ids", () => {
    const ids = paletteActionCapabilityIds();
    expect(ids.has("k8s.deleteResource")).toBe(true);
    expect(ids.has("k8s.scale")).toBe(true);
    expect(ids.has("k8s.debugPod")).toBe(true);
  });

  it("marks scale as dialog-based with no run", () => {
    const scale = actionByCapability("k8s.scale");
    expect(scale.opensDialog).toBe("scale");
    expect(scale.run).toBeUndefined();
  });

  it("marks debug pod as dialog-based with no run", () => {
    const debug = actionByCapability("k8s.debugPod");
    expect(debug.opensDialog).toBe("debug");
    expect(debug.run).toBeUndefined();
  });

  it("deletes a resource, converting kind via K8S_KIND and passing namespace through as-is", async () => {
    deleteResourceMock.mockResolvedValue({ ok: true });
    const del = actionByCapability("k8s.deleteResource");
    await del.run!(ctx({ kind: "deployments", namespace: null, name: "web" }));
    expect(deleteResourceMock).toHaveBeenCalledWith("kind-dev", "Deployment", null, "web");
  });

  it("triggers a rollout restart with the converted kind and namespace", async () => {
    rolloutRestartMock.mockResolvedValue({ ok: true });
    const restart = actionByCapability("k8s.rolloutRestart");
    await restart.run!(ctx({ kind: "statefulsets", namespace: "prod", name: "web" }));
    expect(rolloutRestartMock).toHaveBeenCalledWith("kind-dev", "StatefulSet", "prod", "web");
  });

  it("cordons a node", async () => {
    cordonNodeMock.mockResolvedValue({ ok: true });
    const cordon = actionByCapability("k8s.cordonNode", "Cordon node");
    await cordon.run!(ctx({ kind: "nodes", namespace: null, name: "node-a" }));
    expect(cordonNodeMock).toHaveBeenCalledWith("kind-dev", "node-a", true);
  });

  it("uncordons a node", async () => {
    cordonNodeMock.mockResolvedValue({ ok: true });
    const uncordon = actionByCapability("k8s.cordonNode", "Uncordon node");
    await uncordon.run!(ctx({ kind: "nodes", namespace: null, name: "node-a" }));
    expect(cordonNodeMock).toHaveBeenCalledWith("kind-dev", "node-a", false);
  });

  it("suspends a cronjob", async () => {
    cronjobSetSuspendMock.mockResolvedValue({ ok: true });
    const suspend = actionByCapability("k8s.cronjobSetSuspend", "Suspend CronJob");
    await suspend.run!(ctx({ kind: "cronjobs", namespace: "ops", name: "nightly" }));
    expect(cronjobSetSuspendMock).toHaveBeenCalledWith("kind-dev", "ops", "nightly", true);
  });

  it("resumes a cronjob", async () => {
    cronjobSetSuspendMock.mockResolvedValue({ ok: true });
    const resume = actionByCapability("k8s.cronjobSetSuspend", "Resume CronJob");
    await resume.run!(ctx({ kind: "cronjobs", namespace: "ops", name: "nightly" }));
    expect(cronjobSetSuspendMock).toHaveBeenCalledWith("kind-dev", "ops", "nightly", false);
  });

  it("triggers a cronjob run now", async () => {
    cronjobTriggerNowMock.mockResolvedValue({ jobName: "nightly-123" });
    const runNow = actionByCapability("k8s.cronjobTriggerNow");
    await runNow.run!(ctx({ kind: "cronjobs", namespace: "ops", name: "nightly" }));
    expect(cronjobTriggerNowMock).toHaveBeenCalledWith("kind-dev", "ops", "nightly");
  });

  it("evicts a pod", async () => {
    evictPodMock.mockResolvedValue({ ok: true });
    const evict = actionByCapability("k8s.evictPod");
    await evict.run!(ctx({ kind: "pods", namespace: "default", name: "web-1" }));
    expect(evictPodMock).toHaveBeenCalledWith("kind-dev", "default", "web-1");
  });

  it("force deletes a pod", async () => {
    deletePodMock.mockResolvedValue({ deleted: true });
    const del = actionByCapability("k8s.deletePod");
    await del.run!(ctx({ kind: "pods", namespace: "default", name: "web-1" }));
    expect(deletePodMock).toHaveBeenCalledWith("kind-dev", "default", "web-1");
  });

  it("drains a node", async () => {
    drainNodeMock.mockResolvedValue({ evicted: 3, skipped: 0 });
    const drain = actionByCapability("k8s.drainNode");
    await drain.run!(ctx({ kind: "nodes", namespace: null, name: "node-a" }));
    expect(drainNodeMock).toHaveBeenCalledWith("kind-dev", "node-a");
  });

  it("creates a node debug pod", async () => {
    createNodeDebugPodMock.mockResolvedValue({ namespace: "kube-system", pod: "node-debugger-abc" });
    const debugNode = actionByCapability("k8s.createNodeDebugPod");
    await debugNode.run!(ctx({ kind: "nodes", namespace: null, name: "node-a" }));
    expect(createNodeDebugPodMock).toHaveBeenCalledWith("kind-dev", "node-a");
  });
});

describe("appPaletteCommands (#544)", () => {
  const manifest = {
    id: "org.example.gitops", name: "GitOps", version: "0.1.0", srelensApiVersion: "^0.3", kind: "declarative",
    permissions: [],
    capabilities: [
      { name: "apps", title: "Apps", target: "k8s.listCustomResource", arguments: { group: "argoproj.io", kind: "Application" }, inputs: [] },
      { name: "projects", title: "Projects", target: "k8s.listCustomResource", arguments: { group: "argoproj.io", kind: "AppProject" }, inputs: [] },
    ],
    actions: [
      { name: "sync", title: "Sync", target: "k8s.mergePatch", resource: "apps", arguments: {} },
      { name: "touch", title: "Touch", target: "k8s.annotate", resource: "projects", arguments: {} },
    ],
    contributions: {
      pages: [{ id: "apps", title: "Applications", capability: "apps" }, { id: "projects", title: "Projects", capability: "projects" }],
      detailTabs: [], detailLinks: [],
      commands: [
        { id: "open", title: "Open applications", target: { page: "apps" } },
        { id: "sync", title: "Sync application", target: { action: "sync" }, forKinds: ["argoproj.io/Application"] },
      ],
    },
  } as ExtensionManifest;
  const app = { id: manifest.id, name: "Argo CD", manifest };
  const withCommands = (commands: NonNullable<ExtensionManifest["contributions"]["commands"]>) =>
    ({ ...app, manifest: { ...manifest, contributions: { ...manifest.contributions, commands } } });

  it("offers page commands with no resource open, under the host's name for the app", () => {
    const [open, ...rest] = appPaletteCommands(app, null);
    expect(rest).toEqual([]);
    expect(open).toEqual({ id: "org.example.gitops/open", label: "Argo CD: Open applications", target: { kind: "page", page: "apps" } });
  });

  it("offers an action command only on a resource of the kind its action acts on", () => {
    expect(appPaletteCommands(app, { capability: "projects" }).map((c) => c.target.kind)).toEqual(["page"]);
    expect(appPaletteCommands(app, { capability: "apps" })[1]).toEqual({
      id: "org.example.gitops/sync", label: "Argo CD: Sync application",
      target: { kind: "action", action: "sync" }, capabilityId: "extensions.action",
    });
  });

  it("refuses an action command whose forKinds do not name that kind", () => {
    const wrong = withCommands([{ id: "sync", title: "Sync", target: { action: "sync" }, forKinds: ["argoproj.io/AppProject"] }]);
    expect(appPaletteCommands(wrong, { capability: "apps" })).toEqual([]);
  });

  it("draws nothing for a command naming an undeclared page or action", () => {
    const broken = withCommands([
      { id: "a", title: "A", target: { page: "gone" } },
      { id: "b", title: "B", target: { action: "gone" }, forKinds: ["argoproj.io/Application"] },
    ]);
    expect(appPaletteCommands(broken, { capability: "apps" })).toEqual([]);
  });
});
