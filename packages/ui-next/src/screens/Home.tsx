import { useEffect, useMemo, useRef, useState } from "react";
import { describeError, listContexts, plural, type ClusterContext } from "@srelens/core";
import { Badge, Button, EmptyState, LoadingState, Mark, NavIcon, Screen, StatusPill, TextInput, usePortalShowing } from "@srelens/ui-kit";
import { useAttention, type ClusterAttention } from "../lib/attention";
import { getContexts, getKubeconfigFiles, setContexts, useContexts, useContextsError, useContextsStatus } from "../lib/clusters";
import { useOrderedContexts } from "../lib/contextOrder";
import { FailureAlert } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { getMark, useEditableMark } from "../lib/marks";
import { symbolFor } from "../lib/markSymbols";
import { openCluster } from "../lib/openCluster";
import { openTab } from "../lib/tabsStore";
import { LINK_WORD, useWorkspaceView } from "../lib/workspace";
import { useTabs } from "../lib/tabsStore";
import { useWorkspaceSealed } from "../shell/LockGate";
import { Apps } from "./home/Apps";
import { LiveNow } from "./home/LiveNow";
import { NeedsAttention } from "./home/NeedsAttention";
import { PickUp } from "./home/PickUp";
import { WhatsNew } from "./home/WhatsNew";

/**
 * App-wide entry point: what needs attention across the workspace, the
 * clusters, and the way back into what the reader was doing. Contexts and
 * connection status come from the shell's shared stores.
 */
