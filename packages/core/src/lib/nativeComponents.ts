import Ajv from "ajv";
import schema from "../../../../schemas/native-component.v1.json";

export type NativeTone = "sev" | "warn" | "ok" | "info" | "accent" | "muted";
export type NativeValue = string | number | boolean | null;
export interface NativeComponentData {
  KeyValue: { items: Array<{ label: string; value: NativeValue }> };
  Badge: { label: string; tone: NativeTone };
  Metric: { label: string; value: number | null; unit?: string; description?: string };
  Conditions: { items: Array<{ type: string; status: "True" | "False" | "Unknown"; reason?: string; message?: string; observedGeneration?: number; lastTransitionTime?: string }> };
  Events: { items: Array<{ type: string; reason: string; message: string; count?: number; time?: string }> };
  Table: { columns: Array<{ key: string; label: string }>; rows: NativeValue[][] };
  Timeline: { items: Array<{ time: string; title: string; detail?: string; tone?: NativeTone }> };
  Markdown: { text: string };
  Code: { text: string; language: "yaml" | "none" };
}
export type NativeComponentType = keyof NativeComponentData;
export type NativeComponentPayload = {
  [K in NativeComponentType]: { version: 1; type: K; data: NativeComponentData[K] }
}[NativeComponentType];

const validate = new Ajv({ strict: true, allowUnionTypes: true, allErrors: false, ownProperties: true }).compile(schema);

/** Bounded JSON only. Reject getters/prototypes/cycles before schema traversal. */
function boundedJson(value: unknown): boolean {
  const stack = [{ value, depth: 0 }];
  const seen = new Set<object>();
  let nodes = 0;
  let characters = 0;
  while (stack.length) {
    const { value, depth } = stack.pop()!;
    if (++nodes > 25000 || depth > 8) return false;
    if (typeof value === "string") characters += value.length;
    else if (typeof value === "number") { if (!Number.isFinite(value)) return false; }
    else if (value !== null && typeof value === "object") {
      if (Array.isArray(value) && value.length > 1000) return false;
      if (seen.has(value)) return false;
      seen.add(value);
      const prototype = Object.getPrototypeOf(value);
      if (prototype !== Object.prototype && prototype !== Array.prototype && prototype !== null) return false;
      if (Object.getOwnPropertySymbols(value).length) return false;
      const descriptors = Object.getOwnPropertyDescriptors(value);
      const entries = Object.entries(descriptors);
      if (entries.length > 1001) return false;
      for (const [key, descriptor] of entries) {
        if (Array.isArray(value) && key === "length") continue;
        if (!descriptor.enumerable || !("value" in descriptor)) return false;
        characters += key.length;
        stack.push({ value: descriptor.value, depth: depth + 1 });
      }
    } else if (value !== null && typeof value !== "boolean") return false;
    if (characters > 524288) return false;
  }
  return true;
}

export type NativeComponentValidation =
  | { ok: true; value: NativeComponentPayload }
  | { ok: false; error: string };

/** Validate every untrusted payload at the rendering boundary, without coercion or truncation. */
export function validateNativeComponent(value: unknown): NativeComponentValidation {
  try {
    if (!boundedJson(value)) return { ok: false, error: "Component data exceeds the supported limits or is not JSON." };
    if (!validate(value)) return { ok: false, error: "Unsupported component version or invalid component data." };
    const component = value as NativeComponentPayload;
    if (component.type === "Table") {
      const { columns, rows } = component.data;
      if (new Set(columns.map(column => column.key)).size !== columns.length || rows.some(row => row.length !== columns.length)) {
        return { ok: false, error: "Table columns must have unique keys and each row must match the columns." };
      }
    }
    return { ok: true, value: component };
  } catch {
    return { ok: false, error: "Component data could not be validated." };
  }
}
