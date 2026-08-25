import { useState, type ReactNode } from "react";
import { diagnoseContext, installPlugin, isTauri, plural, startToolInstall } from "@srelens/core";
import type { RequirementResult, RequirementStatus } from "@srelens/core/lib/toolbox";
import {
  Badge,
  Button,
  EmptyState,
  LoadingState,
  Section,
  StatusPill,
  type StatusKind,
} from "@srelens/ui-kit";
import { FailureAlert, FailureLine, FailureState } from "../../lib/errorCopy";
import { useResource } from "../../lib/useResource";

/**
 * §17's rail width, and this screen's alone — see `SideRail`'s note on why the
 * width is a number per screen rather than a scale. Exported so the screen and
 * this file cannot hold two different answers to the same question.
 */
export const EXEC_AUTH_RAIL_WIDTH = 288;

/**
 * What the Toolbox can actually tell apart about a tool. **Four values.**
 *
 * THE ONE HAND-PAIRED WORD/TONE TABLE IN THIS SCREEN, AND IT IS MARKED BECAUSE
 * IT SHOULD NOT HAVE TO EXIST. It was written by Task 8 in `Toolbox.tsx` and
 * MOVED here rather than copied: this rail needs a word for a resolution too,
 * and a second table beside the first is exactly the drift — an amber word and
 * a grey one for the same fact — that the "every status word comes from core"
 * rule exists to stop. It lives in this file because `Toolbox` imports this
 * rail to mount it, so this is the direction the import can run without a
 * cycle. **If a `toolVerdict` is ever added to `packages/core`, delete this and
 * call it** — and do not spell `status === "missing" ? … : …` anywhere else.
 *
 * Task 8's three states are unchanged. `off-path` is the fourth, and it is a
 * genuinely different fact from both of the ones already here: `unmanaged`
 * means somebody else installed it and it WORKS, `missing` means it is not on
 * the machine, and this means it is on the machine and srelens still cannot
 * run it. Collapsing it into either one loses the only thing the reader needs
 * to know — which of two completely different remedies applies.
 *
 * The severities are §17's own rule (`installed` → ok, everything else muted).
 * A missing exec-auth tool IS the reason a cluster cannot be reached, which
 * argues for `warning` — but the severity of a state is not a property of the
 * pane it is drawn in, and the same word must not be amber in the rail and
 * grey in the table six inches to its left. The urgency is carried where it
 * belongs: in the section's own sentence, which says the cluster cannot be
 * reached, and in the count beside its heading.
 */
export type ToolState = "installed" | "unmanaged" | "off-path" | "missing";

export const TOOL_VERDICT: Record<ToolState, { word: string; kind: StatusKind }> = {
  installed: { word: "Installed", kind: "success" },
  unmanaged: { word: "Unmanaged", kind: "neutral" },
  "off-path": { word: "Not on PATH", kind: "neutral" },
  missing: { word: "Missing", kind: "neutral" },
};

/**
 * The backend's three resolutions, named in the vocabulary above.
 *
 * Not a second word/tone table: it pairs one core vocabulary with another and
 * carries neither a word nor a colour of its own. `found` is `installed`
 * because that is what a binary the app can execute is — the pane head's
 * `~/.srelens/bin` promise is the table's business, not this rail's.
 */
const RESOLVED: Record<RequirementStatus, ToolState> = {
  found: "installed",
  "not-on-app-path": "off-path",
  missing: "missing",
};

/** One context's answer — or the reason it has none. */
interface ContextCheck {
  context: string;
  items: RequirementResult[];
  /** Set when `toolbox.diagnoseContext` refused for this context alone. */
  error?: string;
}

/** A requirement srelens cannot satisfy right now. */
function unresolved(item: RequirementResult): boolean {
  return item.status !== "found";
}

/** Whether a context has anything to say beyond "everything resolves". */
function reportable(check: ContextCheck): boolean {
  return check.error !== undefined || check.items.some(unresolved);
}

/**
 * The install srelens can actually run for a requirement, or null.
 *
 * **`installable` alone is not the answer, and that is the whole point of this
 * function.** The backend sets it for kubectl and for krew plugins — a fact
 * about the TOOL — while whether installing would help is a fact about the
 * RESOLUTION. A binary sitting at `/opt/homebrew/bin/kubectl` that the app
 * cannot see is `installable: true`, and installing it would put a second copy
 * under `~/.srelens/bin` and leave this one exactly where it is. That is a
 * button that appears to work and changes nothing, which is worse than no
 * button: the reader clicks it, watches it succeed, and still cannot reach the
 * cluster.
 */
