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
   * The rule, as stated: "if open in a tab where namespace is selected then
   * that, pass kind type like which tab is opened etc, when a new one is
   * initialised from agents tab then only cluster context should be passed."
   *
   * The screenshot behind it: a question asked with a chip reading
   * `m01-prod-04-dataservices` that never changed, because the narrowing
   * belongs to a cluster and outlives every conversation.
   */
  it("carries the tab's kind and the namespace it is narrowed to", () => {
    expect(askContextFor("/k/statefulsets", "prod-eu", ["m01-prod-04-dataservices"])).toEqual({
      cluster: "prod-eu",
      kind: "StatefulSet",
      namespaces: ["m01-prod-04-dataservices"],
    });
  });

  it("carries several, when the reader picked several", () => {
    expect(askContextFor("/k/pods", "prod-eu", ["a", "b"]).namespaces).toEqual(["a", "b"]);
  });

  it("says nothing about namespaces when the reader chose all of them", () => {
    // An empty selection IS "all namespaces" — a real answer, and inventing a
    // scope the reader did not set would be worse than saying nothing. The
    // kind still travels: which list is open is not a guess.
    expect(askContextFor("/k/statefulsets", "prod-eu", [])).toEqual({
      cluster: "prod-eu",
      kind: "StatefulSet",
    });
  });

  it("names no kind for a route that is not one of core's lists", () => {
    // `/helm` and the control room show no Kubernetes kind. Naming one srelens
    // cannot resolve would be worse than naming none.
    expect(askContextFor("/helm", "prod-eu", [])).toEqual({ cluster: "prod-eu" });
    expect(askContextFor("/", "prod-eu", [])).toEqual({ cluster: "prod-eu" });
  });

  /**
   * The agent tab is the FULL VIEW of whichever conversation is selected, not a
   * subject of its own — so a new conversation started there has no list behind
   * it and no kind on screen. Anything more would be scope borrowed from a tab
   * the reader is not on.
   */
  it("passes the cluster alone for a conversation started on the agent tab", () => {
    expect(askContextFor("/agent", "prod-eu", ["m01-prod-04-dataservices"])).toEqual({
      cluster: "prod-eu",
    });
  });

  it("prefers a route's own resource over the standing selection", () => {
    // A logs route names one pod; the picker could only widen that.
    const route = logsRoute("Pod", "m01-cnips-01-services", "ai-editor");
    const about = askContextFor(route, "prod-eu", ["some-other-namespace"]);
    expect(about.namespace).toBe("m01-cnips-01-services");
    expect(about.namespaces).toBeUndefined();
  });

  it("still takes a namespace the ROUTE names", () => {
    // The route is a resource's identity here, and that has not changed — what
    // went is the standing filter, not the subject on screen.
    const route = logsRoute("Pod", "m01-cnips-01-services", "ai-editor");
    expect(askContextFor(route, "prod-eu").namespace).toBe("m01-cnips-01-services");
  });
});
