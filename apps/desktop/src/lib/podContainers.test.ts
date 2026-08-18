import { describe, expect, it } from "vitest";
import { defaultContainer, execCandidates, podContainerChoices } from "./podContainers";

const running = (name: string) => ({ name, state: { running: { startedAt: "now" } } });
const terminated = (name: string) => ({ name, state: { terminated: { exitCode: 0 } } });
const waiting = (name: string) => ({ name, state: { waiting: { reason: "CrashLoopBackOff" } } });

describe("podContainerChoices", () => {
  it("lists app containers with their running state", () => {
    const pod = {
      spec: { containers: [{ name: "app" }, { name: "sidecar" }] },
      status: { containerStatuses: [running("app"), waiting("sidecar")] },
    };
    expect(podContainerChoices(pod)).toEqual([
      { name: "app", kind: "app", running: true },
      { name: "sidecar", kind: "app", running: false },
    ]);
  });

  it("keeps a crash-looping container in the list", () => {
    // The one you most want a shell into is often the broken one; the caller
    // shows why it can't attach rather than pretending it isn't there.
    const pod = {
      spec: { containers: [{ name: "broken" }] },
      status: { containerStatuses: [waiting("broken")] },
    };
    expect(podContainerChoices(pod)).toEqual([{ name: "broken", kind: "app", running: false }]);
  });

  it("lists a finished init container, marked as not running", () => {
    // Shown rather than hidden: a menu that omits half the pod is harder to
    // trust than one that says which parts are up.
    const pod = {
      spec: { containers: [{ name: "app" }], initContainers: [{ name: "migrate" }] },
      status: { containerStatuses: [running("app")], initContainerStatuses: [terminated("migrate")] },
    };
    expect(podContainerChoices(pod)).toEqual([
      { name: "app", kind: "app", running: true },
      { name: "migrate", kind: "init", running: false },
    ]);
  });

  it("offers a native sidecar, which is an init container that stays up", () => {
    const pod = {
      spec: {
        containers: [{ name: "app" }],
        initContainers: [{ name: "proxy", restartPolicy: "Always" }],
      },
      status: { containerStatuses: [running("app")], initContainerStatuses: [running("proxy")] },
    };
    expect(podContainerChoices(pod)).toContainEqual({ name: "proxy", kind: "init", running: true });
  });

  it("offers ephemeral debug containers", () => {
    const pod = {
      spec: { containers: [{ name: "app" }], ephemeralContainers: [{ name: "debugger" }] },
      status: {
        containerStatuses: [running("app")],
        ephemeralContainerStatuses: [running("debugger")],
      },
    };
    expect(podContainerChoices(pod)).toContainEqual({
      name: "debugger",
      kind: "ephemeral",
      running: true,
    });
  });

  it("returns nothing for a pod that hasn't loaded, rather than throwing", () => {
    expect(podContainerChoices(undefined)).toEqual([]);
    expect(podContainerChoices({})).toEqual([]);
    expect(podContainerChoices({ spec: { containers: "not-a-list" } })).toEqual([]);
  });
});

describe("execCandidates", () => {
  it("ignores a finished init container, so a plain app pod asks nothing", () => {
    const pod = {
      spec: { containers: [{ name: "app" }], initContainers: [{ name: "migrate" }] },
      status: { containerStatuses: [running("app")], initContainerStatuses: [terminated("migrate")] },
    };
    expect(execCandidates(podContainerChoices(pod)).map((c) => c.name)).toEqual(["app"]);
  });

  it("counts a sidecar that is still up", () => {
    const pod = {
      spec: { containers: [{ name: "app" }], initContainers: [{ name: "proxy" }] },
      status: { containerStatuses: [running("app")], initContainerStatuses: [running("proxy")] },
    };
    expect(execCandidates(podContainerChoices(pod)).map((c) => c.name)).toEqual(["app", "proxy"]);
  });

  it("keeps an app container that isn't running", () => {
    // Otherwise a pod whose only container is crash-looping would look like it
    // has nowhere to exec, when trying is exactly what the user wants.
    const pod = {
      spec: { containers: [{ name: "app" }] },
      status: { containerStatuses: [waiting("app")] },
    };
    expect(execCandidates(podContainerChoices(pod)).map((c) => c.name)).toEqual(["app"]);
  });
});

describe("defaultContainer", () => {
  it("honours the annotation kubectl uses", () => {
    // The API server ignores this annotation, so without resolving it here the
    // shell lands on whichever container the manifest happens to list first.
    const pod = {
      metadata: { annotations: { "kubectl.kubernetes.io/default-container": "app" } },
      spec: { containers: [{ name: "istio-proxy" }, { name: "app" }] },
      status: { containerStatuses: [running("istio-proxy"), running("app")] },
    };
    expect(defaultContainer(pod, podContainerChoices(pod))).toBe("app");
  });

  it("ignores an annotation naming a container that isn't there", () => {
    const pod = {
      metadata: { annotations: { "kubectl.kubernetes.io/default-container": "removed" } },
      spec: { containers: [{ name: "app" }] },
      status: { containerStatuses: [running("app")] },
    };
    expect(defaultContainer(pod, podContainerChoices(pod))).toBe("app");
  });

  it("skips a leading container that isn't running", () => {
    const pod = {
      spec: { containers: [{ name: "crashed" }, { name: "app" }] },
      status: { containerStatuses: [waiting("crashed"), running("app")] },
    };
    expect(defaultContainer(pod, podContainerChoices(pod))).toBe("app");
  });

  it("falls back to the first container when none is running", () => {
    const pod = {
      spec: { containers: [{ name: "a" }, { name: "b" }] },
      status: { containerStatuses: [waiting("a"), waiting("b")] },
    };
    expect(defaultContainer(pod, podContainerChoices(pod))).toBe("a");
  });

  it("never prefers an ephemeral container over an app container", () => {
    // Debug containers accumulate on a pod and can't be removed; picking the
    // newest one by default would quietly hijack every later shell.
    const pod = {
      spec: { containers: [{ name: "app" }], ephemeralContainers: [{ name: "debugger" }] },
      status: {
        containerStatuses: [waiting("app")],
        ephemeralContainerStatuses: [running("debugger")],
      },
    };
    expect(defaultContainer(pod, podContainerChoices(pod))).toBe("app");
  });

  it("is undefined when there is nothing to pick", () => {
    expect(defaultContainer({}, [])).toBeUndefined();
  });
});
