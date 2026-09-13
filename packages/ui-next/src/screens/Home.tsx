import { useEffect, useRef, useState } from "react";
import { describeError, listContexts, type ClusterContext } from "@srelens/core";
import { Button, EmptyState, LoadingState, Mark, NavIcon, Screen, StatusPill, TextInput } from "@srelens/ui-kit";
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

/** App-wide entry point. Contexts and connection status come from the shell's shared stores. */
export function Home() {
  const contexts = useContexts();
  const status = useContextsStatus();
  const error = useContextsError();
  const ordered = useOrderedContexts(contexts);
  // Like the rail, subscribe once so filtering follows saved display-name edits.
  useEditableMark("", "");
  const { links } = useWorkspaceView();
  const { workspace } = useTabs();
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
              <ul className="home-cluster-list scroll" aria-label="Saved clusters">
                {filtered.map(ctx => <ClusterRow key={ctx.stableId} context={ctx} link={links[ctx.stableId]} paused={workspace.pausedClusters?.includes(ctx.stableId) === true} />)}
              </ul>
            )}
          </section>
          <aside className="home-start" aria-label="Workspace tools">
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

function ClusterRow({ context, link, paused }: { context: ClusterContext; link?: ReturnType<typeof useWorkspaceView>["links"][string]; paused: boolean }) {
  const mark = getMark(context.stableId, context.name);
  const status = paused ? "Paused" : link ? LINK_WORD[link.state] : "Not checked";
  let secondary = context.name;
  if (mark.name === context.name) {
    try { secondary = new URL(context.server).host; }
    catch { secondary = context.cluster !== context.name ? context.cluster : context.sourceFile; }
  }
  return <li className="border-b border-rule">
    <button type="button" className="home-cluster-row" aria-label={`Open cluster ${mark.name} — ${status}`} onClick={() => openCluster(context)}>
      <Mark decorative name={mark.name} short={mark.short} color={mark.color} size="sm" withBadge={mark.withText}
        icon={mark.mark === "icon" ? symbolFor(mark.icon) : undefined} imageSrc={mark.mark === "image" ? mark.imageSrc : undefined} />
      <span className="min-w-0 flex-1 text-left">
        <span className="block truncate font-medium">{mark.name}</span>
        <span className="block truncate text-xs text-muted" title={secondary}>{secondary}</span>
      </span>
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
