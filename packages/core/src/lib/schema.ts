import { invokeCapability, type Invoker } from "../transport/transport";

/** A resource's OpenAPI schema plus the referenced type schemas, keyed. */
export interface SchemaBundle {
  schemas: Record<string, unknown>;
  key: string | null;
}

/** Fetch the OpenAPI schema for a kind via `k8s.openApiSchema`. */
export async function openApiSchema(
  context: string,
  apiVersion: string,
  kind: string,
  invoke: Invoker = invokeCapability,
): Promise<SchemaBundle | { error: string }> {
  try {
    const out = await invoke<{ schemas: string; key: string | null }>("k8s.openApiSchema", {
      context,
      apiVersion,
      kind,
    });
    return { schemas: JSON.parse(out.schemas) as Record<string, unknown>, key: out.key };
  } catch (e) {
    return { error: String(e) };
  }
}
