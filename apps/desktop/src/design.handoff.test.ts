import { describe, it, expect, beforeEach } from "vitest";
import { HANDOFF_KEY, handoffFor, saveHandoff, takeHandoff } from "./design";

describe("handoffFor", () => {
  it("carries nothing when there is no cluster to land on", () => {
    // A handoff is "reopen this place on this cluster"; without the cluster
    // classic would have to guess, and guessing wrong is worse than not going.
    expect(handoffFor("/k/pods")).toBeNull();
    expect(handoffFor("/", undefined)).toBeNull();
  });

  it("maps a built-in kind slug to that kind", () => {
    expect(handoffFor("/k/pods", "prod")).toEqual({ context: "prod", kind: "pods" });
    expect(handoffFor("/k/configmaps", "prod")).toEqual({ context: "prod", kind: "configmaps" });
  });

  it("treats the overview slug as a plain overview", () => {
    expect(handoffFor("/k/overview", "prod")).toEqual({ context: "prod", kind: "overview" });
  });

  it("falls back to an overview for a slug it does not know", () => {
    expect(handoffFor("/k/nonsense", "prod")).toEqual({ context: "prod", kind: "overview" });
  });

  it("knows the app-scoped routes", () => {
    expect(handoffFor("/events", "prod")).toEqual({ context: "prod", kind: "events" });
    expect(handoffFor("/forwards", "prod")).toEqual({ context: "prod", kind: "portforwards" });
    expect(handoffFor("/helm", "prod")).toEqual({ context: "prod", kind: "helmreleases" });
  });

  it("lands home and overview routes on the overview", () => {
    expect(handoffFor("/", "prod")).toEqual({ context: "prod", kind: "overview" });
    expect(handoffFor("/overview", "prod")).toEqual({ context: "prod", kind: "overview" });
  });

  it("lands anywhere else on the overview rather than dropping the cluster", () => {
    // R-F: routes with no classic kind still hand off the cluster — classic's
    // overview is the nearest place to stand while looking at it.
    expect(handoffFor("/applog", "prod")).toEqual({ context: "prod", kind: "overview" });
    expect(handoffFor("/settings", "prod")).toEqual({ context: "prod", kind: "overview" });
  });
});

describe("saveHandoff and takeHandoff", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it("round trips, and taking clears the key", () => {
    saveHandoff("/k/pods", "prod");
    expect(takeHandoff()).toEqual({ context: "prod", kind: "pods" });
    // A handoff is for the one reload that follows; leaving it behind would
    // reopen the same view on every future launch of classic.
    expect(takeHandoff()).toBeNull();
    expect(sessionStorage.getItem(HANDOFF_KEY)).toBeNull();
  });

  it("writes nothing when there is no context to hand off", () => {
    saveHandoff("/");
    expect(sessionStorage.getItem(HANDOFF_KEY)).toBeNull();
  });

  it("reads malformed JSON as nothing", () => {
    sessionStorage.setItem(HANDOFF_KEY, "{not json");
    expect(takeHandoff()).toBeNull();
    expect(sessionStorage.getItem(HANDOFF_KEY)).toBeNull();
  });
});
