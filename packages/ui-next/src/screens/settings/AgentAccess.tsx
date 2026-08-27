import { useMemo } from "react";
import { gatedCapabilityIds, isTauri } from "@srelens/core";
import { Panel, SubHead, Switch } from "@srelens/ui-kit";

/**
 * §23 draws three switches for what an MCP-connected agent may do. An
 * investigation for this task established that only the first is a real,
 * persisted, runtime-togglable setting; the other two are not settings at
 * all, and this pane does not draw them — not even disabled.
 *
 * **`Read cluster state`** is true: `assert_mutating_capabilities_are_gated`
 * (`crates/mcp/src/completeness.rs:36-45`) is a build-time invariant that
 * panics if a registered mutating capability lacks `requires_confirm`. A
 * read-only capability carries no such requirement, so nothing gates it —
 * there is no flag or preference that could turn it off, which is exactly
 * why the switch is drawn on and disabled rather than merely defaulted on.
 *
 * **`Propose changes`** does not exist as a setting. Every mutating
 * capability is confirm-gated by the invariant above, unconditionally —
 * there is no preference anywhere that widens or narrows that gate, so a
 * switch for it would imply a control that has nothing to control.
 *
 * **`Read Secrets`** is half real: `ConsentKind::SensitiveRead`
 * (`crates/mcp/src/policy.rs:21-31`) is a genuine gate, but the only thing on
 * either side of it is a headless launch flag, `--mcp-allow-sensitive-reads`
 * (`ConsentKind::flag`, same file). A switch implies a persisted preference
 * a reader can flip here and have it stick; a flag is read once at process
 * start and lives outside this screen entirely. Drawing a disabled switch
 * for it would tell the reader "you could change this somewhere else", and
 * there is no somewhere else — see issue #368.
 *
 * A disabled switch for either of the excluded two would read as "not yet
 * wired up" rather than "does not exist", which is the wrong claim to make
 * about a control this screen can never grow. So this pane draws one real
 * switch, the confirm-gated capabilities, and a plain sentence about what free
 * reading means — worded to agree with, not quote, the same fact `/connect`'s
 * footer tells its reader one page over.
 *
 * **The chips are read from the registry, not transcribed from §23.** This
 * pane shipped §23's six labels verbatim — `node.drain`, `pod.evict`,
 * `resource.delete`, `workload.scale`, `rollout.undo`, `helm.uninstall` — and
 * NOT ONE is a capability srelens registers. The real ids are
 * `k8s.drainNode`, `k8s.evictPod`, `k8s.deleteResource`, `k8s.scale`. Six was
 * not the set either: {@link gatedCapabilityIds} finds twenty-eight. Two
 * separate falsehoods came of that. Under a heading claiming completeness, six
 * chips told a reader that `k8s.applyManifest` is ungated, and it is not; and
 * `AuditPane`, one panel below on this same screen, renders the ids the
 * backend actually recorded — so `resource.delete` and `k8s.deleteResource`
 * appeared six inches apart.
 *
 * The list now comes from `packages/core/src/lib/capabilities.ts`, over the
 * generated catalog that `capability_catalog_json_is_in_sync`
 * (`crates/registry/src/lib.rs`) holds equal to the live Rust registry. A
 * capability added, renamed or re-annotated cannot ship without this pane
 * following it.
 *
 * **All of them, not a sample.** Twenty-eight `code` chips is a wall, and it
 * was tempting to show a handful and say "and others". A handful is what was
 * wrong here: the heading promises that nothing outside this set runs
 * unconfirmed, so a reader must be able to look an id up and find it. The
 * count is stated beside them, from the rendered array's own length, so the
 * sentence cannot drift from the chips.
 */

export function AgentAccess() {
  // `isTauri()` rather than the catalog wholesale: a web build does not
  // register the host settings capabilities
  // (`web_registry_omits_host_desktop_settings`, `crates/registry/src/lib.rs`),
  // and naming an id the running build has no entry for would be this pane
  // inventing one again — one instead of six.
  const gated = useMemo(() => gatedCapabilityIds(isTauri() ? "desktop" : "web"), []);

  return (
    <Panel
      title="Agent access"
      description="What a connected agent may do without asking, and what it can never do without confirming first."
    >
      <Switch
        on
        disabled
        label="Read cluster state"
        hint="List and get any non-secret object, tail logs, read events and metrics. Always on."
      />
      <p className="mt-3 text-[0.75rem] leading-relaxed text-muted">
        A connected agent reads cluster state without asking first; every change it
        proposes stops at a confirmation prompt.
      </p>
      <SubHead className="mt-4" variant="caps">
        Never without confirmation
      </SubHead>
      {/* The count comes from the array that draws the chips, so the two
          cannot disagree. Both kinds are named, because the set is not just
          "the destructive ones": `k8s.getSecret` changes nothing and is gated,
          and `k8s.scale` changes something without being destructive. */}
      <p data-testid="gated-count" className="mt-1 text-[0.75rem] leading-relaxed text-muted">
        All {gated.length} of them — every capability that changes anything, and the one that returns
        Secret values. The list is srelens&apos;s own capability registry, not a selection: nothing
        here can be switched off, and nothing outside it needs a confirmation.
      </p>
      <div className="mt-2 flex flex-wrap gap-2">
        {gated.map((capability) => (
          <code
            key={capability}
            data-testid="gated-capability"
            className="code inline-flex shrink-0 items-center rounded px-1.5 py-0.5 text-[0.6875rem]"
            style={{ border: "1px solid var(--sev)", color: "var(--sev)" }}
          >
            {capability}
          </code>
        ))}
      </div>
    </Panel>
  );
}