function installAction(item: RequirementResult): (() => Promise<{ error?: string }>) | null {
  if (item.status !== "missing" || !item.installable) return null;
  if (item.kind === "kubectl") return () => startToolInstall("kubectl", () => {});
  const plugin = item.plugin;
  // A krew requirement with no plugin name is a shape the backend does not
  // produce; a click on it would reject on an empty argument, so it is not
  // drawn. A control that cannot work is not drawn.
  if (item.kind === "krew-plugin" && plugin) return () => installPlugin(plugin);
  return null;
}

/** What a requirement says about itself, in the resolution's own terms. */
function sentence(item: RequirementResult): string {
  if (item.status === "not-on-app-path") {
    return (
      "Present on this machine, but not on the PATH srelens runs commands with. " +
      "Installing it again would not change that — put this copy on that PATH, " +
      "or link it into ~/.srelens/bin."
    );
  }
  return "srelens did not find it on the PATH it runs commands with, nor anywhere else it looked.";
}

export interface ExecAuthRailProps {
  /**
   * Every context this window knows about, by the name `toolbox.diagnoseContext`
   * takes. One call each: the capability reads a context's own exec block out
   * of the kubeconfig that owns it, and there is no batched form.
   */
  contexts: readonly string[];
}

/**
 * §17's right rail: which kubeconfig contexts cannot authenticate, and why.
 *
 * A context can name an external binary in its user's `exec` block —
 * `kubectl-oidc_login`, `aws`, `gke-gcloud-auth-plugin` — and when srelens
 * cannot run that binary the cluster is simply unreachable, with nothing on
 * screen saying so. Every other symptom (an empty resource list, a rail of red
 * clusters) is a consequence. This is the one surface that names the cause.
 *
 * **IT REPORTS RESOLUTION, NOT VERSION ADEQUACY.** §17's copy for this rail
 * reads "needs kubelogin v1.32.0. The installed v1.31.0 cannot refresh its
 * token, so watches will drop after an hour", and not one clause of that can
 * be backed. A kubeconfig `exec` block records a command and its arguments;
 * `Requirement` is `{ binary, kind }` accordingly. Kubernetes records WHICH
 * binary a context runs and never which version, nothing in this app queries a
 * release feed, and no token lifetime is reported anywhere. The rail says the
 * true thing instead: this context wants a tool that is missing, or one that
 * is present but not where srelens looks. The suite asserts no version string
 * appears here at all, with a resolved requirement's `v1.31.4` in the fixture
 * so the assertion has something to catch.
 *
 * **A context with nothing to report is healthy, not empty.**
 * `ContextRequirements` says it in as many words — a context whose user has no
 * exec block needs nothing external, which is a healthy state and not an
 * absence — and the same is true of one whose every requirement resolved. Both
 * fold into one sentence for the whole rail rather than a blank card each.
 * Sections are for contexts that have something to say.
 */
export function ExecAuthRail({ contexts }: ExecAuthRailProps) {
  const checks = useResource<ContextCheck[]>(
    async () =>
      Promise.all(
        contexts.map(async (context): Promise<ContextCheck> => {
          const result = await diagnoseContext(context);
          // Kept per context rather than thrown. `diagnoseContext` refuses one
          // context at a time — an unknown name, a kubeconfig that no longer
          // parses — and one unreadable context must not take the other nine
          // contexts' answers off the rail with it.
          return { context, items: result.data?.items ?? [], error: result.error };
        }),
      ),
    // The list, not its identity: `useContexts` hands back a new array on every
    // store notification, and depending on the reference would re-diagnose
    // every context each time anything in the window changed.
    [JSON.stringify(contexts)],
  );

  if (checks.status === "loading") {
    return <LoadingState label="Checking exec auth" className="py-6" />;
  }
  if (checks.status === "error") {
    // Nothing in `diagnoseContext` rejects today — it reports a refusal as
    // `{ error }` — so this is the guard against that changing underneath us.
    // The alternative is worse than an error card: with no data, the healthy
    // sentence below is what would render, and the rail would certify auth it
    // never managed to check.
    return (
      <FailureState
        title="Could not check exec auth"
        error={checks.error}
        onRetry={checks.reload}
        className="py-6"
      />
    );
  }

  const rows = (checks.data ?? []).filter(reportable);

  if (rows.length === 0) {
    return contexts.length === 0 ? (
      <EmptyState
        compact
        title="No contexts to check"
        hint="srelens has no kubeconfig context loaded, so there is no exec auth to resolve."
      />
    ) : (
      <EmptyState
        compact
        title="Every context's auth resolves"
        // Counted by core's `plural`, and said out loud: the reader needs to
        // know a check happened, not merely that nothing is on screen.
        hint={`${plural(contexts.length, "context")} checked. None of them runs a tool srelens cannot.`}
      />
    );
  }

  // A context srelens could not check at all is a different fact from one whose
  // auth needs a tool, and it repeats: the same refusal arrives once per
  // context. Drawn one card each, eleven of them push the finding this rail
  // exists for off the bottom of a 288px column — the shape that made the
  // cluster overview unreadable when a raw API error took four rows per
  // cluster. They collapse into one section; the actionable ones keep theirs.
  const unchecked = rows.filter((check) => check.error !== undefined);
  const actionable = rows.filter((check) => check.error === undefined);

  return (
    <>
      {actionable.map((check) => (
        <ContextSection key={check.context} check={check} onInstalled={checks.reload} />
      ))}
      {unchecked.length > 0 && <UncheckedSection checks={unchecked} />}
    </>
  );
}

