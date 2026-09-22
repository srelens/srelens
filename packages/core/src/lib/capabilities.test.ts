import { describe, expect, it } from "vitest";
import catalog from "./capability-catalog.json";
import {
  CAPABILITY_CATALOG,
  CAPABILITY_IMPACT_ORDER,
  CONFIRM_TEMPLATE_FIELDS,
  HOST_ONLY_CAPABILITY_IDS,
  gatedCapabilityIds,
  renderConfirmTemplate,
} from "./capabilities";

/**
 * The generated catalog is the source of truth here — held equal to the live
 * Rust registry by `capability_catalog_json_is_in_sync`. So these tests compare
 * the derivation against the FILE rather than against a list retyped in a test
 * body: a hardcoded expectation would be a third copy, and the reason this
 * module exists at all is that the new design's `Agent access` pane shipped a
 * hardcoded array of six ids the backend does not register, pinned by a test
 * that compared one hardcoded array to another.
 */
type Entry = { id: string; readOnly: boolean; destructive: boolean; requiresConfirm: boolean };
const entries = catalog as Entry[];

describe("gatedCapabilityIds", () => {
  it("is every confirm-gated capability the backend registers, and nothing else", () => {
    const want = entries.filter((c) => c.requiresConfirm).map((c) => c.id);
    expect(gatedCapabilityIds("desktop")).toEqual(want);
    expect(want.length).toBeGreaterThan(0);
  });

  it("names ids that exist, in the form the audit trail records", () => {
    const known = new Set(entries.map((c) => c.id));
    for (const id of gatedCapabilityIds("desktop")) expect(known.has(id)).toBe(true);
    // The invented set this replaced was `node.drain`, `pod.evict`,
    // `resource.delete`, `workload.scale`, `rollout.undo`, `helm.uninstall`.
    // Not one is registered, and the audit pane one panel below renders the
    // real ids — so both appeared on the same screen.
    for (const invented of ["node.drain", "pod.evict", "resource.delete", "workload.scale", "rollout.undo", "helm.uninstall"]) {
      expect(known.has(invented)).toBe(false);
    }
  });

  /**
   * The gate is `requiresConfirm`, not `destructive`. A pane that filtered on
   * `destructive` would drop `k8s.scale`, `k8s.applyManifest` and every toolbox
   * install — all gated, none flagged destructive.
   */
  it("includes gated capabilities that are not destructive", () => {
    const ids = gatedCapabilityIds("desktop");
    const gatedNonDestructive = entries.filter((c) => c.requiresConfirm && !c.destructive);
    expect(gatedNonDestructive.length).toBeGreaterThan(0);
    for (const c of gatedNonDestructive) expect(ids).toContain(c.id);
  });

  /** `k8s.getSecret` is read-only and gated — the case no other flag implies. */
  it("includes the read that returns secret material", () => {
    const secret = entries.find((c) => c.id === "k8s.getSecret");
    expect(secret).toBeTruthy();
    expect(secret?.readOnly).toBe(true);
    expect(gatedCapabilityIds("desktop")).toContain("k8s.getSecret");
  });

  it("leaves out the capabilities a web registry does not register", () => {
    const web = gatedCapabilityIds("web");
    const desktop = gatedCapabilityIds("desktop");
    expect(web).not.toContain("settings.set");
    expect(web).not.toContain("extensions.action");
    expect(HOST_ONLY_CAPABILITY_IDS).toContain("extensions.resource");
    expect(desktop).toContain("extensions.action");
    expect(desktop).toContain("settings.set");
    expect(desktop.filter((id) => !HOST_ONLY_CAPABILITY_IDS.includes(id))).toEqual(web);
  });

  it("exposes the catalog it derives from, unchanged", () => {
    expect(CAPABILITY_CATALOG).toEqual(entries);
  });
});

/**
 * The metadata #548 added. Again against the FILE rather than a retyped list:
 * the point of the generated catalog is that nothing here is a second copy.
 */
