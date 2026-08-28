import { describe, it, expect } from "vitest";
import {
  containerLastRestartTime,
  latestRestartTime,
  portText,
  probeChips,
  resourceText,
  envText,
  mountText,
  podContainerStatuses,
  resourceSummary,
  restartTotal,
  tolerationText,
} from "./k8sContainer";

// Moved verbatim from apps/desktop/src/components/ResourceOverview.test.tsx
// (only the import path changed).
describe("containerLastRestartTime", () => {
  it("uses the previous termination time only for restarted containers", () => {
    expect(
      containerLastRestartTime({
        restartCount: 2,
        lastState: { terminated: { finishedAt: "2025-12-31T23:55:00Z" } },
      }),
    ).toBe("2025-12-31T23:55:00Z");
    expect(containerLastRestartTime({ restartCount: 0, lastState: {} })).toBe("");
  });
});

describe("latestRestartTime", () => {
  it("returns '' for an empty list", () => {
    expect(latestRestartTime([])).toBe("");
  });

  it("returns '' when no container has restarted", () => {
    const statuses = [
      { restartCount: 0, lastState: {} },
      { restartCount: 0, lastState: {} },
    ];
    expect(latestRestartTime(statuses)).toBe("");
  });

  it("picks the most recent restart among several, regardless of array order", () => {
    const statuses = [
      { restartCount: 1, lastState: { terminated: { finishedAt: "2026-01-01T00:00:00Z" } } },
      { restartCount: 3, lastState: { terminated: { finishedAt: "2026-01-01T02:00:00Z" } } },
      { restartCount: 2, lastState: { terminated: { finishedAt: "2026-01-01T01:00:00Z" } } },
    ];
    expect(latestRestartTime(statuses)).toBe("2026-01-01T02:00:00Z");
  });

  it("ignores non-restarted containers mixed in with a restarted one", () => {
    const statuses = [
      { restartCount: 0, lastState: {} },
      { restartCount: 1, lastState: { terminated: { finishedAt: "2026-01-01T00:00:00Z" } } },
    ];
    expect(latestRestartTime(statuses)).toBe("2026-01-01T00:00:00Z");
  });
});

describe("portText", () => {
  it("prefixes the name when present", () => {
    expect(portText({ name: "http", containerPort: 8080, protocol: "TCP" })).toBe("http: 8080/TCP");
  });

  it("omits the name prefix when absent", () => {
    expect(portText({ containerPort: 8080, protocol: "TCP" })).toBe("8080/TCP");
  });

  it("uses an explicit protocol", () => {
    expect(portText({ containerPort: 53, protocol: "UDP" })).toBe("53/UDP");
  });

  it("defaults the protocol to TCP when absent", () => {
    expect(portText({ containerPort: 8080 })).toBe("8080/TCP");
  });
});

describe("probeChips", () => {
  it("formats an httpGet probe, defaulting scheme to http (lowercased)", () => {
    expect(probeChips({ httpGet: { port: 8080, path: "/healthz" } })).toEqual([
      "http-get http://:8080/healthz",
    ]);
  });

  it("formats an httpGet probe with an explicit scheme, lowercased", () => {
    expect(probeChips({ httpGet: { scheme: "HTTPS", port: 443, path: "/" } })).toEqual([
      "http-get https://:443/",
    ]);
  });

  it("formats a tcpSocket probe", () => {
    expect(probeChips({ tcpSocket: { port: 5432 } })).toEqual(["tcp-socket :5432"]);
  });

  it("formats an exec probe by joining its command", () => {
    expect(probeChips({ exec: { command: ["cat", "/tmp/healthy"] } })).toEqual([
      "exec [cat /tmp/healthy]",
    ]);
  });

  it("prefers httpGet over tcpSocket when both are present", () => {
    expect(probeChips({ httpGet: { port: 80 }, tcpSocket: { port: 81 } })).toEqual([
      "http-get http://:80",
    ]);
  });

  it("prefers tcpSocket over exec when both are present", () => {
    expect(probeChips({ tcpSocket: { port: 81 }, exec: { command: ["true"] } })).toEqual([
      "tcp-socket :81",
    ]);
  });

  it("emits no probe-type chip when none of httpGet/tcpSocket/exec is set", () => {
    expect(probeChips({ initialDelaySeconds: 5 })).toEqual(["delay=5s"]);
  });

  it("appends timing and threshold chips, including zero values", () => {
    expect(
      probeChips({
        tcpSocket: { port: 5432 },
        initialDelaySeconds: 0,
        timeoutSeconds: 1,
        periodSeconds: 10,
        successThreshold: 1,
        failureThreshold: 3,
      }),
    ).toEqual([
      "tcp-socket :5432",
      "delay=0s",
      "timeout=1s",
      "period=10s",
      "#success=1",
      "#failure=3",
    ]);
  });
});

