import { describe, it, expect } from "vitest";
import { toKubectl } from "./kubectlMapper";

describe("kubectlMapper", () => {
  describe("read-only commands", () => {
    it("get resource with namespace and context", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "default", name: "web-0", context: "prod" }),
      ).toBe("kubectl get pod web-0 -n default --context prod");
    });

    it("describe resource with namespace and context", () => {
      expect(
        toKubectl({ action: "describe", kind: "Deployment", namespace: "apps", name: "api", context: "staging" }),
      ).toBe("kubectl describe deployment api -n apps --context staging");
    });

    it("get resource as yaml", () => {
      expect(
        toKubectl({ action: "get", kind: "Service", namespace: "kube-system", name: "dns", context: "dev", output: "yaml" }),
      ).toBe("kubectl get service dns -n kube-system --context dev -o yaml");
    });

    it("omits namespace for cluster-scoped resources", () => {
      expect(
        toKubectl({ action: "get", kind: "Node", name: "node-1", context: "prod" }),
      ).toBe("kubectl get node node-1 --context prod");
    });

    it("omits namespace when namespace is empty string", () => {
      expect(
        toKubectl({ action: "get", kind: "Node", namespace: "", name: "node-1", context: "prod" }),
      ).toBe("kubectl get node node-1 --context prod");
    });
  });

  describe("delete", () => {
    it("delete a namespaced resource", () => {
      expect(
        toKubectl({ action: "delete", kind: "Pod", namespace: "default", name: "web-0", context: "prod" }),
      ).toBe("kubectl delete pod web-0 -n default --context prod");
    });

    it("delete a cluster-scoped resource", () => {
      expect(
        toKubectl({ action: "delete", kind: "PersistentVolume", name: "pv-01", context: "prod" }),
      ).toBe("kubectl delete persistentvolume pv-01 --context prod");
    });
  });

  describe("scale", () => {
    it("scale a deployment", () => {
      expect(
        toKubectl({ action: "scale", kind: "Deployment", namespace: "apps", name: "api", context: "prod", replicas: 5 }),
      ).toBe("kubectl scale deployment/api --replicas=5 -n apps --context prod");
    });

    it("scale a statefulset", () => {
      expect(
        toKubectl({ action: "scale", kind: "StatefulSet", namespace: "db", name: "postgres", context: "prod", replicas: 3 }),
      ).toBe("kubectl scale statefulset/postgres --replicas=3 -n db --context prod");
    });
  });

  describe("rollout restart", () => {
    it("restart a deployment", () => {
      expect(
        toKubectl({ action: "rollout-restart", kind: "Deployment", namespace: "apps", name: "api", context: "prod" }),
      ).toBe("kubectl rollout restart deployment/api -n apps --context prod");
    });

    it("restart a daemonset", () => {
      expect(
        toKubectl({ action: "rollout-restart", kind: "DaemonSet", namespace: "kube-system", name: "fluentd", context: "prod" }),
      ).toBe("kubectl rollout restart daemonset/fluentd -n kube-system --context prod");
    });
  });

  describe("node operations", () => {
    it("cordon a node", () => {
      expect(
        toKubectl({ action: "cordon", kind: "Node", name: "node-1", context: "prod" }),
      ).toBe("kubectl cordon node-1 --context prod");
    });

    it("uncordon a node", () => {
      expect(
        toKubectl({ action: "uncordon", kind: "Node", name: "node-1", context: "prod" }),
      ).toBe("kubectl uncordon node-1 --context prod");
    });

    it("drain a node", () => {
      expect(
        toKubectl({ action: "drain", kind: "Node", name: "node-1", context: "prod" }),
      ).toBe("kubectl drain node-1 --ignore-daemonsets --delete-emptydir-data --context prod");
    });
  });

  describe("pod operations", () => {
    it("evict a pod (delete with grace period)", () => {
      expect(
        toKubectl({ action: "evict", kind: "Pod", namespace: "default", name: "web-0", context: "prod" }),
      ).toBe("kubectl delete pod web-0 --grace-period=0 -n default --context prod");
    });
  });

  describe("cronjob operations", () => {
    it("suspend a cronjob", () => {
      expect(
        toKubectl({ action: "cronjob-suspend", kind: "CronJob", namespace: "ops", name: "nightly", context: "prod" }),
      ).toBe("kubectl patch cronjob nightly -p '{\"spec\":{\"suspend\":true}}' -n ops --context prod");
    });

    it("resume a cronjob", () => {
      expect(
        toKubectl({ action: "cronjob-resume", kind: "CronJob", namespace: "ops", name: "nightly", context: "prod" }),
      ).toBe("kubectl patch cronjob nightly -p '{\"spec\":{\"suspend\":false}}' -n ops --context prod");
    });

    it("trigger a cronjob", () => {
      expect(
        toKubectl({ action: "cronjob-trigger", kind: "CronJob", namespace: "ops", name: "nightly", context: "prod" }),
      ).toBe("kubectl create job --from=cronjob/nightly nightly-manual -n ops --context prod");
    });
  });

  describe("kind normalisation", () => {
    it("lowercases kind for kubectl", () => {
      expect(
        toKubectl({ action: "get", kind: "ConfigMap", namespace: "default", name: "config", context: "dev" }),
      ).toBe("kubectl get configmap config -n default --context dev");
    });

    it("handles multi-word kinds like PersistentVolumeClaim", () => {
      expect(
        toKubectl({ action: "get", kind: "PersistentVolumeClaim", namespace: "default", name: "data", context: "dev" }),
      ).toBe("kubectl get persistentvolumeclaim data -n default --context dev");
    });
  });
});
