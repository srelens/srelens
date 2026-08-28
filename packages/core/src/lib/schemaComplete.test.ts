import { describe, it, expect } from "vitest";
import {
  extractApiVersionKind,
  pathAtCursor,
  fieldCompletions,
  valueCompletions,
} from "./schemaComplete";
import type { SchemaBundle } from "./schema";

// A tiny Deployment-shaped schema bundle (root → spec → template.spec.containers[]).
const bundle: SchemaBundle = {
  key: "Deployment",
  schemas: {
    Deployment: {
      properties: {
        apiVersion: { type: "string" },
        kind: { type: "string" },
        metadata: { type: "object" },
        // k8s v3 wraps object property refs in allOf — the case that was broken.
        spec: { description: "the spec", allOf: [{ $ref: "#/components/schemas/DeploymentSpec" }] },
      },
    },
    DeploymentSpec: {
      properties: {
        replicas: { type: "integer", description: "desired replicas" },
        paused: { type: "boolean" },
        template: { allOf: [{ $ref: "#/components/schemas/PodTemplateSpec" }] },
      },
    },
    PodTemplateSpec: { properties: { spec: { allOf: [{ $ref: "#/components/schemas/PodSpec" }] } } },
    PodSpec: {
      properties: {
        containers: { type: "array", items: { $ref: "#/components/schemas/Container" } },
      },
    },
    Container: {
      properties: {
        name: { type: "string" },
        image: { type: "string" },
        imagePullPolicy: { type: "string", enum: ["Always", "IfNotPresent", "Never"] },
      },
    },
  },
};

describe("extractApiVersionKind", () => {
  it("reads apiVersion and kind regardless of order", () => {
    expect(extractApiVersionKind("kind: Deployment\napiVersion: apps/v1\n")).toEqual({
      apiVersion: "apps/v1",
      kind: "Deployment",
    });
    expect(extractApiVersionKind("metadata:\n  name: x\n")).toBeNull();
  });
});

describe("pathAtCursor", () => {
  it("builds the mapping path from indentation", () => {
    const text = "spec:\n  template:\n    spec:\n      ";
    const { path, onValue } = pathAtCursor(text, text.length);
    expect(path).toEqual(["spec", "template", "spec"]);
    expect(onValue).toBe(false);
  });

  it("detects a value position", () => {
    const text = "spec:\n  paused: t";
    const { onValue, valueKey } = pathAtCursor(text, text.length);
    expect(onValue).toBe(true);
    expect(valueKey).toBe("paused");
  });
});

describe("fieldCompletions", () => {
  it("suggests top-level fields", () => {
    const labels = fieldCompletions(bundle, []).map((f) => f.label);
    expect(labels).toContain("spec");
    expect(labels).toContain("metadata");
  });

  it("resolves through $ref to nested fields", () => {
    const fields = fieldCompletions(bundle, ["spec"]);
    const labels = fields.map((f) => f.label);
    expect(labels).toEqual(expect.arrayContaining(["replicas", "template", "paused"]));
    expect(fields.find((f) => f.label === "replicas")?.detail).toBe("integer");
    expect(fields.find((f) => f.label === "replicas")?.info).toBe("desired replicas");
  });

  it("descends into array item types", () => {
    const labels = fieldCompletions(bundle, ["spec", "template", "spec", "containers"]).map((f) => f.label);
    expect(labels).toEqual(expect.arrayContaining(["name", "image", "imagePullPolicy"]));
  });
});

describe("valueCompletions", () => {
  it("suggests enum values", () => {
    const values = valueCompletions(bundle, ["spec", "template", "spec", "containers"], "imagePullPolicy").map(
      (v) => v.label,
    );
    expect(values).toEqual(["Always", "IfNotPresent", "Never"]);
  });

  it("suggests booleans", () => {
    const values = valueCompletions(bundle, ["spec"], "paused").map((v) => v.label);
    expect(values).toEqual(["true", "false"]);
  });
});
