import React, { useEffect, useState } from "react";
import { listEvents, type EventSummary } from "@srelens/core";
import { Spinner, StatusPill, Table, type Column } from "../ui";
import { ageSortValue } from "@srelens/core";

/**
 * Events involving a single object, shown in its detail drawer. Fetches the
 * events via `k8s.listEvents`, scoped by the object's exact kind and name.
 * `listEventsFn` is injectable for testing.
 */
export function ResourceEvents({
  context,
  namespace,
  objectKind,
  objectName,
  listEventsFn = listEvents,
}: {
  context: string;
  namespace: string | null;
  objectKind: string;
  objectName: string;
  listEventsFn?: typeof listEvents;
}) {
  const [events, setEvents] = useState<EventSummary[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    setEvents(null);
    setError("");
    void listEventsFn(context, namespace, { kind: objectKind, name: objectName }).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else setEvents(out.events ?? []);
    });
    return () => {
      active = false;
    };
  }, [context, namespace, objectKind, objectName, listEventsFn]);

  if (error) return <p style={{ color: "var(--fl-color-danger)" }}>Error: {error}</p>;
  if (events === null) return <Spinner label="Loading events" />;

  // Keep a defensive exact filter in case an older backend ignores selectors.
  // Splitting on the first slash also handles object names that contain slashes
  // without accepting another resource kind with the same name.
  const mine = events.filter((event) => {
    const separator = event.object.indexOf("/");
    if (separator < 0) return event.object === objectName;
    const kind = event.object.slice(0, separator);
    const name = event.object.slice(separator + 1);
    return kind.toLowerCase() === objectKind.toLowerCase() && name === objectName;
  });
  const columns: Column<EventSummary>[] = [
    {
      key: "type",
      header: "Type",
      render: (e) => (
        <StatusPill status={e.type} kind={e.type === "Warning" ? "warning" : "neutral"} />
      ),
    },
    { key: "reason", header: "Reason", render: (e) => e.reason },
    { key: "message", header: "Message", render: (e) => e.message },
    { key: "age", header: "Age", getSortValue: ageSortValue, render: (e) => e.age },
  ];

  return (
    <Table
      columns={columns}
      data={mine}
      getRowKey={(e) => e.name || `${e.reason}-${e.message}-${e.age}`}
      emptyText="No events for this object"
    />
  );
}