describe("resourceText", () => {
  it("shows both cpu and memory when present", () => {
    expect(resourceText({ cpu: "500m", memory: "256Mi" })).toBe("CPU: 500m, Memory: 256Mi");
  });

  it("falls back to — for a missing cpu", () => {
    expect(resourceText({ memory: "256Mi" })).toBe("CPU: —, Memory: 256Mi");
  });

  it("falls back to — for a missing memory", () => {
    expect(resourceText({ cpu: "500m" })).toBe("CPU: 500m, Memory: —");
  });

  it("falls back to — for both when neither is set", () => {
    expect(resourceText({})).toBe("CPU: —, Memory: —");
  });
});

describe("envText", () => {
  it("formats a literal value", () => {
    expect(envText({ name: "FOO", value: "bar" })).toBe("FOO=bar");
  });

  it("treats an empty-string value as set (not falling through to valueFrom)", () => {
    expect(envText({ name: "FOO", value: "", valueFrom: { secretKeyRef: {} } })).toBe("FOO=");
  });

  it("marks a secretKeyRef source", () => {
    expect(envText({ name: "FOO", valueFrom: { secretKeyRef: { name: "s", key: "k" } } })).toBe(
      "FOO=<secret>",
    );
  });

  it("marks a configMapKeyRef source", () => {
    expect(
      envText({ name: "FOO", valueFrom: { configMapKeyRef: { name: "cm", key: "k" } } }),
    ).toBe("FOO=<configMap>");
  });

  it("marks a fieldRef source", () => {
    expect(envText({ name: "FOO", valueFrom: { fieldRef: { fieldPath: "status.podIP" } } })).toBe(
      "FOO=<field>",
    );
  });

  it("marks a resourceFieldRef source", () => {
    expect(
      envText({ name: "FOO", valueFrom: { resourceFieldRef: { resource: "limits.cpu" } } }),
    ).toBe("FOO=<resource>");
  });

  it("falls back to a generic ref when valueFrom has none of the known keys", () => {
    expect(envText({ name: "FOO", valueFrom: {} })).toBe("FOO=<ref>");
    expect(envText({ name: "FOO" })).toBe("FOO=<ref>");
  });

  it("prefers secretKeyRef over configMapKeyRef when both are present", () => {
    expect(
      envText({
        name: "FOO",
        valueFrom: { secretKeyRef: { name: "s" }, configMapKeyRef: { name: "cm" } },
      }),
    ).toBe("FOO=<secret>");
  });

  it("prefers configMapKeyRef over fieldRef when both are present", () => {
    expect(
      envText({
        name: "FOO",
        valueFrom: { configMapKeyRef: { name: "cm" }, fieldRef: { fieldPath: "spec.nodeName" } },
      }),
    ).toBe("FOO=<configMap>");
  });

  it("prefers fieldRef over resourceFieldRef when both are present", () => {
    expect(
      envText({
        name: "FOO",
        valueFrom: { fieldRef: { fieldPath: "spec.nodeName" }, resourceFieldRef: { resource: "limits.cpu" } },
      }),
    ).toBe("FOO=<field>");
  });
});

