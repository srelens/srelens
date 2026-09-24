import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../transport/transport")>();
  return {
    ...actual,
    invokeCommand: invokeCommandMock,
    subscribe: subscribeMock,
  };
});

import { helmUpgrade, helmRollback, helmVersion, helmSearchRepo, diffTextLines, startHelmOp, getHelmRelease, listHelmReleases, listHelmReleasesIn } from "./helm";

beforeEach(() => {
  invokeCommandMock.mockReset();
  subscribeMock.mockReset();
});

describe("helm write wrappers", () => {
  it("helmUpgrade passes context, name, chart, namespace, values", async () => {
    const invoke = vi.fn().mockResolvedValue({ output: "ok" });
    const r = await helmUpgrade("ctx", { name: "web", chart: "bitnami/nginx", namespace: "apps", values: "a: 1" }, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.helmUpgrade", {
      context: "ctx", name: "web", chart: "bitnami/nginx", namespace: "apps", values: "a: 1", version: null,
    });
    expect(r.output).toBe("ok");
  });

  it("helmUpgrade passes a given version", async () => {
    const invoke = vi.fn().mockResolvedValue({ output: "ok" });
    await helmUpgrade("ctx", { name: "web", chart: "bitnami/nginx", namespace: "apps", values: "a: 1", version: "18.1.0" }, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.helmUpgrade", {
      context: "ctx", name: "web", chart: "bitnami/nginx", namespace: "apps", values: "a: 1", version: "18.1.0",
    });
  });

  it("helmRollback passes revision", async () => {
    const invoke = vi.fn().mockResolvedValue({ output: "rolled back" });
    await helmRollback("ctx", { name: "web", revision: 3, namespace: "apps" }, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.helmRollback", { context: "ctx", name: "web", revision: 3, namespace: "apps" });
  });

  it("helmVersion returns the version string", async () => {
    const invoke = vi.fn().mockResolvedValue({ version: "v3.14.0" });
    expect((await helmVersion("ctx", invoke)).version).toBe("v3.14.0");
  });

  it("helmVersion surfaces errors", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("helm not found on PATH"));
    const r = await helmVersion("ctx", invoke);
    expect(r.error).toContain("helm not found");
  });

  it("helmSearchRepo passes context and chart, returns entries", async () => {
    const entries = [{ name: "bitnami/nginx", version: "18.1.0", appVersion: "1.27.0", description: "d" }];
    const invoke = vi.fn().mockResolvedValue({ entries });
    const r = await helmSearchRepo("ctx", "nginx", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.helmSearchRepo", { context: "ctx", chart: "nginx" });
    expect(r.entries).toEqual(entries);
  });

  it("helmSearchRepo surfaces errors", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("helm not found on PATH"));
    const r = await helmSearchRepo("ctx", "nginx", invoke);
    expect(r.error).toContain("helm not found");
  });
});

describe("getHelmRelease", () => {
  it("omits the revision key entirely when none is given", async () => {
    const invoke = vi.fn().mockResolvedValue({ name: "web" });
    await getHelmRelease("ctx", "apps", "web", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.getHelmRelease", { context: "ctx", namespace: "apps", name: "web" });
    const payload = invoke.mock.calls[0][1] as Record<string, unknown>;
    expect("revision" in payload).toBe(false);
  });

  it("passes a given revision through to the payload", async () => {
    const invoke = vi.fn().mockResolvedValue({ name: "web" });
    await getHelmRelease("ctx", "apps", "web", invoke, 118);
    expect(invoke).toHaveBeenCalledWith("k8s.getHelmRelease", { context: "ctx", namespace: "apps", name: "web", revision: 118 });
  });

  it("surfaces errors", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("release not found"));
    const r = await getHelmRelease("ctx", "apps", "web", invoke);
    expect(r.error).toContain("release not found");
  });
});

describe("startHelmOp", () => {
  it("forwards values to start_helm_op", async () => {
    invokeCommandMock.mockResolvedValue(1);
    subscribeMock.mockResolvedValue(() => {});

    await startHelmOp("ctx", ["upgrade", "web", "c"], () => {}, () => {}, [], "replicaCount: 2");

    expect(invokeCommandMock).toHaveBeenCalledWith(
      "start_helm_op",
      expect.objectContaining({ context: "ctx", args: ["upgrade", "web", "c"], values: "replicaCount: 2" }),
    );
  });

  it("defaults values to empty string when omitted", async () => {
    invokeCommandMock.mockResolvedValue(2);
    subscribeMock.mockResolvedValue(() => {});

    await startHelmOp("ctx", ["uninstall", "web"], () => {}, () => {});

    expect(invokeCommandMock).toHaveBeenCalledWith(
      "start_helm_op",
      expect.objectContaining({ values: "" }),
    );
  });
});

