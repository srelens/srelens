import { describe, it, expect, vi } from "vitest";
import {
  listPersistentVolumeClaims,
  listPersistentVolumes,
  listStorageClasses,
  podsForPvc,
  formatStorageSize,
} from "./storage";

describe("formatStorageSize", () => {
  it("humanizes a raw byte quantity to the nearest binary unit", () => {
    expect(formatStorageSize("7586630231655")).toBe("6.9Ti");
    expect(formatStorageSize("1073741824")).toBe("1Gi");
    expect(formatStorageSize("536870912")).toBe("512Mi");
  });

  it("normalizes an already-suffixed quantity", () => {
    expect(formatStorageSize("10Gi")).toBe("10Gi");
    expect(formatStorageSize("500Mi")).toBe("500Mi");
    expect(formatStorageSize("5G")).toBe("4.7Gi");
  });

  it("shows raw bytes below 1Ki and a dash for empty/invalid", () => {
    expect(formatStorageSize("512")).toBe("512");
    expect(formatStorageSize("")).toBe("—");
    expect(formatStorageSize("nonsense")).toBe("nonsense");
  });
});

describe("listPersistentVolumeClaims", () => {
  it("calls k8s.listPersistentVolumeClaims and returns typed rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      persistentvolumeclaims: [
        { name: "data", namespace: "default", status: "Bound", capacity: "10Gi", accessModes: "RWO", storageClass: "standard", volume: "pv-123", age: "3d" },
      ],
    });
    const out = await listPersistentVolumeClaims("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listPersistentVolumeClaims", { context: "kind-dev", namespace: "default" });
    expect(out.persistentvolumeclaims?.[0].volume).toBe("pv-123");
  });

  it("returns an error string when the call rejects", async () => {
    const out = await listPersistentVolumeClaims("x", "default", () => Promise.reject(new Error("boom")));
    expect(out.error).toContain("boom");
  });
});

describe("listPersistentVolumes", () => {
  it("calls k8s.listPersistentVolumes (cluster-scoped, no namespace)", async () => {
    const invoke = vi.fn().mockResolvedValue({
      persistentvolumes: [
        { name: "pv-123", capacity: "20Gi", accessModes: "RWO", reclaimPolicy: "Retain", status: "Bound", claim: "default/data", storageClass: "standard", age: "5d" },
      ],
    });
    const out = await listPersistentVolumes("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listPersistentVolumes", { context: "kind-dev" });
    expect(out.persistentvolumes?.[0].claim).toBe("default/data");
  });
});

describe("podsForPvc", () => {
  it("calls k8s.podsForPvc with the claim name", async () => {
    const invoke = vi.fn().mockResolvedValue({ pods: [{ name: "web-1", namespace: "default" }] });
    const out = await podsForPvc("kind-dev", "default", "data", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podsForPvc", { context: "kind-dev", namespace: "default", pvc: "data" });
    expect(out.pods?.[0].name).toBe("web-1");
  });
});

describe("listStorageClasses", () => {
  it("calls k8s.listStorageClasses and flags the default", async () => {
    const invoke = vi.fn().mockResolvedValue({
      storageclasses: [
        { name: "standard", provisioner: "kubernetes.io/aws-ebs", reclaimPolicy: "Delete", volumeBindingMode: "WaitForFirstConsumer", default: true, age: "9d" },
      ],
    });
    const out = await listStorageClasses("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listStorageClasses", { context: "kind-dev" });
    expect(out.storageclasses?.[0].default).toBe(true);
  });
});