describe("mountText", () => {
  it("marks a read-only mount", () => {
    expect(mountText({ mountPath: "/data", name: "vol", readOnly: true })).toBe(
      "/data (ro) ← vol",
    );
  });

  it("omits the (ro) marker when not read-only", () => {
    expect(mountText({ mountPath: "/data", name: "vol", readOnly: false })).toBe("/data ← vol");
  });

  it("omits the (ro) marker when readOnly is absent", () => {
    expect(mountText({ mountPath: "/data", name: "vol" })).toBe("/data ← vol");
  });
});

describe("tolerationText", () => {
  it("formats a full toleration", () => {
    expect(
      tolerationText({
        key: "dedicated",
        operator: "Equal",
        value: "gpu",
        effect: "NoSchedule",
        tolerationSeconds: 30,
      }),
    ).toBe("dedicated=gpu → NoSchedule for 30s");
  });

  it("falls back to '(any taint)' when key is absent", () => {
    expect(tolerationText({ operator: "Exists" })).toBe("(any taint) exists → all effects");
  });

  it("defaults operator to Equal (key=value form) when absent", () => {
    expect(tolerationText({ key: "dedicated", value: "gpu" })).toBe("dedicated=gpu → all effects");
  });

  it("uses the 'key exists' form for operator Exists, dropping value", () => {
    expect(tolerationText({ key: "dedicated", operator: "Exists" })).toBe(
      "dedicated exists → all effects",
    );
  });

  it("falls back to 'all effects' when effect is absent", () => {
    expect(tolerationText({ key: "dedicated", value: "gpu" })).toBe("dedicated=gpu → all effects");
  });

  it("omits the duration suffix when tolerationSeconds is absent", () => {
    expect(tolerationText({ key: "dedicated", value: "gpu", effect: "NoSchedule" })).toBe(
      "dedicated=gpu → NoSchedule",
    );
  });
});

/**
 * The pod-wide reads a header, a fact list and a metric tile all want, in one
 * place. `PodBody` derived both of these inline and the full tab needed the
 * same two numbers — a second copy of a reduce is how two surfaces start
 * disagreeing about how many times a pod has restarted.
 */
describe("podContainerStatuses", () => {
  it("reads init, app and ephemeral statuses in that order", () => {
    expect(
      podContainerStatuses({
        initContainerStatuses: [{ name: "migrate" }],
        containerStatuses: [{ name: "app" }],
        ephemeralContainerStatuses: [{ name: "debugger" }],
      }).map((s) => s.name),
    ).toEqual(["migrate", "app", "debugger"]);
  });

  it("answers an empty list for a pod the kubelet has not reported on", () => {
    expect(podContainerStatuses({})).toEqual([]);
    expect(podContainerStatuses(undefined)).toEqual([]);
  });
});

describe("restartTotal", () => {
  it("adds up every container's restarts", () => {
    expect(restartTotal([{ restartCount: 3 }, { restartCount: 2 }])).toBe(5);
  });

  it("reads a missing count as none, not as a gap", () => {
    expect(restartTotal([{ name: "app" }])).toBe(0);
  });

  it("ignores a count that is not a number rather than answering NaN", () => {
    // A NaN would render as "NaN" in a metric tile, which reads as a fault in
    // srelens rather than in the payload.
    expect(restartTotal([{ restartCount: "many" }, { restartCount: 2 }])).toBe(2);
  });
});

describe("resourceSummary", () => {
  it("reads cpu and memory as one compact value", () => {
    expect(resourceSummary({ cpu: "250m", memory: "512Mi" })).toBe("250m · 512Mi");
  });

  it("stands an em dash in for a half that was never set", () => {
    expect(resourceSummary({ memory: "512Mi" })).toBe("— · 512Mi");
    expect(resourceSummary({})).toBe("— · —");
  });
});
