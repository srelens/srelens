import { describe, it, expect, vi } from "vitest";
import {
  getManifest,
  listNodes,
  applyManifest,
  diffManifest,
  parseResourceVersion,
  listEvents,
  listResource,
  redactSecretManifest,
} from "./manifest";

describe("getManifest", () => {
  it("passes kind/namespace/name and returns yaml", async () => {
    const invoke = vi.fn().mockResolvedValue({ yaml: "kind: Pod\n" });
    const out = await getManifest("kind-dev", "Pod", "default", "web-1", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.getManifest", {
      context: "kind-dev",
      kind: "Pod",
      namespace: "default",
      name: "web-1",
    });
    expect(out.yaml).toContain("kind: Pod");
  });

  it("normalises errors", async () => {
    const out = await getManifest("c", "Pod", null, "x", () =>
      Promise.reject(new Error("not found")),
    );
    expect(out.error).toContain("not found");
  });
});

describe("listNodes", () => {
  it("returns node summaries", async () => {
    const invoke = vi.fn().mockResolvedValue({
      nodes: [{ name: "cp", status: "Ready", version: "v1.35.0", roles: "control-plane" }],
    });
    const out = await listNodes("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listNodes", { context: "kind-dev" });
    expect(out.nodes?.[0].status).toBe("Ready");
  });

  it("normalises errors", async () => {
    const out = await listNodes("c", () => Promise.reject(new Error("forbidden")));
    expect(out.error).toContain("forbidden");
  });
});

describe("listEvents", () => {
  it("passes an exact involved-object filter", async () => {
    const invoke = vi.fn().mockResolvedValue({ events: [] });
    await listEvents("kind-dev", "default", { kind: "Pod", name: "web-1" }, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listEvents", {
      context: "kind-dev",
      namespace: "default",
      objectKind: "Pod",
      objectName: "web-1",
    });
  });

  it("carries an event's repeat count through", async () => {
    const invoke = async () => ({
      events: [
        {
          name: "web.17a",
          type: "Warning",
          reason: "BackOff",
          object: "pod/web",
          message: "m",
          age: "12s",
          count: 37,
        },
      ],
    });
    const out = await listEvents("ctx", "ns", undefined, invoke as never);
    expect(out.events?.[0].count).toBe(37);
  });

  it("carries the namespace an event came from, empty for a cluster-scoped one", async () => {
    const invoke = async () => ({
      events: [
        { name: "shop/web.17a", namespace: "shop", type: "Normal", reason: "Pulled", object: "pod/web", message: "m", age: "4m", count: 1 },
        { name: "node-a.17b", namespace: "", type: "Warning", reason: "NodeNotReady", object: "node/node-a", message: "m", age: "1m", count: 1 },
      ],
    });
    const out = await listEvents("ctx", null, undefined, invoke as never);
    expect(out.events?.map((e) => e.namespace)).toEqual(["shop", ""]);
  });
});

describe("applyManifest", () => {
  it("passes context+yaml and returns applied", async () => {
    const invoke = vi.fn().mockResolvedValue({ documents: [{ kind: "ConfigMap", name: "cm", applied: true, conflict: null, error: null }], applied: true });
    const out = await applyManifest("kind-dev", "kind: ConfigMap\n", false, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.applyManifest", {
      context: "kind-dev",
      yaml: "kind: ConfigMap\n",
      force: false,
    });
    expect(out.applied).toBe(true);
  });

  it("normalises errors", async () => {
    const out = await applyManifest("c", "bad", false, () => Promise.reject(new Error("invalid")));
    expect(out.error).toContain("invalid");
  });
});

