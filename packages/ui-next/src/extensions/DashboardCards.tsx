import { Fragment, useEffect, useState, type ReactNode } from "react";
import { CollectionScopeNotice, NamespaceScopePicker } from "../screens/collections/NamespaceScope";
import {
  extensionCardRoute,
  extensionEnabledFor,
  listCrds,
  resolveDashboardCards,
  type ClusterContext,
  type ExtensionDashboardCard,
  type InstalledExtension,
  type ResolvedDashboardCard,
  selectsNothing,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import { Button, LoadingState, MetricTile, Section, Toolbar } from "@srelens/ui-kit";
import { getKubeconfigFiles } from "../lib/clusters";
import { FailureAlert } from "../lib/errorCopy";
import { openTab } from "../lib/tabsStore";
import { useResource } from "../lib/useResource";
import { useInCollectionScope, useNamespaces, useSetNamespaces } from "../lib/workspace";
import { NamespaceErrorAlert } from "../screens/resourceShell";
import { ExtensionResults } from "./ExtensionResults";
import { NO_PINNED_ID_MESSAGE } from "./contextIds";
import { plainText } from "./displayText";
import { extensionLabel, useExtensions } from "./inventoryStore";
import { LiveNotice, LiveStatus, useLiveApps } from "./liveReaders";
import { crdRequirements, readersServed } from "./requirements";

/**
 * The cluster dashboard's app cards (#540): one band of host-drawn figures,
 * each declared by an installed app over one of its granted readers.
 *
 * Every card is in exactly one of three states besides its figure, and no
 * two of them look alike, because two of them are easy to confuse and the
 * confusion is the bug this repo keeps finding: **loading** has a spinner and
 * no figure; **couldn't read** has the reason and a retry, and no figure;
 * **none** has a figure, `0`, said as an answer. A failed read is never drawn
 * as zero.
 *
 * Nothing here renders app markup. Titles, names and values come from the
 * manifest or the cluster and are drawn through `plainText`.
 */
export function DashboardCards({ context }: { context: ClusterContext }) {
  const inventory = useExtensions();
  if (inventory.status === "error") {
    return (
      <Section title="App cards" smallCaps padded={false}>
        <div className="px-3 py-2.5">
          <FailureAlert tone="sev" title="App cards could not be listed" error={inventory.error} onRetry={inventory.reload} />
        </div>
      </Section>
    );
  }
  // An app that is off, blocked or quarantined, or not enabled for this
  // cluster, contributes nothing here — the host would refuse its reads anyway.
  const apps = (inventory.data?.plugins ?? []).filter(
    (plugin) =>
      plugin.enabled &&
      !plugin.quarantined &&
      !plugin.policyBlocked &&
      extensionEnabledFor(plugin, context.key) &&
      (plugin.manifest.contributions.dashboardCards?.length ?? 0) > 0,
  );
  // Still listing, or nothing declares a card: there is no band to draw, and a
  // placeholder on every cluster's overview would be furniture.
  if (apps.length === 0) return null;
  return <ServedCards context={context} apps={apps} />;
}

/**
 * The band, over only the cards whose reader this cluster serves (#1024): a
 * card over a missing CRD would only say "Couldn't read", and its watch would
 * stop the band's live updates. No band while that is unknown, nor when no card
 * is left. A discovery that fails is unknown rather than missing — ordinary
 * RBAC often refuses it — so every card shows, as on the app's own page.
 */
function ServedCards({ context, apps }: { context: ClusterContext; apps: InstalledExtension[] }) {
  const cardsOf = (plugin: InstalledExtension) => plugin.manifest.contributions.dashboardCards ?? [];
  const needed = apps.some((plugin) => cardsOf(plugin).some((card) => crdRequirements(plugin, new Set([card.source])).length));
  // The cluster is asked, by pinned ID, only when a card reads a CRD. An answer
  // to another question — another cluster's, say — is no answer.
  const key = (needed && context.pinnedId) || "";
  // ponytail: one uncached listCrds per Overview; share the sidebar's if it shows up in a profile.
  const discovery = useResource(async () => ({ key, ...(key ? await listCrds(key) : {}) }), [key]);
  if (discovery.data?.key !== key) return null;
  const crds = discovery.data.crds;
  const served = apps
    .map((plugin) => ({ ...plugin, manifest: { ...plugin.manifest, contributions: { ...plugin.manifest.contributions,
      dashboardCards: cardsOf(plugin).filter((card) => readersServed(plugin, new Set([card.source]), crds)) } } }))
    .filter((plugin) => plugin.manifest.contributions.dashboardCards.length);
  // Refresh asks again too, so an operator installed since shows its cards. Quietly: the band
  // stays up until the answer lands.
  return served.length ? <CardsBand context={context} apps={served} recheck={discovery.refresh} /> : null;
}

function CardsBand({ context, apps, recheck }: { context: ClusterContext; apps: InstalledExtension[]; recheck(): void }) {
  const selection = useNamespaces(context.stableId);
  const setNamespaces = useSetNamespaces();
  const namespaceOptions = useNamespaceOptions(context.name, getKubeconfigFiles());
  const { namespaces, scope, error: namespaceError } = namespaceOptions;
  const [refresh, setRefresh] = useState(0);
  const scopedByCollection = useInCollectionScope(context.stableId);
  // A credential that may only read one namespace reads that one, whatever is
  // selected — unless the tab is scoped to a namespace collection (#881),
  // which has already resolved against what those credentials can see, and
  // may have matched nothing.
  const effective = scope && !scopedByCollection ? [scope] : selection;
  // Each app's card readers are followed (#566): a change redraws that app's
  // figures in place. One namespace is watched there; several, or none, in
  // every namespace — the scope the cards themselves are read in.
  const [pulses, setPulses] = useState<Record<string, number>>({});
  // The host is asked by pinned ID, which names this context alone even when another shares
  // its stable ID (#695). A cluster listed without one cannot be asked at all.
  const host = context.pinnedId;
  const live = useLiveApps({
    apps: !host || namespaces === null ? [] : apps.map((plugin) => ({
      plugin, capabilities: (plugin.manifest.contributions.dashboardCards ?? []).map((card) => card.source),
    })),
    context: host ?? "",
    namespace: effective.length === 1 ? effective[0] : "",
    label: "dashboard:cards",
    // Why nothing is followed yet: the same reasons nothing is read yet.
    off: !host ? NO_PINNED_ID_MESSAGE
      : namespaces === null ? "Waiting for the cluster's namespaces before following the cards." : undefined,
    onChange: (id) => setPulses((current) => ({ ...current, [id]: (current[id] ?? 0) + 1 })),
  });
  const stale = live.state === "reconnecting";
  return (
    <Section title="App cards" smallCaps padded={false} className="dashboard-cards-band">
      <Toolbar wrap className="justify-end gap-x-2">
        <NamespaceScopePicker context={context} options={namespaceOptions} />
        <LiveStatus live={live} />
        <Button variant="secondary" size="sm" aria-label="Refresh app cards" onClick={() => { recheck(); setRefresh((n) => n + 1); }}>
          Refresh
        </Button>
      </Toolbar>
      <NamespaceErrorAlert error={namespaceError} />
      <CollectionScopeNotice context={context} />
      <LiveNotice live={live} what="figures" />
      {!host ? (
        <p className="px-3 py-2.5 text-[0.75rem] text-muted">{NO_PINNED_ID_MESSAGE}</p>
      ) : (
        <div className="dashboard-cards" data-stale={stale || undefined}>
          {apps.map((plugin) =>
            // Until the namespaces answer, a restricted credential's scope is not
            // known, and a read of "every namespace" would draw its refusal as a
            // failure the card does not have. So nothing reads yet.
            namespaces === null ? (
              <PendingAppCards key={plugin.manifest.id} plugin={plugin} />
            ) : (
              <AppCards key={plugin.manifest.id} plugin={plugin} host={host} selection={effective}
                refresh={refresh} pulse={pulses[plugin.manifest.id] ?? 0}
                onOpen={(card) => openTab(extensionCardRoute(context.key, plugin.manifest.id, card.target!.page, effective.length === 1 ? effective[0] : "", card.id, effective), { clusterName: context.name })} />
            ),
          )}
        </div>
      )}
    </Section>
  );
}

/** One app's cards before anything may be read: each is loading. */
function PendingAppCards({ plugin }: { plugin: InstalledExtension }) {
  const appName = extensionLabel(plugin);
  return (
    <>
      {(plugin.manifest.contributions.dashboardCards ?? []).map((card) => (
        <DashboardCard key={card.id} card={card} appName={appName} answer={undefined} retry={() => {}} />
      ))}
    </>
  );
}

/** One app's cards: one host call answers all of them. */
export function AppCards({
  plugin,
  host,
  selection,
  refresh,
  pulse = 0,
  cards = plugin.manifest.contributions.dashboardCards ?? [],
  onOpen,
  previewLists = false,
}: {
  plugin: InstalledExtension;
  /** The context's pinned ID, which the host is asked by. */
  host: string;
  selection: string[];
  refresh: number;
  /** Bumped when a watch saw one of this app's card readers change. */
  pulse?: number;
  cards?: ExtensionDashboardCard[];
  onOpen(card: ExtensionDashboardCard): void;
  previewLists?: boolean;
}) {
  const { id } = plugin.manifest;
  const answers = useResource(
    // A namespace collection that matches nothing (#881) has no figures to read.
    async () => (selectsNothing(selection) ? { cards: [] } : resolveDashboardCards(id, plugin.revision, host, selection)),
    [id, plugin.revision, host, selection.join("\u0000"), refresh],
    () => false,
  );
  const { refresh: reread } = answers;
  // Redrawn in place: a figure that flashed back to a spinner on every change would be unreadable.
  useEffect(() => { if (pulse) reread(); }, [pulse, reread]);
  const appName = extensionLabel(plugin);
  return (
    <>
      {cards.map((card) => {
        const answer: ResolvedDashboardCard | undefined =
          answers.status === "error"
            ? { id: card.id, state: "error", reason: answers.error ?? "" }
            : answers.status === "loading"
              ? undefined
              : (answers.data?.cards.find((found) => found.id === card.id) ?? {
                  id: card.id,
                  state: "error",
                  reason: "srelens returned no answer for this card; refresh the view",
                });
        const target = card.target;
        if (previewLists && card.type === "list" && answer?.state === "list" && answer.total > 0) {
          return <section key={card.id} className="extension-overview-list" aria-label={plainText(card.title)}>
            <Toolbar wrap><h3 className="toolbar-title mr-auto">{plainText(card.title)}</h3>{target && <Button variant="ghost" onClick={()=>onOpen(card)}>View all matching</Button>}</Toolbar>
            <ExtensionResults plugin={plugin} capability={card.source} context={host} namespaces={selection} refresh={refresh} hideToolbar card={card.id} previewRows={answer.rows}/>
          </section>;
        }
        const windowSetting = /^\$\{settings\.([^}]+)\}$/.exec(card.predicate?.within ?? "")?.[1];
        const setting = plugin.manifest.settings?.find(setting=>setting.id===windowSetting);
        const windowValue = windowSetting ? plugin.settings?.[windowSetting] ?? setting?.default : card.predicate?.within;
        const windowLabel = typeof windowValue === "string" ? setting?.options?.find(option=>option.value===windowValue)?.label ?? windowValue : undefined;
        return (
          <DashboardCard
            key={card.id}
            card={card}
            appName={appName}
            windowLabel={windowLabel}
            answer={answer}
            retry={answers.reload}
            open={target ? () => onOpen(card) : undefined}
          />
        );
      })}
    </>
  );
}

