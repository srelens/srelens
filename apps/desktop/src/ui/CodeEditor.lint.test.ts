import { describe, it, expect } from "vitest";
import { yamlDiagnostics, k8sDiagnostics } from "./CodeEditor";

describe("yamlDiagnostics", () => {
  it("returns no diagnostics for valid YAML", () => {
    const yaml = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app\n";
    expect(yamlDiagnostics(yaml)).toHaveLength(0);
  });

  it("returns none for empty/whitespace input", () => {
    expect(yamlDiagnostics("")).toHaveLength(0);
    expect(yamlDiagnostics("   \n  ")).toHaveLength(0);
  });

  it("flags a YAML syntax error with a position and message", () => {
    // Nested mapping in a compact/inline position is invalid YAML.
    const bad = "spec:\n  foo: bar: baz\n";
    const diags = yamlDiagnostics(bad);
    expect(diags.length).toBeGreaterThan(0);
    expect(diags[0].severity).toBe("error");
    expect(diags[0].to).toBeGreaterThan(diags[0].from);
    expect(typeof diags[0].message).toBe("string");
  });

  it("flags an unterminated flow sequence", () => {
    const diags = yamlDiagnostics("ports: [80, 443\n");
    expect(diags.some((d) => d.severity === "error")).toBe(true);
  });

  it("returns no diagnostics for a valid two-document manifest", () => {
    const doc1 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app\n";
    const doc2 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app-2\n";
    const twoDoc = `${doc1}---\n${doc2}`;
    expect(yamlDiagnostics(twoDoc)).toHaveLength(0);
  });

  it("locates a syntax error in the SECOND document of a multi-doc manifest", () => {
    const doc1 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app\n";
    const separator = "---\n";
    const badDoc2 = "spec:\n  foo: bar: baz\n";
    const twoDoc = `${doc1}${separator}${badDoc2}`;
    const secondDocOffset = doc1.length + separator.length;
    const diags = yamlDiagnostics(twoDoc);
    expect(diags.length).toBeGreaterThan(0);
    expect(diags[0].severity).toBe("error");
    // The offending line lives inside the second document, not at/before the `---`.
    expect(diags[0].from).toBeGreaterThanOrEqual(secondDocOffset);
  });
});

const manifest = `apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-app
spec:
  replicas: -1
  foo: bar
`;

describe("k8sDiagnostics (server validation → editor positions)", () => {
  it("positions an unknown-field error at that field in the YAML", () => {
    const diags = k8sDiagnostics(manifest, [{ docIndex: 0, message: 'strict decoding error: unknown field "spec.foo"' }]);
    expect(diags).toHaveLength(1);
    // The range should cover the `foo: bar` value, not the top of the doc.
    const at = manifest.indexOf("bar");
    expect(diags[0].from).toBeLessThanOrEqual(at);
    expect(diags[0].to).toBeGreaterThanOrEqual(at);
    expect(diags[0].message).toContain("unknown field");
  });

  it("positions an invalid-value error at the offending field", () => {
    const diags = k8sDiagnostics(manifest, [
      { docIndex: 0, message: 'Deployment.apps "my-app" is invalid: spec.replicas: Invalid value: -1: must be >= 0' },
    ]);
    expect(diags).toHaveLength(1);
    const at = manifest.indexOf("-1");
    expect(diags[0].from).toBeLessThanOrEqual(at);
    expect(diags[0].to).toBeGreaterThanOrEqual(at);
  });

  it("falls back to the top of the document when no field can be located", () => {
    const diags = k8sDiagnostics(manifest, [{ docIndex: 0, message: "admission webhook denied the request" }]);
    expect(diags).toHaveLength(1);
    expect(diags[0].from).toBe(0);
    expect(diags[0].message).toContain("admission webhook");
  });

  it("returns nothing for no messages or empty text", () => {
    expect(k8sDiagnostics(manifest, [])).toHaveLength(0);
    expect(k8sDiagnostics("", [{ docIndex: 0, message: "some error" }])).toHaveLength(0);
  });

  it("maps a field that exists only in the SECOND document to a range inside it", () => {
    const doc1 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app\nspec:\n  replicas: 1\n";
    const separator = "---\n";
    const doc2 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app-2\nspec:\n  replicas: 1\n  foo: bar\n";
    const twoDoc = `${doc1}${separator}${doc2}`;
    const separatorOffset = doc1.length;
    const diags = k8sDiagnostics(twoDoc, [{ docIndex: 1, message: 'strict decoding error: unknown field "spec.foo"' }]);
    expect(diags).toHaveLength(1);
    expect(diags[0].from).toBeGreaterThan(separatorOffset);
    const at = twoDoc.indexOf("bar");
    expect(diags[0].from).toBeLessThanOrEqual(at);
    expect(diags[0].to).toBeGreaterThanOrEqual(at);
  });

  it("uses docIndex to target the SECOND document when the field exists in BOTH", () => {
    // `spec.replicas` is present in BOTH documents. An error tagged docIndex:1
    // must land in the SECOND document, not the first — a first-match-wins
    // implementation would wrongly point at doc 1's replicas.
    const doc1 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app\nspec:\n  replicas: 1\n";
    const separator = "---\n";
    const doc2 = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: my-app-2\nspec:\n  replicas: 2\n";
    const twoDoc = `${doc1}${separator}${doc2}`;
    const separatorOffset = doc1.length;
    const diags = k8sDiagnostics(twoDoc, [
      { docIndex: 1, message: 'Deployment.apps "my-app-2" is invalid: spec.replicas: Invalid value: 2: bad' },
    ]);
    expect(diags).toHaveLength(1);
    // Must point into the SECOND document (past the `---`), i.e. at doc2's replicas.
    expect(diags[0].from).toBeGreaterThan(separatorOffset);
    const secondReplicas = twoDoc.indexOf("replicas: 2");
    expect(diags[0].from).toBeGreaterThanOrEqual(secondReplicas);
  });
});
