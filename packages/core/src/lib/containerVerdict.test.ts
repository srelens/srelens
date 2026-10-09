import { describe, expect, it } from "vitest";
import {
  containerVerdict,
  containersReadyText,
  containersSortValue,
  containersWord,
  describeContainer,
} from "./containerVerdict";
import type { PodContainer } from "./workloads";

const c = (over: Partial<PodContainer> = {}): PodContainer => ({
  name: "api", kind: "app", state: "running", reason: "", exitCode: null, ready: true, restarts: 0, image: "acme/api:1",
  ...over,
});

describe("containerVerdict", () => {
  it("is ready for a container that is running and ready", () => {
    expect(containerVerdict(c())).toEqual({ kind: "ready", word: "Running", restarted: false });
  });

  it("is unready for one that is running and not ready", () => {
    expect(containerVerdict(c({ ready: false }))).toEqual({
      kind: "unready", word: "Running, not ready", restarted: false,
    });
  });

  it("is starting for one waiting on its way up", () => {
    for (const reason of ["ContainerCreating", "PodInitializing", ""]) {
      expect(containerVerdict(c({ state: "waiting", ready: false, reason })).kind).toBe("starting");
    }
    expect(containerVerdict(c({ state: "waiting", reason: "ContainerCreating" })).word).toBe(
      "Waiting (ContainerCreating)",
    );
    // With no reason there is nothing to put in brackets.
    expect(containerVerdict(c({ state: "waiting", reason: "" })).word).toBe("Waiting");
  });

  it("is stuck for one that waiting will not fix", () => {
    for (const reason of [
      "CrashLoopBackOff",
      "ImagePullBackOff",
      "ErrImagePull",
      "ErrImageNeverPull",
      "InvalidImageName",
      "CreateContainerConfigError",
      "CreateContainerError",
      "RunContainerError",
    ]) {
      expect(containerVerdict(c({ state: "waiting", ready: false, reason })).kind, reason).toBe("stuck");
    }
  });

  it("is completed for one that exited with 0, and only that", () => {
    expect(containerVerdict(c({ state: "terminated", ready: false, exitCode: 0, reason: "Completed" }))).toEqual({
      kind: "completed", word: "Completed", restarted: false,
    });
  });

  it("is failed for one that exited with anything else", () => {
    expect(containerVerdict(c({ state: "terminated", ready: false, exitCode: 137, reason: "OOMKilled" }))).toEqual({
      kind: "failed", word: "Exited 137 (OOMKilled)", restarted: false,
    });
    expect(containerVerdict(c({ state: "terminated", exitCode: 1, reason: "Error" })).word).toBe("Exited 1 (Error)");
  });

  it("does not take a missing exit code for a clean exit", () => {
    for (const exitCode of [null, undefined]) {
      expect(containerVerdict(c({ state: "terminated", exitCode, reason: "" }))).toMatchObject({
        kind: "failed", word: "Exited",
      });
    }
  });

  it("is unknown for one the kubelet has not reported on", () => {
    expect(containerVerdict(c({ state: "unknown", ready: false }))).toMatchObject({
      kind: "unknown", word: "Not reported yet",
    });
  });

  /**
   * A restart is something that happened, not what the container is doing: it
   * is carried beside the state and never replaces it.
   */
  it("carries a restart beside whatever the container is doing now", () => {
    expect(containerVerdict(c({ restarts: 3 }))).toEqual({ kind: "ready", word: "Running", restarted: true });
    expect(containerVerdict(c({ state: "waiting", reason: "CrashLoopBackOff", restarts: 14 }))).toMatchObject({
      kind: "stuck", restarted: true,
    });
    expect(containerVerdict(c({ restarts: 0 })).restarted).toBe(false);
  });
});

describe("describeContainer", () => {
  it("says the name, the state, the restarts and the image", () => {
    expect(describeContainer(c({ name: "worker", state: "waiting", reason: "CrashLoopBackOff", restarts: 14, image: "acme/worker:1" }))).toBe(
      "worker: Waiting (CrashLoopBackOff), 14 restarts, acme/worker:1",
    );
  });

  it("leaves out restarts that did not happen, and says one in the singular", () => {
    expect(describeContainer(c())).toBe("api: Running, acme/api:1");
    expect(describeContainer(c({ restarts: 1 }))).toBe("api: Running, 1 restart, acme/api:1");
  });

  it("says when a container is an init container or a sidecar", () => {
    expect(describeContainer(c({ name: "migrate", kind: "init", state: "terminated", exitCode: 0, image: "" }))).toBe(
      "migrate (init): Completed",
    );
    expect(describeContainer(c({ name: "proxy", kind: "sidecar", image: "" }))).toBe("proxy (sidecar): Running");
  });
});