describe("diffTextLines", () => {
  it("marks equal lines same and changed lines replace/insert/delete", () => {
    const rows = diffTextLines("a\nb\nc", "a\nB\nc\nd");
    expect(rows[0]).toEqual({ tag: "same", left: "a", right: "a" });
    expect(rows.some((r) => r.tag !== "same")).toBe(true);
    // last proposed line "d" has no left counterpart
    expect(rows.some((r) => r.tag === "insert" && r.right === "d")).toBe(true);
  });

  it("all-same when identical", () => {
    const rows = diffTextLines("x\ny", "x\ny");
    expect(rows.every((r) => r.tag === "same")).toBe(true);
  });

  it("falls back to an index-aligned diff above MAX_LCS_LINES and returns promptly", () => {
    const size = 2500;
    const leftLines = Array.from({ length: size }, (_, i) => `line ${i}`);
    const rightLines = Array.from({ length: size }, (_, i) => `line ${i}`);
    rightLines[1234] = "line 1234 (changed)";

    const start = Date.now();
    const rows = diffTextLines(leftLines.join("\n"), rightLines.join("\n"));
    expect(Date.now() - start).toBeLessThan(1000);

    expect(rows.length).toBe(size);
    expect(rows[1234]).toEqual({ tag: "replace", left: "line 1234", right: "line 1234 (changed)" });
    expect(rows[0]).toEqual({ tag: "same", left: "line 0", right: "line 0" });
  });
});

// #688: `helm list --namespace` takes one namespace, and "all" reads release
// Secrets cluster-wide — which a namespace-scoped credential is refused.
describe("listHelmReleasesIn", () => {
  const rel = (name: string, namespace: string) => ({
    name, namespace, revision: 1, status: "deployed", updated: "", chart: "c", chartVersion: "1", appVersion: "", description: "",
  });
  // A capability double run through the REAL `listHelmReleases`, so a thrown
  // refusal reaches the fan-out exactly as `listHelmReleases` words it.
  const invokerFor = (answers: Record<string, () => unknown>) => {
    const invoke = vi.fn(async (_cap: string, input: unknown) => answers[(input as { namespace: string }).namespace]());
    const list = (context: string, namespace: string | null) =>
      listHelmReleases(context, namespace, invoke as unknown as Parameters<typeof listHelmReleases>[2]);
    return Object.assign(list, { mock: invoke.mock });
  };

  it("lists each selected namespace on its own and merges the releases", async () => {
    const invoke = invokerFor({
      "team-a": () => ({ releases: [rel("api", "team-a")] }),
      "team-b": () => ({ releases: [rel("web", "team-b")] }),
    });
    const out = await listHelmReleasesIn("c", ["team-a", "team-b"], invoke);
    expect(invoke.mock.calls.map((c) => (c[1] as { namespace: string }).namespace))
      .toEqual(["team-a", "team-b"]);
    expect(out.releases?.map((r) => r.name)).toEqual(["api", "web"]);
    expect(out.failures).toEqual([]);
    expect(out.error).toBeUndefined();
  });

  it("asks for every namespace at once for an empty selection", async () => {
    const invoke = invokerFor({ "": () => ({ releases: [rel("api", "team-a")] }) });
    const out = await listHelmReleasesIn("c", [], invoke);
    expect(out.releases).toHaveLength(1);
  });

  it("keeps the namespaces that answered and names the one that was refused", async () => {
    const invoke = invokerFor({
      "team-a": () => ({ releases: [rel("api", "team-a")] }),
      "team-b": () => {
        throw new Error("secrets is forbidden");
      },
    });
    const out = await listHelmReleasesIn("c", ["team-a", "team-b"], invoke);
    expect(out.releases?.map((r) => r.name)).toEqual(["api"]);
    expect(out.failures).toEqual([{ namespace: "team-b", error: "Error: secrets is forbidden" }]);
    expect(out.error).toBeUndefined();
  });

  it("is the list's error when every namespace was refused", async () => {
    const boom = () => {
      throw new Error("secrets is forbidden");
    };
    const out = await listHelmReleasesIn("c", ["team-a", "team-b"], invokerFor({ "team-a": boom, "team-b": boom }));
    expect(out.releases).toBeUndefined();
    expect(out.error).toBe("Error: secrets is forbidden");
  });

  it("reports no namespace failures for a one-namespace listing", async () => {
    const out = await listHelmReleasesIn("c", ["team-a"], invokerFor({
      "team-a": () => {
        throw new Error("secrets is forbidden");
      },
    }));
    expect(out.error).toBe("Error: secrets is forbidden");
    expect(out.failures).toEqual([]);
  });
});
