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
 * switch, the six capabilities that are always confirm-gated, and a plain
 * sentence about what free reading means — worded to agree with, not quote,
 * the same fact `/connect`'s footer tells its reader one page over.
 */

/** §23's order, verbatim — the mutating capabilities that always confirm. */
const GATED_CAPABILITIES = [
  "node.drain",
  "pod.evict",
  "resource.delete",
  "workload.scale",
  "rollout.undo",
  "helm.uninstall",
] as const;

export function AgentAccess() {
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
      <div className="mt-2 flex flex-wrap gap-2">
        {GATED_CAPABILITIES.map((capability) => (
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
