import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";

const { listCustomResourceMock, listNamespacesMock } = vi.hoisted(() => ({
  listCustomResourceMock: vi.fn(),
  listNamespacesMock: vi.fn(),
}));
vi.mock("@srelens/core/lib/crds", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/crds")>();
  return { ...actual, listCustomResource: listCustomResourceMock };
});
vi.mock("@srelens/core/lib/workloads", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/workloads")>();
  return { ...actual, listNamespaces: listNamespacesMock };
});

import { CustomResourceBrowser } from "./CustomResourceBrowser";

const crd = {
  name: "widgets.example.com",
  group: "example.com",
  version: "v1",
  kind: "Widget",
  plural: "widgets",
  namespaced: true,
};

beforeEach(() => {
  listCustomResourceMock.mockReset();
  listNamespacesMock.mockReset();
  listNamespacesMock.mockResolvedValue({ namespaces: ["default"] });
});

describe("CustomResourceBrowser", () => {
  it("lists custom resource instances", async () => {
    listCustomResourceMock.mockResolvedValue({
      items: [{ name: "demo-widget", namespace: "default", age: "1m" }],
    });
    render(<CustomResourceBrowser context="kind-dev" crd={crd} />);
    await waitFor(() => expect(screen.getByText("demo-widget")).toBeDefined());
    expect(listCustomResourceMock).toHaveBeenCalledWith("kind-dev", crd, "");
    // the kind name heads the first column
    expect(screen.getByText("Widget")).toBeDefined();
  });

  it("renders the columns the CRD asks for, between the name and Age (#267)", async () => {
    const withColumns = {
      ...crd,
      printerColumns: [
        { name: "Health", jsonPath: ".status.health", type: "string" },
        { name: "Nodes", jsonPath: ".spec.nodes", type: "integer" },
      ],
    };
    listCustomResourceMock.mockResolvedValue({
      items: [{ name: "demo-widget", namespace: "default", age: "1m", columns: ["GREEN", "3"] }],
    });
    render(<CustomResourceBrowser context="kind-dev" crd={withColumns} />);
    await waitFor(() => expect(screen.getByText("demo-widget")).toBeDefined());
    // Headings from the CRD, and the resolved values on the row.
    expect(screen.getByText("Health")).toBeDefined();
    expect(screen.getByText("Nodes")).toBeDefined();
    expect(screen.getByText("GREEN")).toBeDefined();
    expect(screen.getByText("3")).toBeDefined();
  });

  it("still lists a CRD that declares no printer columns (#267)", async () => {
    listCustomResourceMock.mockResolvedValue({
      items: [{ name: "demo-widget", namespace: "default", age: "1m" }],
    });
    render(<CustomResourceBrowser context="kind-dev" crd={crd} />);
    await waitFor(() => expect(screen.getByText("demo-widget")).toBeDefined());
    expect(screen.getByText("Age")).toBeDefined();
  });

  it("shows an error when listing fails", async () => {
    listCustomResourceMock.mockResolvedValue({ error: "the server could not find the requested resource" });
    render(<CustomResourceBrowser context="kind-dev" crd={crd} />);
    await waitFor(() => expect(screen.getByText(/could not find/)).toBeDefined());
  });
});