describe("capability metadata v2", () => {
  const rows = catalog as Array<
    Entry & { sensitive: boolean; impact: string; confirm: string | null }
  >;

  it("gives every capability a level, and only the three the host defines", () => {
    expect(rows.length).toBeGreaterThan(0);
    for (const c of rows) expect(CAPABILITY_IMPACT_ORDER).toContain(c.impact);
  });

  /**
   * The level and the gate must not disagree, or a pane showing both says two
   * things at once. Mirrors `assert_impact_matches_the_gate`
   * (`crates/mcp/src/completeness.rs`), which fails the Rust build first.
   */
  it("keeps the level and the gate in agreement", () => {
    for (const c of rows) {
      if (c.destructive) expect(c.impact).toBe("high");
      if (c.requiresConfirm) expect(c.impact).not.toBe("low");
      if (c.readOnly && !c.requiresConfirm) expect(c.impact).toBe("low");
    }
  });

  /**
   * The case the issue names: one capability covers an Argo CD status refresh
   * and an Argo CD sync, so its published level is the ceiling — the per-action
   * level travels with the resource instead.
   */
  it("publishes the ceiling for the capability that carries several actions", () => {
    const action = rows.find((c) => c.id === "k8s.gitOpsAction");
    expect(action).toBeUndefined();
    expect(rows.find((c) => c.id === "extensions.action")?.impact).toBe("high");
  });

  /**
   * `sensitive` is a redaction flag, not a gate — but "sensitive and ungated"
   * is a claim the host has to earn, so the exceptions are named here rather
   * than merely permitted. A new one has to be added to this list on purpose.
   *
   * `k8s.diffManifest` is the only member. It earns it by redaction rather
   * than consent: `redact_secret_data` (`crates/kube/src/secrets.rs`) blanks a
   * Secret's `data`, `stringData` AND every `metadata.annotations` value on
   * both sides of the diff, so no cluster-side secret material reaches the
   * response. `sensitive` stays true because the *request* carries the
   * caller's own manifest, which the audit log must redact.
   */
  it("names every capability that is sensitive but not gated", () => {
    const sensitiveUngated = rows.filter((c) => c.sensitive && !c.requiresConfirm).map((c) => c.id);
    expect(sensitiveUngated).toEqual(["k8s.diffManifest"]);
    // And it is a read at the lowest level, which is the only way an ungated
    // capability can be spelled (see the coherence test above).
    const diff = rows.find((c) => c.id === "k8s.diffManifest");
    expect(diff?.readOnly).toBe(true);
    expect(diff?.impact).toBe("low");
  });

  it("gives every gated capability the host's own confirmation wording", () => {
    const gated = rows.filter((c) => c.requiresConfirm);
    expect(gated.length).toBeGreaterThan(0);
    for (const c of gated) expect(c.confirm).toBeTruthy();
    // And an ordinary read asks nothing.
    for (const c of rows.filter((c) => c.readOnly && !c.requiresConfirm)) {
      expect(c.confirm).toBeNull();
    }
  });

  /**
   * Every committed template must render with NO fields resolved — the check
   * `assert_confirm_templates_are_renderable` makes in Rust, repeated here
   * against this renderer, because a template that only the backend can render
   * is a template the host confirmation cannot use.
   */
  it("renders every committed template, with fields and without", () => {
    // Guarded, because the loop below says nothing about a catalog that
    // carries no templates at all — which is exactly the catalog this change
    // replaced, and exactly the state a botched regeneration would restore.
    expect(rows.filter((c) => c.confirm).length).toBeGreaterThan(0);
    for (const c of rows) {
      if (!c.confirm) continue;
      expect(renderConfirmTemplate(c.confirm, {})).toBeTruthy();
      expect(
        renderConfirmTemplate(c.confirm, {
          action: "sync",
          cluster: "prod",
          kind: "Application",
          name: "api",
          namespace: "team",
          resource: "Application team/api",
        }),
      ).toBeTruthy();
    }
  });

  it("names only fields the host's vocabulary contains", () => {
    const named = rows.flatMap((c) => [...(c.confirm ?? "").matchAll(/\{([a-z]+)\}/g)]);
    // Same guard, same reason: "no template names a field outside the
    // vocabulary" is true of a catalog with no templates in it.
    expect(named.length).toBeGreaterThan(0);
    for (const [, field] of named) expect(CONFIRM_TEMPLATE_FIELDS).toContain(field);
  });
});

describe("renderConfirmTemplate", () => {
  it("substitutes named fields", () => {
    expect(renderConfirmTemplate("Suspend {resource} in cluster {cluster}?", { resource: "r", cluster: "c" }))
      .toBe("Suspend r in cluster c?");
  });

  it("drops an optional segment whose field is missing, and keeps one whose field is not", () => {
    const template = "Suspend {resource}[ in cluster {cluster}]?";
    expect(renderConfirmTemplate(template, { resource: "r", cluster: "c" })).toBe("Suspend r in cluster c?");
    expect(renderConfirmTemplate(template, { resource: "r" })).toBe("Suspend r?");
  });

  /**
   * The alternative is "Suspend ?" in a dialog that authorizes a write. A
   * caller that gets null shows the capability summary instead.
   */
  it("refuses to render when a field outside a segment is missing", () => {
    expect(renderConfirmTemplate("Suspend {resource}?", { cluster: "c" })).toBeNull();
    expect(renderConfirmTemplate("Suspend {resource}?", { resource: "" })).toBeNull();
  });

  it("renders a template with no placeholders as itself", () => {
    expect(renderConfirmTemplate("Allow this change?", {})).toBe("Allow this change?");
  });

  /**
   * The backend renders the same strings; a divergence here would show one
   * sentence in the MCP denial and a different one in the dialog.
   */
  it("agrees with the backend on a committed template", () => {
    const mutating = "Allow this change[ to {resource}][ in cluster {cluster}]?";
    expect(renderConfirmTemplate(mutating, { resource: "api", cluster: "prod" }))
      .toBe("Allow this change to api in cluster prod?");
    expect(renderConfirmTemplate(mutating, {})).toBe("Allow this change?");
  });
});
