import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { K8sObject, PrinterColumn } from "@srelens/core";
import { CustomResourceBody, customResourceFacts } from "./CustomResourceBody";
import type { DetailFact } from "./facts";

const COLUMNS: PrinterColumn[] = [
  { name: "Sync Status", jsonPath: ".status.sync.status", type: "string" },
  { name: "Health Status", jsonPath: ".status.health.status", type: "string" },
  { name: "Revision", jsonPath: ".status.sync.revision", type: "string" },
];

const APP: K8sObject = {
  apiVersion: "argoproj.io/v1alpha1",
  kind: "Application",
  metadata: { name: "guestbook", namespace: "argocd", generation: 5 },
  status: { sync: { status: "Synced" }, health: { status: "Healthy" }, observedGeneration: 5 },
};

const labels = (facts: DetailFact[]) => facts.map((f) => f.label);
const value = (facts: DetailFact[], label: string) => facts.find((f) => f.label === label)?.value;
const daysAgo = (n: number) => new Date(Date.now() - n * 86_400_000).toISOString();

describe("customResourceFacts", () => {
  it("leads with the CRD's own printer columns and leaves out one with no value", () => {
    const facts = customResourceFacts(APP, COLUMNS);
    expect(labels(facts)).toEqual(["Sync Status", "Health Status", "Namespace"]);
    expect(value(facts, "Sync Status")).toBe("Synced");
    expect(value(facts, "Health Status")).toBe("Healthy");
  });

  it("reads a date as an age, and a number, a boolean or a map as text", () => {
    const object: K8sObject = {
      metadata: { name: "w" },
      spec: { replicas: 3, paused: false, selector: { app: "web" } },
      status: { lastSync: daysAgo(3) },
    };
    const facts = customResourceFacts(object, [
      { name: "Last Sync", jsonPath: ".status.lastSync", type: "date" },
      { name: "Replicas", jsonPath: ".spec.replicas", type: "integer" },
      { name: "Paused", jsonPath: ".spec.paused", type: "boolean" },
      { name: "Selector", jsonPath: ".spec.selector", type: "string" },
    ]);
    expect(value(facts, "Last Sync")).toBe("3d");
    expect(value(facts, "Replicas")).toBe("3");
    expect(value(facts, "Paused")).toBe("false");
    expect(value(facts, "Selector")).toBe('{"app":"web"}');
  });

  it("reads a filtered column the way kubectl does", () => {
    const object: K8sObject = {
      metadata: { name: "c" },
      status: { conditions: [{ type: "Issuing", status: "False" }, { type: "Ready", status: "True" }] },
    };
    const facts = customResourceFacts(object, [
      { name: "Ready", jsonPath: '.status.conditions[?(@.type=="Ready")].status', type: "string" },
    ]);
    expect(value(facts, "Ready")).toBe("True");
  });

  it("adds the phase only when no printer column already shows it", () => {
    const object: K8sObject = { metadata: { name: "w" }, status: { phase: "Running" } };
    expect(value(customResourceFacts(object), "Phase")).toBe("Running");
    const shown = customResourceFacts(object, [{ name: "State", jsonPath: ".status.phase", type: "string" }]);
    expect(labels(shown)).toEqual(["State"]);
  });

  it("says the controller is behind only when observedGeneration lags generation", () => {
    const behind: K8sObject = { metadata: { name: "w", generation: 5 }, status: { observedGeneration: 4 } };
    expect(value(customResourceFacts(behind), "Observed generation")).toBe("4 of 5");
    expect(labels(customResourceFacts(APP))).not.toContain("Observed generation");
  });

  it("still gives the identity facts for a resource with no column values and no status yet", () => {
    const fresh: K8sObject = { metadata: { name: "w", namespace: "default" } };
    expect(labels(customResourceFacts(fresh, COLUMNS))).toEqual(["Namespace"]);
  });
});

describe("CustomResourceBody", () => {
  it("shows the conditions, then the whole status as YAML", () => {
    const object: K8sObject = {
      metadata: { name: "guestbook" },
      status: { sync: { status: "Synced" }, conditions: [{ type: "Ready", status: "True", reason: "Done" }] },
    };
    const { container } = render(<CustomResourceBody object={object} name="guestbook" />);
    expect(screen.getByRole("heading", { level: 3, name: "Conditions" })).toBeDefined();
    expect(screen.getByRole("heading", { level: 3, name: "Raw status" })).toBeDefined();
    expect(container.querySelector(".cm-content")?.textContent).toContain("Synced");
  });

  it("draws nothing for a resource with no status", () => {
    const { container } = render(<CustomResourceBody object={{ metadata: { name: "w" } }} name="w" />);
    expect(container.textContent).toBe("");
  });
});