type CardState = "loading" | "error" | "zero" | "value";

function stateOf(answer: ResolvedDashboardCard | undefined): CardState {
  if (!answer) return "loading";
  switch (answer.state) {
    case "error":
      return answer.state;
    case "count":
      return answer.count === 0 ? "zero" : "value";
    case "countByStatus":
    case "list":
      return answer.total === 0 ? "zero" : "value";
    case "metric":
      return answer.value === null || answer.value === 0 ? "zero" : "value";
  }
}

const figure = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 2 });

const AGGREGATE = { sum: "Sum", min: "Lowest", max: "Highest" } as const;

function DashboardCard({
  card,
  appName,
  answer,
  retry,
  open,
  windowLabel,
}: {
  card: ExtensionDashboardCard;
  appName: string;
  answer: ResolvedDashboardCard | undefined;
  retry: () => void;
  open?: () => void;
  windowLabel?: string;
}) {
  const title = plainText(card.title);
  const state = stateOf(answer);
  const { value, body, caption } = cardParts(card, title, answer, retry);
  // The caption, the window it counts over and the app it is from, one line under the figure.
  const description = [caption, windowLabel && `Window: ${plainText(windowLabel)}`, plainText(appName)]
    .filter(Boolean)
    .map((part, index) => (
      <Fragment key={index}>
        {index > 0 && " · "}
        <span>{part}</span>
      </Fragment>
    ));
  return (
    // The band lays out this cell, by the card's state and size; the kit's tile draws the card.
    <div className="dashboard-tile" data-state={state} data-size={state === "zero" ? "s" : card.size}>
      <MetricTile
        label={title}
        onOpen={open}
        // None is an answer, drawn quieter than a count but still a figure.
        value={value !== undefined && (state === "zero" ? <span className="text-muted">{value}</span> : value)}
        description={description}
      >
        {body}
      </MetricTile>
    </div>
  );
}

