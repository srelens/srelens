import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";
import type { listEvents } from "@srelens/core";
import { ResourceEvents } from "./ResourceEvents";

// Typed where it is used, not where it is declared: `vi.fn<typeof listEvents>()`
// pins the mock to the real capability's signature, so widening `EventSummary`
// reddens this file instead of passing an `any` through it. Before that, this
// array satisfied nothing and had drifted two fields behind the backend.
const events = [
  { name: "default/web-1.17a", namespace: "default", type: "Warning", reason: "BackOff", object: "Pod/web-1", message: "restarting", age: "2m", count: 1 },
  { name: "default/other-9.17b", namespace: "default", type: "Normal", reason: "Pulled", object: "other-9", message: "pulled image", age: "5m", count: 1 },
  { name: "default/web-1.17c", namespace: "default", type: "Normal", reason: "Scheduled", object: "Pod/web-1", message: "assigned", age: "6m", count: 1 },
  { name: "default/web-1.17d", namespace: "default", type: "Warning", reason: "Collision", object: "Service/web-1", message: "same name", age: "1m", count: 1 },
];

describe("ResourceEvents", () => {
  it("shows only events involving the object", async () => {
    const listEventsFn = vi.fn<typeof listEvents>().mockResolvedValue({ events });
    render(
      <ResourceEvents
        context="kind-dev"
        namespace="default"
        objectKind="Pod"
        objectName="web-1"
        listEventsFn={listEventsFn}
      />,
    );
    await waitFor(() => expect(screen.getByText("BackOff")).toBeDefined());
    expect(screen.getByText("Scheduled")).toBeDefined(); // matched "Pod/web-1"
    expect(screen.queryByText("Pulled")).toBeNull(); // other object filtered out
    expect(screen.queryByText("Collision")).toBeNull(); // same name, different kind
    expect(listEventsFn).toHaveBeenCalledWith("kind-dev", "default", {
      kind: "Pod",
      name: "web-1",
    });
  });

  it("shows an empty message when no events involve the object", async () => {
    const listEventsFn = vi.fn<typeof listEvents>().mockResolvedValue({ events: [] });
    render(
      <ResourceEvents
        context="kind-dev"
        namespace="default"
        objectKind="Pod"
        objectName="web-1"
        listEventsFn={listEventsFn}
      />,
    );
    await waitFor(() => expect(screen.getByText("No events for this object")).toBeDefined());
  });
});