/**
 * The contexts srelens could not check, together.
 *
 * One card names its context and says why; several say the same sentence over
 * and over, and the reason belongs to all of them. The first error is shown
 * because a repeated failure has a repeated cause — and the names are listed
 * so a reader can tell whether the contexts they care about are among them.
 */
function UncheckedSection({ checks }: { checks: ContextCheck[] }) {
  return (
    <Section title={`${plural(checks.length, "context")} not checked`}>
      <div className="flex flex-col gap-1 text-[0.8125rem]">
        <div>srelens could not read the exec auth for {checks.length === 1 ? "this context" : "these contexts"}.</div>
        <div className="text-muted">{checks.map((c) => c.context).join(", ")}</div>
        <FailureLine error={checks[0].error ?? ""} className="text-muted" />
      </div>
    </Section>
  );
}

/** One context that cannot authenticate, and what would fix it. */
function ContextSection({
  check,
  onInstalled,
}: {
  check: ContextCheck;
  onInstalled: () => void;
}) {
  const items = check.items.filter(unresolved);

  return (
    <Section title={check.context}>
      {check.error !== undefined ? (
        <div className="flex flex-col gap-1 text-[0.8125rem]">
          <div>srelens could not check this context.</div>
          <FailureLine error={check.error} className="text-muted" />
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <div>
            {/* §17's action badge. The word is core's `plural`, the tone is the
                Badge's own default — a count is a figure, not a verdict, and
                colouring one here would be the second hand-paired table this
                file exists to avoid. */}
            <Badge>{plural(items.length, "issue")}</Badge>
          </div>
          <p className="text-[0.8125rem] leading-snug text-muted">
            This context authenticates by running an external tool. srelens cannot reach the
            cluster until each tool below resolves.
          </p>
          {items.map((item) => (
            <Requirement
              key={item.binary}
              item={item}
              onInstalled={onInstalled}
            />
          ))}
        </div>
      )}
    </Section>
  );
}

/** One binary a context wants, and the remedy its resolution actually has. */
function Requirement({ item, onInstalled }: { item: RequirementResult; onInstalled: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const verdict = TOOL_VERDICT[RESOLVED[item.status]];
  const run = installAction(item);

  async function install() {
    if (!run) return;
    setBusy(true);
    setError(null);
    const result = await run();
    setBusy(false);
    if (result.error) setError(result.error);
    // Re-check either way: a failed install can still have moved a binary, and
    // the resolution is the only honest report of where it ended up.
    else onInstalled();
  }

  let remedy: ReactNode = null;
  if (run === null && item.status === "missing" && !item.installable) {
    remedy = (
      <p className="text-[0.8125rem] leading-snug text-muted">
        srelens does not install this one — install it yourself and put it on the PATH.
      </p>
    );
  } else if (run !== null && !isTauri()) {
    // Said rather than drawn disabled: `toolbox.installKubectl` and
    // `toolbox.installPlugin` are both in the server's
    // `WEB_DENIED_CAPABILITIES`, so the button would reject on click.
    remedy = (
      <p className="text-[0.8125rem] leading-snug text-muted">
        Installing happens in the srelens desktop app.
      </p>
    );
  } else if (run !== null) {
    remedy = (
      <Button
        className="w-full"
        size="sm"
        // Three sections can each offer an "Install"; the accessible name says
        // which binary this one is for.
        aria-label={`Install ${item.binary}`}
        disabled={busy}
        onClick={() => void install()}
      >
        {busy ? "Installing…" : `Install ${item.binary}`}
      </Button>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2">
        <span className="truncate font-medium">{item.binary}</span>
        <StatusPill status={verdict.word} kind={verdict.kind} />
      </div>
      <p className="text-[0.8125rem] leading-snug text-muted">{sentence(item)}</p>
      {/* On screen, in the open. A resolution's path is a value, and the rule
          `PairList` and `KV` were stripped for is that a value never hides in
          a `title` attribute nothing announces. */}
      {item.path ? (
        <div className="break-all font-mono text-[0.6875rem] text-faint">{item.path}</div>
      ) : null}
      {remedy}
      {error !== null && (
        <FailureAlert tone="sev" title={`Could not install ${item.binary}`} error={error} />
      )}
    </div>
  );
}