describe("listResource", () => {
  it("passes kind+namespace and returns items", async () => {
    const invoke = vi.fn().mockResolvedValue({
      items: [{ name: "cm1", namespace: "default" }],
    });
    const out = await listResource("kind-dev", "ConfigMap", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listResource", {
      context: "kind-dev",
      kind: "ConfigMap",
      namespace: "default",
    });
    expect(out.items?.[0].name).toBe("cm1");
  });

  it("normalises errors", async () => {
    const out = await listResource("c", "Secret", "default", () =>
      Promise.reject(new Error("forbidden")),
    );
    expect(out.error).toContain("forbidden");
  });
});

describe("applyManifest force + multi-doc", () => {
  it("passes force and returns per-document results", async () => {
    const invoke = vi.fn().mockResolvedValue({
      documents: [{ kind: "ConfigMap", name: "a", applied: true, conflict: null, error: null }],
      applied: true,
    });
    const out = await applyManifest("ctx", "kind: ConfigMap", true, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.applyManifest", { context: "ctx", yaml: "kind: ConfigMap", force: true });
    expect(out.applied).toBe(true);
    expect(out.documents?.[0].name).toBe("a");
  });

  it("defaults force to false", async () => {
    const invoke = vi.fn().mockResolvedValue({ documents: [], applied: true });
    await applyManifest("ctx", "kind: ConfigMap", undefined, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.applyManifest", { context: "ctx", yaml: "kind: ConfigMap", force: false });
  });

  it("surfaces call errors", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("boom"));
    const out = await applyManifest("ctx", "x", false, invoke);
    expect(out.error).toContain("boom");
  });
});