export function Home() {
  const contexts = useContexts();
  const status = useContextsStatus();
  const error = useContextsError();
  const ordered = useOrderedContexts(contexts);
  // Like the rail, subscribe once so filtering follows saved display-name edits.
  useEditableMark("", "");
  const { links } = useWorkspaceView();
  const { workspace } = useTabs();
  // Only what this workspace holds, is connected, and is not paused: a paused
  // cluster is one the reader asked srelens to stop reading.
  const targets = useMemo(
    () => ordered.filter((c) => workspace.clusters.includes(c.stableId)
      && links[c.stableId]?.state === "connected"
      && workspace.pausedClusters?.includes(c.stableId) !== true),
    [ordered, workspace, links],
  );
  // Hidden tabs stay mounted, so "not on screen" pauses the reads as well as the vault's cover.
  const sealed = useWorkspaceSealed();
  const showing = usePortalShowing();
  const scans = useAttention(targets, sealed || !showing);
  const [query, setQuery] = useState("");
  const [retrying, setRetrying] = useState(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const term = query.trim().toLocaleLowerCase();
  const filtered = ordered.filter(ctx =>
    [getMark(ctx.stableId, ctx.name).name, ctx.name, ctx.cluster, ctx.server].some(value => value.toLocaleLowerCase().includes(term)),
  );

  async function retry() {
    setRetrying(true);
    const previous = getContexts();
    try {
      const result = await listContexts(getKubeconfigFiles());
      if (mounted.current && getContexts() === previous) setContexts(result.contexts ?? previous, result.error ?? "");
    } catch (cause) {
      if (mounted.current && getContexts() === previous) setContexts(previous, describeError(cause).detail);
    } finally {
      if (mounted.current) setRetrying(false);
    }
  }

  return (
    <Screen title="Home" eyebrow="srelens" fill>
      <div className="home-page min-h-0 flex-1">
        <div className="home-intro">
          <div>
            <span className="home-eyebrow">Workspace</span>
            <h2>Clusters</h2>
            <p>Choose a cluster to open its overview.</p>
          </div>
          <Button variant="primary" onClick={() => openTab("/connect")}>
            <NavIcon icon={Icons.add} /> Connect a cluster
          </Button>
        </div>
        <div className="home-layout">
          <div className="home-main">
          <NeedsAttention targets={targets} scans={scans} />
          <section className="home-clusters" aria-labelledby="home-clusters-title">
            <div className="home-section-heading">
              <h2 id="home-clusters-title">Your clusters <span className="text-muted">{contexts.length}</span></h2>
              {contexts.length > 0 && <TextInput type="search" aria-label="Find a cluster" placeholder="Find a cluster…" value={query} onValueChange={setQuery} />}
            </div>
            {status === "failed" && <div className="border-b border-rule p-4">
              <FailureAlert title="Could not load all clusters" error={error} />
              <Button variant="secondary" size="sm" className="mt-2" disabled={retrying} onClick={() => void retry()}>{retrying ? "Retrying…" : "Retry"}</Button>
            </div>}
            {status === "loading" ? <LoadingState label="Loading clusters…" /> : contexts.length === 0 ? (
              status === "loaded" && <EmptyState title="No clusters configured" hint="Add a kubeconfig or connect to a cluster to start exploring. Your saved connections will appear here." />
            ) : filtered.length === 0 ? <EmptyState title="No matching clusters" hint="Search by display name, context, or API server." action={<Button variant="secondary" size="sm" onClick={() => setQuery("")}>Clear search</Button>} /> : (
              <ul className="home-cluster-list" aria-label="Saved clusters">
                {filtered.map(ctx => <ClusterRow key={ctx.stableId} context={ctx} link={links[ctx.stableId]} paused={workspace.pausedClusters?.includes(ctx.stableId) === true} scan={scans[ctx.stableId]} />)}
              </ul>
            )}
          </section>
          <PickUp targets={targets} />
          </div>
          <aside className="home-start" aria-label="Workspace tools">
            <LiveNow />
            <Apps />
            <WhatsNew />
            <h2 className="home-section-heading">Quick links</h2>
            <HomeAction title="Manage connections" icon={Icons.cluster} route="/connections" />
            <HomeAction title="Settings" icon={Icons.settings} route="/settings" />
            <HomeAction title="Release notes" icon={Icons.events} route="/notes" />
          </aside>
        </div>
      </div>
    </Screen>
  );
}

/**
 * One saved cluster. `scan` is this cluster's Needs-attention answer, present
 * only for a cluster that was read: its count is the strip's count for it, and
 * a check that refused says so here too rather than leaving the row looking
 * clean.
 */
function ClusterRow({ context, link, paused, scan }: { context: ClusterContext; link?: ReturnType<typeof useWorkspaceView>["links"][string]; paused: boolean; scan?: ClusterAttention }) {
  const mark = getMark(context.stableId, context.name);
  const status = paused ? "Paused" : link ? LINK_WORD[link.state] : "Not checked";
  const problems = scan?.items.length ?? 0;
  const finding = problems > 0 ? plural(problems, "problem") : scan && scan.failures.length > 0 ? "check failed" : null;
  let secondary = context.name;
  if (mark.name === context.name) {
    try { secondary = new URL(context.server).host; }
    catch { secondary = context.cluster !== context.name ? context.cluster : context.sourceFile; }
  }
  return <li className="border-b border-rule">
    <button type="button" className="home-cluster-row" aria-label={`Open cluster ${mark.name} — ${status}${finding ? `, ${finding}` : ""}`} onClick={() => openCluster(context)}>
      <Mark decorative name={mark.name} short={mark.short} color={mark.color} size="sm" withBadge={mark.withText}
        icon={mark.mark === "icon" ? symbolFor(mark.icon) : undefined} imageSrc={mark.mark === "image" ? mark.imageSrc : undefined} />
      <span className="min-w-0 flex-1 text-left">
        <span className="block truncate font-medium">{mark.name}</span>
        <span className="block truncate text-xs text-muted" title={secondary}>{secondary}</span>
      </span>
      {finding && <Badge tone={problems > 0 ? "sev" : "warn"}>{problems > 0 ? finding : "Check failed"}</Badge>}
      <StatusPill status={status} kind={paused ? "neutral" : link?.state === "connected" ? "success" : link?.state === "connecting" ? "info" : link?.state === "error" ? "danger" : "neutral"} />
      <span aria-hidden className="text-muted">→</span>
    </button>
  </li>;
}

function HomeAction({ title, icon, route }: { title: string; icon: typeof Icons.settings; route: string }) {
  return <button type="button" className="home-action" aria-label={title} onClick={() => openTab(route)}>
    <NavIcon icon={icon} />
    <span className="min-w-0 truncate text-left font-medium">{title}</span>
    <span aria-hidden className="text-muted">→</span>
  </button>;
}
