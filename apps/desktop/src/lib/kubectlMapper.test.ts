import { describe, it, expect } from "vitest";
import { toKubectl } from "./kubectlMapper";

describe("kubectlMapper", () => {
  describe("read-only commands", () => {
    it("get resource with namespace and context", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "default", name: "web-0", context: "prod" }),
      ).toBe("kubectl get pods web-0 -n default --context prod");
    });

    it("describe resource with namespace and context", () => {
      expect(
        toKubectl({ action: "describe", kind: "Deployment", namespace: "apps", name: "api", context: "staging" }),
      ).toBe("kubectl describe deployments api -n apps --context staging");
    });

    it("get resource as yaml", () => {
      expect(
        toKubectl({ action: "get", kind: "Service", namespace: "kube-system", name: "dns", context: "dev", output: "yaml" }),
      ).toBe("kubectl get services dns -n kube-system --context dev -o yaml");
    });

    it("omits namespace for cluster-scoped resources", () => {
      expect(
        toKubectl({ action: "get", kind: "Node", name: "node-1", context: "prod" }),
      ).toBe("kubectl get nodes node-1 --context prod");
    });

    it("omits namespace when namespace is empty string", () => {
      expect(
        toKubectl({ action: "get", kind: "Node", namespace: "", name: "node-1", context: "prod" }),
      ).toBe("kubectl get nodes node-1 --context prod");
    });
  });

  describe("delete", () => {
    it("delete a namespaced resource", () => {
      expect(
        toKubectl({ action: "delete", kind: "Pod", namespace: "default", name: "web-0", context: "prod" }),
      ).toBe("kubectl delete pods web-0 -n default --context prod");
    });

    it("delete a cluster-scoped resource not in the kind table (falls back to lowercased kind)", () => {
      expect(
        toKubectl({ action: "delete", kind: "PersistentVolume", name: "pv-01", context: "prod" }),
      ).toBe("kubectl delete persistentvolume pv-01 --context prod");
    });
  });

  describe("scale", () => {
    it("scale a deployment", () => {
      expect(
        toKubectl({ action: "scale", kind: "Deployment", namespace: "apps", name: "api", context: "prod", replicas: 5 }),
      ).toBe("kubectl scale deployments/api --replicas=5 -n apps --context prod");
    });

    it("scale a statefulset", () => {
      expect(
        toKubectl({ action: "scale", kind: "StatefulSet", namespace: "db", name: "postgres", context: "prod", replicas: 3 }),
      ).toBe("kubectl scale statefulsets/postgres --replicas=3 -n db --context prod");
    });
  });

  describe("rollout restart", () => {
    it("restart a deployment", () => {
      expect(
        toKubectl({ action: "rollout-restart", kind: "Deployment", namespace: "apps", name: "api", context: "prod" }),
      ).toBe("kubectl rollout restart deployments/api -n apps --context prod");
    });

    it("restart a daemonset", () => {
      expect(
        toKubectl({ action: "rollout-restart", kind: "DaemonSet", namespace: "kube-system", name: "fluentd", context: "prod" }),
      ).toBe("kubectl rollout restart daemonsets/fluentd -n kube-system --context prod");
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

    it("drain a node, forcing eviction of unmanaged bare pods to match backend behaviour", () => {
      expect(
        toKubectl({ action: "drain", kind: "Node", name: "node-1", context: "prod" }),
      ).toBe("kubectl drain node-1 --ignore-daemonsets --delete-emptydir-data --force --context prod");
    });
  });

  describe("cronjob operations", () => {
    it("suspend a cronjob, quoted so the patch body works in cmd.exe too", () => {
      expect(
        toKubectl({ action: "cronjob-suspend", kind: "CronJob", namespace: "ops", name: "nightly", context: "prod" }),
      ).toBe('kubectl patch cronjob nightly -p "{\\"spec\\":{\\"suspend\\":true}}" -n ops --context prod');
    });

    it("resume a cronjob, quoted so the patch body works in cmd.exe too", () => {
      expect(
        toKubectl({ action: "cronjob-resume", kind: "CronJob", namespace: "ops", name: "nightly", context: "prod" }),
      ).toBe('kubectl patch cronjob nightly -p "{\\"spec\\":{\\"suspend\\":false}}" -n ops --context prod');
    });

    it("trigger a cronjob with a timestamp suffix so a copy-pasted re-run doesn't collide", () => {
      expect(
        toKubectl({ action: "cronjob-trigger", kind: "CronJob", namespace: "ops", name: "nightly", context: "prod" }),
      ).toBe("kubectl create job --from=cronjob/nightly nightly-manual-$(date +%s) -n ops --context prod");
    });
  });

  describe("kind → resource mapping", () => {
    it("uses the plural resource name from the shared kindToResource table when the kind is known", () => {
      expect(
        toKubectl({ action: "get", kind: "ConfigMap", namespace: "default", name: "config", context: "dev" }),
      ).toBe("kubectl get configmaps config -n default --context dev");
    });

    it("uses the table for multi-word kinds like PersistentVolumeClaim too", () => {
      expect(
        toKubectl({ action: "get", kind: "PersistentVolumeClaim", namespace: "default", name: "data", context: "dev" }),
      ).toBe("kubectl get persistentvolumeclaims data -n default --context dev");
    });

    it("falls back to lowercasing the kind for CRDs/unknown kinds not in the table", () => {
      expect(
        toKubectl({ action: "get", kind: "VirtualService", namespace: "default", name: "vs-1", context: "dev" }),
      ).toBe("kubectl get virtualservice vs-1 -n default --context dev");
    });
  });

  describe("shell quoting", () => {
    it("double-quotes a context name containing spaces (single quotes aren't a quote character in cmd.exe)", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "default", name: "web-0", context: "my cluster" }),
      ).toBe('kubectl get pods web-0 -n default --context "my cluster"');
    });

    it("leaves simple context names unquoted", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "default", name: "web-0", context: "kind-dev" }),
      ).toBe("kubectl get pods web-0 -n default --context kind-dev");
    });

    it("also quotes a namespace containing spaces, not just context", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "my ns", name: "web-0", context: "prod" }),
      ).toBe('kubectl get pods web-0 -n "my ns" --context prod');
    });

    it("single-quotes a value with an embedded double quote (double-quoting can't hold it safely)", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "default", name: "web-0", context: 'my "prod" cluster' }),
      ).toBe("kubectl get pods web-0 -n default --context 'my \"prod\" cluster'");
    });

    it("single-quotes a context carrying command substitution so pasting can't execute it", () => {
      // Inside double quotes, $( ) still EXECUTES in bash/zsh/PowerShell —
      // only single quotes make it inert. Context names are user-chosen and
      // not DNS-restricted, so this input is reachable.
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "$(touch /tmp/pwn)" }),
      ).toBe("kubectl get pods web-0 --context '$(touch /tmp/pwn)'");
    });

    it("single-quotes backtick substitution the same way", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "`id` cluster" }),
      ).toBe("kubectl get pods web-0 --context '`id` cluster'");
    });

    it("keeps a plain apostrophe (no expansion syntax) safely double-quoted", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "bob's cluster" }),
      ).toBe('kubectl get pods web-0 --context "bob\'s cluster"');
    });

    it("apostrophe + expansion syntax becomes a placeholder — the POSIX '\\'' escape breaks out in PowerShell", () => {
      // In pwsh, backslash is not a quote escape: 'a'\''$(whoami)'\''b'
      // leaves $(whoami) outside any string and it executes. No single
      // representation is inert in bash/zsh AND pwsh, so refuse to render
      // the value, exactly like the Windows tier.
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "a'$(whoami)'b" }),
      ).toBe('kubectl get pods web-0 --context "<enter context>"');
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "bob's $HOME" }),
      ).toBe('kubectl get pods web-0 --context "<enter context>"');
    });

    it("windows: double quotes hold inert odd values — spaces and even & are literal inside them in cmd and PowerShell", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "my cluster" }, true),
      ).toBe('kubectl get pods web-0 --context "my cluster"');
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "a & b" }, true),
      ).toBe('kubectl get pods web-0 --context "a & b"');
    });

    it("windows: values with no cmd+PowerShell-safe representation become a fill-in placeholder", () => {
      // Single quotes aren't quoting in cmd (& would chain a command through
      // them) and double quotes leave $/` live in PowerShell and % live in
      // cmd — so the hostile value must not be rendered at all.
      expect(
        toKubectl({ action: "get", kind: "Pod", name: "web-0", context: "x$&whoami>pwned&" }, true),
      ).toBe('kubectl get pods web-0 --context "<enter context>"');
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "%TEMP%", name: "web-0", context: "prod" }, true),
      ).toBe('kubectl get pods web-0 -n "<enter namespace>" --context prod');
    });

    it("leaves plain resource names unquoted (DNS-1123 names are always in the safe set)", () => {
      expect(
        toKubectl({ action: "get", kind: "Pod", namespace: "default", name: "web-0", context: "prod" }),
      ).toBe("kubectl get pods web-0 -n default --context prod");
    });
  });
});