describe("diffManifest", () => {
  it("returns documents", async () => {
    const invoke = vi.fn().mockResolvedValue({
      documents: [{ kind: "ConfigMap", name: "a", namespace: "d", exists: true, changed: true, rows: [], currentResourceVersion: "9" }],
    });
    const out = await diffManifest("ctx", "kind: ConfigMap", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.diffManifest", { context: "ctx", yaml: "kind: ConfigMap" });
    expect(out.documents?.[0].currentResourceVersion).toBe("9");
  });
});

describe("parseResourceVersion", () => {
  it("reads metadata.resourceVersion", () => {
    expect(parseResourceVersion("metadata:\n  resourceVersion: \"42\"\n")).toBe("42");
  });
  it("returns null when absent", () => {
    expect(parseResourceVersion("metadata:\n  name: a\n")).toBeNull();
  });
});

// Obviously-fake fixture text — never anything that reads as a real
// credential, per this screen's secrecy ruling.
const FIXTURE_VALUE = "ZmFrZS1maXh0dXJlLW5vdC1hLXJlYWwtc2VjcmV0";
const FIXTURE_VALUE_2 = "ZmFrZS1maXh0dXJlLXNlY29uZC12YWx1ZQ==";

function secretManifest(body: string): string {
  return `apiVersion: v1\nkind: Secret\nmetadata:\n  name: s-1\n  namespace: default\n${body}`;
}

describe("redactSecretManifest", () => {
  it("replaces every value under the top-level data map, keeping the keys", () => {
    const out = redactSecretManifest(
      secretManifest(`data:\n  token: ${FIXTURE_VALUE}\n  other: ${FIXTURE_VALUE_2}\n`),
    );
    expect(out.error).toBeUndefined();
    expect(out.yaml).not.toContain(FIXTURE_VALUE);
    expect(out.yaml).not.toContain(FIXTURE_VALUE_2);
    // The shape is still readable: the keys say what the Secret holds.
    expect(out.yaml).toContain("token:");
    expect(out.yaml).toContain("other:");
    expect(out.yaml).toContain("REDACTED");
  });

  it("replaces every value under the top-level stringData map too", () => {
    const out = redactSecretManifest(secretManifest(`stringData:\n  plain: ${FIXTURE_VALUE}\n`));
    expect(out.error).toBeUndefined();
    expect(out.yaml).not.toContain(FIXTURE_VALUE);
    expect(out.yaml).toContain("plain:");
  });

  it("does not preserve a value's length, which would leak its size", () => {
    const short = redactSecretManifest(secretManifest("data:\n  k: YQ==\n"));
    const long = redactSecretManifest(
      secretManifest(`data:\n  k: ${"QUFB".repeat(40)}\n`),
    );
    expect(short.yaml).toEqual(long.yaml);
  });

  it("leaves a `data` key that is not the Secret's own alone", () => {
    const out = redactSecretManifest(
      `apiVersion: v1\nkind: Secret\nmetadata:\n  name: s-1\n  annotations:\n    data: annotation-value-not-a-secret\nspec:\n  data:\n    nested: nested-value-not-a-secret\ndata:\n  token: ${FIXTURE_VALUE}\n`,
    );
    expect(out.error).toBeUndefined();
    // `spec.data.nested` is the scoping proof: only the TOP-LEVEL `data` is
    // the Secret's value map. The annotation named `data` keeps its key but
    // loses its value — not because it is called `data`, but because every
    // annotation value on a Secret is blanked; see the test above.
    expect(out.yaml).toContain("nested-value-not-a-secret");
    expect(out.yaml).toContain("    data: REDACTED");
    expect(out.yaml).not.toContain("annotation-value-not-a-secret");
    expect(out.yaml).not.toContain(FIXTURE_VALUE);
  });

  it("blanks the applied-configuration annotation, which carries the whole data map", () => {
    // A `kubectl apply`-managed Secret keeps the ENTIRE manifest it last
    // applied — base64 `data` map included — in this one annotation. Blanking
    // the top-level `data` while leaving this alone puts the same value back
    // on screen two lines further down, under an Alert promising it is not
    // shown. (#331)
    const applied = `{"apiVersion":"v1","kind":"Secret","data":{"token":"${FIXTURE_VALUE}"}}`;
    const out = redactSecretManifest(
      `apiVersion: v1\nkind: Secret\nmetadata:\n  name: s-1\n  annotations:\n    kubectl.kubernetes.io/last-applied-configuration: '${applied}'\ndata:\n  token: ${FIXTURE_VALUE}\n`,
    );
    expect(out.error).toBeUndefined();
    expect(out.yaml).not.toContain(FIXTURE_VALUE);
    // The key stays: the reader still learns the Secret is `kubectl`-managed.
    expect(out.yaml).toContain("kubectl.kubernetes.io/last-applied-configuration:");
  });

  it("blanks every annotation on a Secret, not only the one kubectl writes", () => {
    // Any controller can echo a Secret's material into an annotation of its
    // own — a checksum of the value, a copy for a sidecar, a "previous value"
    // an operator left behind. A rule naming one well-known key would read as
    // complete and cover one carrier out of many.
    const out = redactSecretManifest(
      `apiVersion: v1\nkind: Secret\nmetadata:\n  name: s-1\n  annotations:\n    example.com/copy-of-token: ${FIXTURE_VALUE}\n    example.com/harmless: rotated-by-dana\ndata:\n  token: ${FIXTURE_VALUE_2}\n`,
    );
    expect(out.error).toBeUndefined();
    expect(out.yaml).not.toContain(FIXTURE_VALUE);
    expect(out.yaml).not.toContain(FIXTURE_VALUE_2);
    expect(out.yaml).not.toContain("rotated-by-dana");
    expect(out.yaml).toContain("example.com/copy-of-token:");
    expect(out.yaml).toContain("example.com/harmless:");
  });

  it("leaves labels and the rest of metadata alone, which the annotation rule does not claim", () => {
    // Scope, stated as a test so it cannot drift into a vaguer promise: this
    // covers `data`, `stringData` and annotation VALUES. Labels, names and
    // everything else are printed as the cluster returned them.
    const out = redactSecretManifest(
      `apiVersion: v1\nkind: Secret\nmetadata:\n  name: s-1\n  labels:\n    app: web\n  annotations:\n    a: ${FIXTURE_VALUE}\ndata:\n  token: ${FIXTURE_VALUE_2}\n`,
    );
    expect(out.error).toBeUndefined();
    expect(out.yaml).toContain("app: web");
    expect(out.yaml).toContain("name: s-1");
  });

  it("preserves key order and comments, so the pane shows the cluster's own manifest", () => {
    const out = redactSecretManifest(
      `# top comment\napiVersion: v1\nkind: Secret\nzz: last\nmetadata:\n  name: s-1\ndata:\n  token: ${FIXTURE_VALUE}\n`,
    );
    expect(out.error).toBeUndefined();
    expect(out.yaml).toContain("# top comment");
    const lines = (out.yaml ?? "").split("\n");
    expect(lines.indexOf("apiVersion: v1")).toBeLessThan(lines.indexOf("zz: last"));
    expect(lines.indexOf("zz: last")).toBeLessThan(lines.findIndex((l) => l.startsWith("metadata:")));
  });

  it("passes a Secret with no data at all through as a map, unchanged in substance", () => {
    const out = redactSecretManifest(secretManifest("type: Opaque\n"));
    expect(out.error).toBeUndefined();
    expect(out.yaml).toContain("type: Opaque");
  });

  describe("failing closed", () => {
    it("returns an error, and never the input, for a manifest that does not parse", () => {
      const bad = `data:\n\ttoken: ${FIXTURE_VALUE}\n`;
      const out = redactSecretManifest(bad);
      expect(out.error).toBeTruthy();
      expect(out.yaml).toBeUndefined();
      // The `yaml` package's own parse errors quote the offending source
      // line — which, for a Secret, IS the value. The error is rendered on
      // screen, so it must never carry the manifest's text.
      expect(out.error).not.toContain(FIXTURE_VALUE);
      expect(out.error).not.toContain("token");
    });

    it("returns an error for a document that is not a map", () => {
      for (const input of ["just a string", "- a\n- b\n", ""]) {
        const out = redactSecretManifest(input);
        expect(out.error).toBeTruthy();
        expect(out.yaml).toBeUndefined();
      }
    });

    it("returns an error when `data` is present but is not a map, rather than passing it through", () => {
      const out = redactSecretManifest(secretManifest(`data: ${FIXTURE_VALUE}\n`));
      expect(out.error).toBeTruthy();
      expect(out.yaml).toBeUndefined();
    });

    it("returns an error when an alias could re-expose a redacted value elsewhere in the document", () => {
      const out = redactSecretManifest(
        `kind: Secret\ndata:\n  token: &t ${FIXTURE_VALUE}\nmetadata:\n  annotations:\n    copy: *t\n`,
      );
      expect(out.error).toBeTruthy();
      expect(out.yaml).toBeUndefined();
    });

    it("returns an error when `metadata` or its `annotations` is not a mapping", () => {
      // The annotation rule can only hold if the maps it walks are maps. A
      // `metadata` it cannot read is not "no annotations to blank" — it is a
      // document this does not understand, and reading zero annotations out
      // of it would pass an unredacted manifest through.
      for (const input of [
        `kind: Secret\nmetadata: ${FIXTURE_VALUE}\ndata:\n  token: ${FIXTURE_VALUE_2}\n`,
        `kind: Secret\nmetadata:\n  annotations: ${FIXTURE_VALUE}\ndata:\n  token: ${FIXTURE_VALUE_2}\n`,
      ]) {
        const out = redactSecretManifest(input);
        expect(out.error).toBeTruthy();
        expect(out.yaml).toBeUndefined();
        expect(out.error).not.toContain(FIXTURE_VALUE);
        expect(out.error).not.toContain(FIXTURE_VALUE_2);
      }
    });

    it("never returns partially redacted text alongside an error", () => {
      const out = redactSecretManifest(secretManifest(`data: ${FIXTURE_VALUE}\n`));
      expect(out.yaml).toBeUndefined();
      expect(out.error).not.toContain(FIXTURE_VALUE);
    });
  });
});
