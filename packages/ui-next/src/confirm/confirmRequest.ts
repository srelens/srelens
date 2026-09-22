import {
  CAPABILITY_IMPACT_ORDER,
  type CapabilityImpact,
  type ConfirmRequest,
  type ConfirmTarget,
  type ConfirmTemplateField,
} from "@srelens/core";
import { boundedPlainText, type ConfirmationSubject } from "./HostConfirmation";

/** What one call fills in of the host's closed template vocabulary. */
export type ConfirmFieldValues = Partial<Record<ConfirmTemplateField, string>>;

/** The four facts a caller of {@link confirmFields} actually holds. */
export interface ConfirmCall {
  action?: string;
  cluster?: string | null;
  kind?: string | null;
  namespace?: string | null;
  name?: string | null;
}

/**
 * The values a host confirmation template may interpolate, for one call.
 *
 * The TypeScript twin of `srelens_capability::annotations::confirm_fields`,
 * and it has to stay one: the UI path renders the host's template here while
 * the MCP path renders the same template in Rust, and two readings of "what is
 * this call about" would be two sentences for one write.
 *
 * Three rules carried over, each for the reason the Rust side gives:
 *
 * - **`{resource}` is derived, never read.** It is `kind namespace/name`
 *   collapsed to whatever is known, so a caller cannot hand the host a
 *   pre-formatted description of what it is about to change.
 * - **A value the call does not have is ABSENT**, not empty, so an optional
 *   `[ … ]` segment naming it drops whole rather than rendering a gap.
 * - **Every value is escaped and then bounded** ({@link boundedPlainText}),
 *   in that order: escaping after bounding would let eighty characters draw
 *   as hundreds.
 */
export function confirmFields(call: ConfirmCall): ConfirmFieldValues {
  const out: ConfirmFieldValues = {};
  const take = (field: ConfirmTemplateField, value: string | null | undefined) => {
    if (value) out[field] = boundedPlainText(value);
  };
  take("action", call.action);
  take("cluster", call.cluster);
  take("kind", call.kind);
  take("namespace", call.namespace);
  take("name", call.name);
  if (out.name) {
    const object = out.namespace ? `${out.namespace}/${out.name}` : out.name;
    // Composed from parts already escaped and bounded, so this only re-applies
    // the bound: three fields at the ceiling would still be unreadable.
    out.resource = boundedPlainText(out.kind ? `${out.kind} ${object}` : object);
  }
  return out;
}

/**
 * The facts under the question, as they arrive from the backend
 * (`ConfirmTarget`, `apps/desktop/src-tauri/src/mcp_confirm.rs`) — narrowed,
 * never cast.
 *
 * This payload crosses a process boundary. A field of the wrong type is
 * dropped rather than drawn: a `Cluster: undefined` under an Approve button is
 * a worse prompt than one that names no cluster, and passing a number on to
 * the text helpers throws where a confirmation should have rendered. It is
 * exactly the hole a malformed or hostile payload would aim for.
 *
 * **An app identity in the payload is not read.** Nothing authenticates an
 * MCP caller as the app its arguments name, so an attribution taken from here
 * would be provenance chosen by the party being vouched for. The backend no
 * longer sends one; this drops it as well, so a hand-built payload cannot
 * reintroduce it.
 */
export function asConfirmTarget(payload: unknown): ConfirmTarget | null {
  if (typeof payload !== "object" || payload === null) return null;
  const raw = payload as Record<string, unknown>;
  const text = (key: string) => {
    const value = raw[key];
    return typeof value === "string" && value !== "" ? value : null;
  };
  return {
    cluster: text("cluster"),
    namespace: text("namespace"),
    name: text("name"),
    kind: text("kind"),
  };
}

const IMPACTS: readonly string[] = CAPABILITY_IMPACT_ORDER;

/**
 * A `mcp://confirm-request` payload, narrowed rather than cast.
 *
 * **Every listener uses this one.** The event crosses a process boundary, so
 * its payload is `unknown` whatever a listener's type parameter says: classic's
 * `McpConfirmDialog` and the assistant transcript both wrote
 * `listen<ConfirmRequest>` and pushed `event.payload` into state unchecked,
 * which is an annotation and not a check. A `target.cluster` arriving as a
 * number then reached {@link boundedPlainText} and threw `.replace is not a
 * function` — a blank screen in place of the confirmation the backend is
 * blocking on, and a call denied sixty seconds later with nothing drawn.
 *
 * A payload with no usable `id` is dropped whole: there is nothing to answer
 * with, and a card headed `undefined` over a call that will time out anyway
 * tells the reader nothing they can act on. Everything else is narrowed field
 * by field, and a field that does not narrow is simply absent — the question
 * and the facts are what a person approves, and a hole in either is what a
 * malformed, or hostile, payload would aim for.
 */
export function asConfirmRequest(payload: unknown): ConfirmRequest | null {
  if (typeof payload !== "object" || payload === null) return null;
  const { id, tool, args, prompt, impact, target } = payload as Partial<ConfirmRequest>;
  if (typeof id !== "string" || id === "" || typeof tool !== "string") return null;
  return {
    id,
    tool,
    args: typeof args === "object" && args !== null ? args : {},
    target: asConfirmTarget(target),
    prompt: typeof prompt === "string" && prompt !== "" ? prompt : null,
    impact: IMPACTS.includes(impact as CapabilityImpact) ? (impact as CapabilityImpact) : undefined,
  };
}

/**
 * What the one confirmation names as the thing about to change.
 *
 * `null` when the call named no object — a capability that installs a tool or
 * lists a namespace has no resource line, and inventing one would be a claim
 * about the call that nothing backs. `bulk` is not produced here: a bulk
 * selection (#553) is the caller's own count, handed to the component
 * directly rather than read out of one request's target.
 */
export function confirmSubject(target: ConfirmTarget | null | undefined): ConfirmationSubject | null {
  if (!target?.name) return null;
  return { kind: "object", namespace: target.namespace ?? null, name: target.name };
}
