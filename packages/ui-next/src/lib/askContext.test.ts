import { describe, it, expect } from "vitest";
import { askContextFor } from "./askContext";
import { logsRoute } from "../screens/Logs";
import { detailRoute } from "./detailRoute";

describe("what a question asked from a route is about", () => {
  // The case a screenshot caught: the reader is on a pod's logs, presses
  // "Summarise this stream", and the agent received only the cluster — so it
  // searched four namespaces looking for a pod to read.
  it("carries the pod's namespace and name off a logs route", () => {
    const route = logsRoute("Pod", "m01-cnips-01-services", "ai-editor");
    expect(askContextFor(route, "prod-eu")).toEqual({
      cluster: "prod-eu",
      namespace: "m01-cnips-01-services",
      kind: "Pod",
      name: "ai-editor",
      surface: "logs",
    });
  });

  it("carries the resource off a detail route, with no logs claim", () => {
    const route = detailRoute("Deployment", "checkout", "api");
    expect(askContextFor(route, "prod-eu")).toEqual({
      cluster: "prod-eu",
      namespace: "checkout",
      kind: "Deployment",
      name: "api",
    });
  });

  it("omits the namespace for a cluster-scoped resource rather than sending null", () => {
    const route = detailRoute("Node", null, "worker-3");
    const about = askContextFor(route, "prod-eu");
    expect(about.namespace).toBeUndefined();
    expect(about).toEqual({ cluster: "prod-eu", kind: "Node", name: "worker-3" });
  });

  it("says only the cluster for a route that names no resource", () => {
    expect(askContextFor("/overview", "prod-eu")).toEqual({ cluster: "prod-eu" });
    expect(askContextFor("/", "prod-eu")).toEqual({ cluster: "prod-eu" });
  });

  it("survives a name with a slash in it, which is why the route encodes segments", () => {
    const route = logsRoute("Pod", "ns", "weird/name");
    expect(askContextFor(route, "prod-eu")).toMatchObject({ namespace: "ns", name: "weird/name" });
  });

  /**
   * The screenshot: a StatefulSets list narrowed to one namespace by the
   * picker, and "which MongoDB replica set spiked?" ran `podMetrics` across
   * ten namespaces — every `*-dataservices` in the cluster. The reader had
   * already answered "which namespace" with the picker and nothing told the
   * agent.
   */
  it("carries the reader's namespace narrowing on a list route", () => {
    expect(askContextFor("/k/statefulsets", "prod-eu", ["m01-prod-04-dataservices"])).toEqual({
      cluster: "prod-eu",
      namespaces: ["m01-prod-04-dataservices"],
    });
  });

  it("carries several, when the reader picked several", () => {
    const about = askContextFor("/k/statefulsets", "prod-eu", ["a", "b"]);
    expect(about.namespaces).toEqual(["a", "b"]);
  });

  it("says nothing about namespaces when the reader chose all of them", () => {
    // An empty selection IS "all namespaces" — a real answer, and inventing a
    // scope the reader did not set would be worse than saying nothing.
    expect(askContextFor("/k/statefulsets", "prod-eu", [])).toEqual({ cluster: "prod-eu" });
  });

  it("prefers a route's own resource over the standing selection", () => {
    // A logs route names one pod; the picker could only widen that.
    const route = logsRoute("Pod", "m01-cnips-01-services", "ai-editor");
    const about = askContextFor(route, "prod-eu", ["some-other-namespace"]);
    expect(about.namespace).toBe("m01-cnips-01-services");
    expect(about.namespaces).toBeUndefined();
  });
});