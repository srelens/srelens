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

import { helmUpgrade, helmRollback, helmVersion, helmSearchRepo, diffTextLines, startHelmOp } from "./helm";

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
