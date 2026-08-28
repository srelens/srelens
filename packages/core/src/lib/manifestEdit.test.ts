import { describe, it, expect, vi } from "vitest";
import { secretToStringData, loadEditableManifest } from "./manifestEdit";
import { parse } from "yaml";

describe("secretToStringData", () => {
  it("replaces redacted base64 data with decoded stringData, keeping other fields", () => {
    const manifest = `apiVersion: v1
kind: Secret
metadata:
  name: web-tls
  namespace: default
  labels:
    app: web
type: Opaque
data:
  token: ""
`;
    // getSecret returns base64 values; "aGVsbG8=" decodes to "hello".
    const out = secretToStringData(manifest, { token: "aGVsbG8=" });
    const doc = parse(out);
    expect(doc.data).toBeUndefined();
    expect(doc.stringData).toEqual({ token: "hello" });
    // Untouched fields survive.
    expect(doc.metadata.labels).toEqual({ app: "web" });
    expect(doc.type).toBe("Opaque");
  });
});

describe("loadEditableManifest", () => {
  it("returns the manifest verbatim for a non-secret kind", async () => {
    const getManifest = vi.fn().mockResolvedValue({ yaml: "kind: ConfigMap\n" });
    const getSecret = vi.fn();
    const out = await loadEditableManifest("kind-dev", "ConfigMap", "default", "cm", { getManifest, getSecret });
    expect(out.yaml).toBe("kind: ConfigMap\n");
    expect(getSecret).not.toHaveBeenCalled();
  });

  it("fetches decoded values via getSecret and inlines them as stringData for a Secret", async () => {
    const getManifest = vi.fn().mockResolvedValue({
      yaml: "apiVersion: v1\nkind: Secret\nmetadata:\n  name: s\n  namespace: default\ndata:\n  token: \"\"\n",
    });
    const getSecret = vi.fn().mockResolvedValue({ data: { token: "aGVsbG8=" } });
    const out = await loadEditableManifest("kind-dev", "Secret", "default", "s", { getManifest, getSecret });
    expect(getSecret).toHaveBeenCalledWith("kind-dev", "default", "s");
    expect(parse(out.yaml!).stringData).toEqual({ token: "hello" });
  });

  it("surfaces a getManifest error", async () => {
    const getManifest = vi.fn().mockResolvedValue({ error: "boom" });
    const out = await loadEditableManifest("x", "Pod", "default", "p", { getManifest, getSecret: vi.fn() });
    expect(out.error).toBe("boom");
  });
});