describe("containersSortValue", () => {
  const stuck = c({ state: "waiting", reason: "CrashLoopBackOff", ready: false });
  const failed = c({ state: "terminated", exitCode: 1, ready: false });
  const unready = c({ ready: false });
  const starting = c({ state: "waiting", reason: "ContainerCreating", ready: false });
  const done = c({ kind: "init", state: "terminated", exitCode: 0, ready: false });

  it("puts the pod with the worst container first", () => {
    const order = [[failed], [stuck], [unready], [starting], [c()], [done]].map(containersSortValue);
    expect(order).toEqual([...order].sort((a, b) => b - a));
    expect(new Set(order).size).toBe(order.length);
  });

  it("is decided by the worst container, however many healthy ones sit beside it", () => {
    expect(containersSortValue([c(), c(), c(), stuck])).toBeGreaterThan(containersSortValue([unready, unready, unready]));
  });

  it("breaks a tie by how many are in that state", () => {
    expect(containersSortValue([stuck, stuck])).toBeGreaterThan(containersSortValue([stuck, c()]));
  });

  it("does not hold a finished init container against a healthy pod", () => {
    expect(containersSortValue([c(), done])).toBe(containersSortValue([c()]));
  });

  it("sorts a finished Job the same with or without init containers", () => {
    // Every container completed: the init ones must not add to the count.
    const finished = c({ state: "terminated", exitCode: 0, ready: false });
    expect(containersSortValue([finished, done, done])).toBe(containersSortValue([finished]));
  });

  it("still counts an init container that is in trouble", () => {
    const failing = c({ kind: "init", state: "waiting", reason: "CrashLoopBackOff", ready: false });
    expect(containersSortValue([c(), failing])).toBeGreaterThan(containersSortValue([c()]));
  });

  it("is the lowest for a pod with no containers reported", () => {
    expect(containersSortValue([])).toBeLessThan(containersSortValue([c()]));
  });
});

describe("containersWord", () => {
  it("says nothing for a pod whose containers are all as they should be", () => {
    expect(containersWord([c(), c({ name: "b" })])).toBeNull();
    const done = c({ kind: "init", state: "terminated", exitCode: 0, ready: false });
    expect(containersWord([c(), done])).toBeNull();
    // A Job that ran to the end.
    expect(containersWord([c({ state: "terminated", exitCode: 0, ready: false })])).toBeNull();
    expect(containersWord([])).toBeNull();
  });

  it("gives the worst container's reason", () => {
    expect(
      containersWord([
        c(),
        c({ name: "slow", ready: false }),
        c({ name: "worker", state: "waiting", reason: "CrashLoopBackOff", ready: false }),
      ]),
    ).toBe("CrashLoopBackOff");
    expect(containersWord([c({ state: "terminated", exitCode: 137, reason: "OOMKilled" })])).toBe("OOMKilled");
  });

  it("gives the state in a word where there is no reason", () => {
    expect(containersWord([c({ ready: false })])).toBe("Not ready");
    expect(containersWord([c({ state: "waiting", reason: "", ready: false })])).toBe("Starting");
    expect(containersWord([c({ state: "terminated", exitCode: 3, reason: "" })])).toBe("Exited 3");
    expect(containersWord([c({ state: "terminated", exitCode: null, reason: "" })])).toBe("Exited");
    expect(containersWord([c({ state: "unknown", ready: false })])).toBe("Not reported");
  });

  it("speaks for a failing init container too", () => {
    expect(containersWord([c({ state: "waiting", reason: "PodInitializing", ready: false }), c({ kind: "init", state: "waiting", reason: "ImagePullBackOff", ready: false })])).toBe("ImagePullBackOff");
  });
});

describe("containersReadyText", () => {
  it("says how many are ready, as the Ready column did", () => {
    expect(containersReadyText([c(), c({ ready: false })])).toBe("1 of 2 containers ready");
    expect(containersReadyText([c()])).toBe("1 of 1 container ready");
  });

  it("does not count init containers, which are never ready once they have finished", () => {
    const init = c({ kind: "init", state: "terminated", exitCode: 0, ready: false });
    expect(containersReadyText([c(), init])).toBe("1 of 1 container ready");
  });

  it("counts a sidecar, which runs beside the app containers", () => {
    expect(containersReadyText([c(), c({ kind: "sidecar", ready: false })])).toBe("1 of 2 containers ready");
  });
});
