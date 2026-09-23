import { useState } from "react";
import {
  describeError,
  extensionCardRoute,
  extensionEnabledFor,
  resolveDashboardCards,
  type ClusterContext,
  type ExtensionDashboardCard,
  type InstalledExtension,
  type ResolvedDashboardCard,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import { Button, RawError, Section, Spinner } from "@srelens/ui-kit";
import { getKubeconfigFiles, useContexts } from "../lib/clusters";
import { FailureAlert } from "../lib/errorCopy";
import { openTab } from "../lib/tabsStore";
import { useResource } from "../lib/useResource";
import { setNamespaces, useNamespaces } from "../lib/workspace";
import { NamespaceErrorAlert, NamespacePicker } from "../screens/resourceShell";
import { SHARED_CONTEXT_ID_MESSAGE } from "./contextIds";
import { plainText } from "./displayText";
import { extensionLabel, useExtensions } from "./inventoryStore";

/**
 * The cluster dashboard's app cards (#540): one band of host-drawn figures,
 * each declared by an installed app over one of its granted readers.
 *
 * Every card is in exactly one of four states, and no two of them look
 * alike, because two of them are easy to confuse and the confusion is the
 * bug this repo keeps finding: **loading** has a spinner and no figure;
 * **couldn't read** has the reason and a retry, and no figure; **none** has a
 * figure, `0`, said as an answer; **not available yet** is a card this host
 * cannot make at all. A failed read is never drawn as zero.
 *
 * Nothing here renders app markup. Titles, names and values come from the
 * manifest or the cluster and are drawn through `plainText`.
 */
export function DashboardCards({ context }: { context: ClusterContext }) {
  const inventory = useExtensions();
  if (inventory.status === "error") {
    return (
      <Section title="App cards" smallCaps padded={false}>
        <div className="dashboard-cards-message">
          <FailureAlert tone="sev" title="App cards could not be listed" error={inventory.error} />
          <Button variant="secondary" size="sm" onClick={inventory.reload}>Retry</Button>
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
  return <CardsBand context={context} apps={apps} />;
}

function CardsBand({ context, apps }: { context: ClusterContext; apps: InstalledExtension[] }) {
  const selection = useNamespaces(context.stableId);
  const { namespaces, scope, error: namespaceError } = useNamespaceOptions(context.name, getKubeconfigFiles());
  const [refresh, setRefresh] = useState(0);
  const contexts = useContexts();
  // A credential that may only read one namespace reads that one, whatever is selected.
  const effective = scope ? [scope] : selection;
  // Two contexts with one stable ID: a read pinned by it cannot say which cluster it is for.
  const shared = contexts.filter((c) => c.stableId === context.stableId).length > 1;
  return (
    <Section title="App cards" smallCaps padded={false} className="dashboard-cards-band">
      <div className="dashboard-cards-toolbar">
        <NamespacePicker
          namespaces={namespaces}
          selection={selection}
          onChange={(next) => setNamespaces(context.stableId, next)}
        />
        <Button variant="secondary" size="sm" aria-label="Refresh app cards" onClick={() => setRefresh((n) => n + 1)}>
          Refresh
        </Button>
      </div>
      <NamespaceErrorAlert error={namespaceError} />
      {shared ? (
        <p className="dashboard-cards-message">{SHARED_CONTEXT_ID_MESSAGE}</p>
      ) : (
        <div className="dashboard-cards">
          {apps.map((plugin) =>
            // Until the namespaces answer, a restricted credential's scope is not
            // known, and a read of "every namespace" would draw its refusal as a
            // failure the card does not have. So nothing reads yet.
            namespaces === null ? (
              <PendingAppCards key={plugin.manifest.id} plugin={plugin} />
            ) : (
              <AppCards key={plugin.manifest.id} plugin={plugin} context={context} selection={effective} refresh={refresh} />
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
function AppCards({
  plugin,
  context,
  selection,
  refresh,
}: {
  plugin: InstalledExtension;
  context: ClusterContext;
  selection: string[];
  refresh: number;
}) {
  const { id } = plugin.manifest;
  const answers = useResource(
    () => resolveDashboardCards(id, plugin.revision, context.stableId, selection),
    [id, plugin.revision, context.stableId, selection.join("\u0000"), refresh],
    () => false,
  );
  const appName = extensionLabel(plugin);
  // The target page reads the one namespace in its path, or all of them narrowed to the selection.
  const namespace = selection.length === 1 ? selection[0] : "";
  return (
    <>
      {(plugin.manifest.contributions.dashboardCards ?? []).map((card) => {
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
        return (
          <DashboardCard
            key={card.id}
            card={card}
            appName={appName}
            answer={answer}
            retry={answers.reload}
            open={
              target
                ? () =>
                    openTab(extensionCardRoute(context.stableId, id, target.page, namespace, card.id, selection), {
                      clusterName: context.name,
                    })
                : undefined
            }
          />
        );
      })}
    </>
  );
}

type CardState = "loading" | "error" | "unavailable" | "zero" | "value";

function stateOf(answer: ResolvedDashboardCard | undefined): CardState {
  if (!answer) return "loading";
  switch (answer.state) {
    case "error":
    case "unavailable":
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
}: {
  card: ExtensionDashboardCard;
  appName: string;
  answer: ResolvedDashboardCard | undefined;
  retry: () => void;
  open?: () => void;
}) {
  const title = plainText(card.title);
  const state = stateOf(answer);
  return (
    <section className="dashboard-card" aria-label={title} data-state={state} data-size={card.size}>
      <header className="dashboard-card-head">
        <span className="dashboard-card-app">{plainText(appName)}</span>
        {open ? (
          <button type="button" className="dashboard-card-title dashboard-card-link" aria-label={`Open ${title}`} onClick={open}>
            {title}
          </button>
        ) : (
          <h3 className="dashboard-card-title">{title}</h3>
        )}
      </header>
      <CardBody card={card} title={title} answer={answer} retry={retry} />
    </section>
  );
}

function CardBody({
  card,
  title,
  answer,
  retry,
}: {
  card: ExtensionDashboardCard;
  title: string;
  answer: ResolvedDashboardCard | undefined;
  retry: () => void;
}) {
  if (!answer) {
    return (
      <p role="status" className="dashboard-card-loading">
        <Spinner role="presentation" aria-hidden="true" />
        Loading…
      </p>
    );
  }
  switch (answer.state) {
    case "error": {
      const { title: headline, raw } = describeError(answer.reason, { domain: "cluster" });
      return (
        <div role="alert" className="dashboard-card-error">
          <p>
            <strong>Couldn’t read</strong> · {headline}
          </p>
          <RawError text={raw || answer.reason} />
          <Button variant="secondary" size="sm" aria-label={`Retry ${title}`} onClick={retry}>
            Retry
          </Button>
        </div>
      );
    }
    case "unavailable":
      return (
        <p className="dashboard-card-note">
          <strong>Not available yet</strong> · {plainText(answer.reason)}
        </p>
      );
    case "count":
      return (
        <>
          <p className="dashboard-card-figure">{figure(answer.count)}</p>
          <p className="dashboard-card-caption">{answer.count === 0 ? "None match" : "Matching"}</p>
        </>
      );
    case "countByStatus":
      return (
        <>
          <p className="dashboard-card-figure">{figure(answer.total)}</p>
          {answer.total === 0 ? (
            <p className="dashboard-card-caption">None match</p>
          ) : (
            <ul className="dashboard-card-statuses">
              {answer.statuses.map((row) => (
                <li key={row.status}>
                  <span>{plainText(row.status)}</span> <span className="num">{figure(row.count)}</span>
                </li>
              ))}
            </ul>
          )}
        </>
      );
    case "metric": {
      const aggregate = card.metric ? AGGREGATE[card.metric.aggregate] : "Value";
      if (answer.value === null) {
        return (
          <>
            <p className="dashboard-card-figure">No value</p>
            <p className="dashboard-card-caption">
              No matching resource has a number at <span className="path">{plainText(card.metric?.jsonPath ?? "")}</span>
            </p>
          </>
        );
      }
      return (
        <>
          <p className="dashboard-card-figure">{figure(answer.value)}</p>
          <p className="dashboard-card-caption">
            {aggregate} over {figure(answer.counted)} {answer.counted === 1 ? "resource" : "resources"}
          </p>
        </>
      );
    }
    case "list":
      if (answer.total === 0) {
        return (
          <>
            <p className="dashboard-card-figure">0</p>
            <p className="dashboard-card-caption">None match</p>
          </>
        );
      }
      return (
        <>
          <ol className="dashboard-card-rows">
            {answer.rows.map((row) => (
              <li key={`${row.namespace}/${row.name}`} className="dashboard-card-row">
                <span className="path">{plainText(row.namespace ? `${row.namespace}/${row.name}` : row.name)}</span>
                {row.value !== undefined && <span className="dashboard-card-row-value">{plainText(row.value)}</span>}
              </li>
            ))}
          </ol>
          <p className="dashboard-card-caption">
            {figure(answer.rows.length)} of {figure(answer.total)}
          </p>
        </>
      );
  }
}
