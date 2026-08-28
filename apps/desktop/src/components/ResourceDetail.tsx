import React, { useState } from "react";
import { Tabs } from "../ui";
import { ResourceOverview } from "./ResourceOverview";
import { ResourceEvents } from "./ResourceEvents";
import { YamlView } from "./YamlView";
import type { OpenResource } from "@srelens/core";

type DetailTab = "overview" | "yaml" | "events";

const TABS = [
  { id: "overview", label: "Overview" },
  { id: "yaml", label: "YAML" },
  { id: "events", label: "Events" },
];

/**
 * The tabbed body of a resource detail drawer: a structured Overview, the
 * editable YAML, and the object's Events. Write actions live in the drawer
 * header (see DetailActions), so the body is just the tabs.
 */
export function ResourceDetail({
  context,
  kind,
  namespace,
  name,
  reloadKey = 0,
  onOpenResource,
  onOpenLogs,
  onOpenExec,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  /** Bumped by the parent after a write action to refresh the overview. */
  reloadKey?: number;
  onOpenResource?: OpenResource;
  /** Open logs scoped to a container (Pod detail only). */
  onOpenLogs?: (container: string) => void;
  /** Open an exec session in a container (Pod detail only). */
  onOpenExec?: (container: string) => void;
}) {
  const [tab, setTab] = useState<DetailTab>("overview");

  return (
    <div>
      <Tabs tabs={TABS} active={tab} onChange={(id) => setTab(id as DetailTab)} />
      <div className="fl-detail-tabpanel">
        {tab === "overview" && (
          <ResourceOverview
            context={context}
            kind={kind}
            namespace={namespace}
            name={name}
            reloadKey={reloadKey}
            onOpenResource={onOpenResource}
            onOpenLogs={onOpenLogs}
            onOpenExec={onOpenExec}
          />
        )}
        {tab === "yaml" && (
          <YamlView context={context} kind={kind} namespace={namespace} name={name} />
        )}
        {tab === "events" && (
          <ResourceEvents
            context={context}
            namespace={namespace}
            objectKind={kind}
            objectName={name}
          />
        )}
      </div>
    </div>
  );
}