/**
 * What a card shows for its answer: a figure, or something under where the figure would be,
 * and the caption that says what the figure is. Loading and a failed read have no figure.
 */
function cardParts(
  card: ExtensionDashboardCard,
  title: string,
  answer: ResolvedDashboardCard | undefined,
  retry: () => void,
): { value?: ReactNode; body?: ReactNode; caption?: ReactNode } {
  if (!answer) return { body: <LoadingState compact /> };
  switch (answer.state) {
    case "error":
      return {
        body: (
          <FailureAlert tone="sev" title="Couldn’t read" error={answer.reason} domain="cluster" onRetry={retry} retryLabel={`Retry ${title}`} />
        ),
      };
    case "count":
      return { value: figure(answer.count), caption: answer.count === 0 ? "None match" : "Matching" };
    case "countByStatus":
      if (answer.total === 0) return { value: "0", caption: "None match" };
      return {
        value: figure(answer.total),
        body: (
          <ul className="text-[0.75rem]">
            {answer.statuses.map((row) => (
              <li key={row.status} className="flex justify-between gap-2">
                <span>{plainText(row.status)}</span> <span className="num">{figure(row.count)}</span>
              </li>
            ))}
          </ul>
        ),
      };
    case "metric": {
      if (answer.value === null) {
        return {
          value: "No value",
          caption: (
            <>
              No matching resource has a number at <span className="path">{plainText(card.metric?.jsonPath ?? "")}</span>
            </>
          ),
        };
      }
      const aggregate = card.metric ? AGGREGATE[card.metric.aggregate] : "Value";
      return {
        value: figure(answer.value),
        caption: `${aggregate} over ${figure(answer.counted)} ${answer.counted === 1 ? "resource" : "resources"}`,
      };
    }
    case "list":
      if (answer.total === 0) return { value: "0", caption: "None match" };
      return {
        body: (
          <ol className="text-[0.75rem]">
            {answer.rows.map((row) => (
              // Machine text keeps its line and scrolls rather than wrapping.
              <li key={`${row.namespace}/${row.name}`} className="flex justify-between gap-2 overflow-x-auto whitespace-nowrap">
                <span className="path">{plainText(row.namespace ? `${row.namespace}/${row.name}` : row.name)}</span>
                {row.value !== undefined && <span className="num text-muted">{plainText(row.value)}</span>}
              </li>
            ))}
          </ol>
        ),
        caption: `${figure(answer.rows.length)} of ${figure(answer.total)}`,
      };
  }
}
