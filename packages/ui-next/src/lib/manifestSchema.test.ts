import { describe, expect, it } from "vitest";
import type { SchemaBundle } from "@srelens/core";
import { keysAt, schemaCompletions } from "./manifestSchema";

// A ConfigMap's schema, cut down: an object ref wrapped in `allOf` the way
// the real document wraps every one, a plain object, a boolean.
const BUNDLE: SchemaBundle = {
  key: "io.k8s.api.core.v1.ConfigMap",
  schemas: {
    "io.k8s.api.core.v1.ConfigMap": {
      type: "object",
      properties: {
        apiVersion: { type: "string", description: "APIVersion defines the versioned schema." },
        kind: { type: "string", description: "Kind is a string value." },
        metadata: {
          description: "Standard object's metadata.",
          allOf: [{ $ref: "#/components/schemas/io.k8s.apimachinery.pkg.apis.meta.v1.ObjectMeta" }],
        },
        data: { type: "object", description: "Data contains the configuration data." },
        immutable: { type: "boolean", description: "Immutable, if set to true, ensures that data cannot be updated." },
      },
    },
    "io.k8s.apimachinery.pkg.apis.meta.v1.ObjectMeta": {
      type: "object",
      properties: {
        name: { type: "string", description: "Name must be unique within a namespace." },
        namespace: { type: "string", description: "Namespace defines the space within which each name must be unique." },
      },
    },
  },
};

const DOC = `apiVersion: v1
kind: ConfigMap
metadata:
  name: web
  na
immutable: 
`;

describe("keysAt", () => {
  it("lists the keys of the mapping the cursor is in, with their types and descriptions", () => {
    const here = keysAt(BUNDLE, DOC, DOC.indexOf("  na") + 4);
    expect(here.path).toEqual(["metadata"]);
    expect(here.onValue).toBe(false);
    expect(here.entries.map((e) => e.label)).toEqual(["name", "namespace"]);
    expect(here.entries[0].detail).toBe("string");
    expect(here.entries[0].info).toContain("unique within a namespace");
  });

  it("lists the top-level keys at the top level", () => {
    const here = keysAt(BUNDLE, DOC, 0);
    expect(here.path).toEqual([]);
    expect(here.entries.map((e) => e.label)).toEqual(["apiVersion", "kind", "metadata", "data", "immutable"]);
  });

  it("switches to a key's values after `key: `, when the key has a fixed set", () => {
    const here = keysAt(BUNDLE, DOC, DOC.indexOf("immutable: ") + "immutable: ".length);
    expect(here.onValue).toBe(true);
    expect(here.valueKey).toBe("immutable");
    expect(here.entries.map((e) => e.label)).toEqual(["true", "false"]);
  });

  it("falls back to the keys beside a key whose values are free text", () => {
    // `name: ` takes any string; an empty "values" section would say less
    // than the keys that belong next to it.
    const doc = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: ";
    const here = keysAt(BUNDLE, doc, doc.length);
    expect(here.onValue).toBe(false);
    expect(here.path).toEqual(["metadata"]);
    expect(here.entries.map((e) => e.label)).toEqual(["name", "namespace"]);
  });

  it("clamps a cursor past the end rather than throwing", () => {
    expect(() => keysAt(BUNDLE, DOC, DOC.length + 50)).not.toThrow();
  });
});

/** A CodeMirror completion context, reduced to what the source reads. */
function context(doc: string, pos: number, explicit = false) {
  const before = doc.slice(0, pos);
  const m = before.match(/[\w.-]*$/)!;
  return {
    pos,
    explicit,
    state: { doc: { toString: () => doc } },
    matchBefore: () => ({ from: pos - m[0].length, to: pos, text: m[0] }),
  } as unknown as Parameters<ReturnType<typeof schemaCompletions>>[0];
}

describe("schemaCompletions", () => {
  it("offers the mapping's field names for the word being typed", () => {
    const source = schemaCompletions(() => BUNDLE);
    const pos = DOC.indexOf("  na") + 4;
    const result = source(context(DOC, pos)) as unknown as { from: number; options: { label: string; detail?: string }[] };
    expect(result.from).toBe(pos - 2);
    expect(result.options.map((o) => o.label)).toEqual(["name", "namespace"]);
    expect(result.options[0].detail).toBe("string");
  });

  it("offers a key's values after `key: `", () => {
    const source = schemaCompletions(() => BUNDLE);
    const pos = DOC.indexOf("immutable: ") + "immutable: ".length;
    const result = source(context(DOC, pos, true)) as unknown as { options: { label: string }[] };
    expect(result.options.map((o) => o.label)).toEqual(["true", "false"]);
  });

  it("stays quiet with nothing typed unless asked, and with no schema at all", () => {
    const source = schemaCompletions(() => BUNDLE);
    const pos = DOC.indexOf("  na") + 2;
    expect(source(context(DOC, pos))).toBeNull();
    expect(source(context(DOC, pos, true))).not.toBeNull();
    expect(schemaCompletions(() => null)(context(DOC, pos, true))).toBeNull();
  });

  it("reads the schema through the getter, so one that arrives later is offered", () => {
    let bundle: SchemaBundle | null = null;
    const source = schemaCompletions(() => bundle);
    const pos = DOC.indexOf("  na") + 4;
    expect(source(context(DOC, pos))).toBeNull();
    bundle = BUNDLE;
    expect(source(context(DOC, pos))).not.toBeNull();
  });
});
