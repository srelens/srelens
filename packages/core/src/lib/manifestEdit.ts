import { parse, stringify } from "yaml";
import { getManifest as defaultGetManifest, getSecret as defaultGetSecret } from "./manifest";

/** Decode a base64 string to UTF-8 text (returns the input unchanged on failure). */
function decodeBase64(value: string): string {
  try {
    const binary = atob(value);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return value;
  }
}

/**
 * Rewrite a Secret manifest so its (redacted) base64 `data` becomes decoded,
 * editable `stringData`. On apply, Kubernetes re-encodes `stringData`, so this
 * keeps values round-trippable without exposing base64 in the editor.
 */
export function secretToStringData(manifestYaml: string, data: Record<string, string>): string {
  const doc = (parse(manifestYaml) ?? {}) as Record<string, unknown>;
  delete doc.data;
  const stringData: Record<string, string> = {};
  for (const [key, value] of Object.entries(data)) {
    stringData[key] = decodeBase64(value);
  }
  doc.stringData = stringData;
  return stringify(doc);
}

interface Deps {
  getManifest?: typeof defaultGetManifest;
  getSecret?: typeof defaultGetSecret;
}

/**
 * Load a resource's manifest for the Edit tab. Most kinds come straight from
 * `k8s.getManifest`. Secrets are special: `getManifest` redacts their values,
 * so we fetch the real (base64) values through the gated `k8s.getSecret` and
 * inline them as decoded `stringData` — keeping the sensitive path intact while
 * making the Secret genuinely editable.
 */
export async function loadEditableManifest(
  context: string,
  kind: string,
  namespace: string | null,
  name: string,
  deps: Deps = {},
): Promise<{ yaml?: string; error?: string }> {
  const getManifest = deps.getManifest ?? defaultGetManifest;
  const getSecret = deps.getSecret ?? defaultGetSecret;

  const out = await getManifest(context, kind, namespace, name);
  if (out.error) return { error: out.error };
  let manifestYaml = out.yaml ?? "";

  if (kind === "Secret") {
    const secret = await getSecret(context, namespace ?? "", name);
    if (secret.error) return { error: secret.error };
    manifestYaml = secretToStringData(manifestYaml, secret.data ?? {});
  }

  return { yaml: manifestYaml };
}
