import { describe, expect, it } from "vitest";
import {
  cleanErrorMessage,
  describeError,
  describeForbidden,
  isExecAuthError,
  serviceAccountNamespace,
} from "./errors";

describe("isExecAuthError", () => {
  it("matches exec credential plugin failures", () => {
    expect(isExecAuthError("unable to run auth exec: no such file or directory")).toBe(true);
    expect(isExecAuthError('exec: "kubectl-oidc_login": executable file not found in $PATH')).toBe(true);
    expect(isExecAuthError("getting credentials: exec plugin failed")).toBe(true);
  });
  it("does not match unrelated errors", () => {
    expect(isExecAuthError("connection refused")).toBe(false);
    expect(isExecAuthError("Unauthorized")).toBe(false);
  });
  it("describeError gives platform-appropriate exec-auth guidance", () => {
    // Web (jsdom default, no Tauri): can't run plugins → point to Add cluster.
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
    expect(describeError("unable to run auth exec: executable not found").title).toBe(
      "This cluster needs OIDC sign-in",
    );
    // Desktop: the plugin can be installed/run locally → Toolbox guidance.
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    expect(describeError("unable to run auth exec: executable not found").title).toBe(
      "Auth plugin couldn't run",
    );
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });
});

describe("cleanErrorMessage", () => {
  it("strips the internal handler-error prefix", () => {
    expect(cleanErrorMessage("handler error: list namespaces timed out")).toBe(
      "list namespaces timed out",
    );
  });

  it("reads the message off an Error instance", () => {
    expect(cleanErrorMessage(new Error("boom"))).toBe("boom");
  });

  it("coerces non-string, non-Error values safely", () => {
    expect(cleanErrorMessage(null)).toBe("");
    expect(cleanErrorMessage(undefined)).toBe("");
    expect(cleanErrorMessage(42)).toBe("42");
  });
});

describe("describeError", () => {
  it("classifies a connection timeout and never leaks the handler prefix", () => {
    const result = describeError("handler error: list namespaces timed out");
    expect(result.title).toBe("Can't reach the cluster");
    expect(result.detail).toMatch(/didn't respond in time/);
    expect(result.detail).not.toMatch(/handler error/);
    expect(result.raw).toBe("list namespaces timed out");
  });

  it("classifies a refused connection", () => {
    expect(describeError("tcp connect error: connection refused").title).toBe(
      "Can't reach the cluster",
    );
  });

  it("classifies an unresolved host", () => {
    expect(describeError("failed to lookup address information: no such host").title).toBe(
      "Cluster address not found",
    );
  });

  it("classifies auth failures distinctly", () => {
    expect(describeError("Unauthorized").title).toBe("Not authorized");
    expect(describeError("forbidden: pods is forbidden").title).toBe("Access denied");
  });

  it("classifies a cluster-login marker as a distinct sign-in prompt, not generic unauthorized", () => {
    expect(describeError("NEEDS_CLUSTER_LOGIN:abc123:my-context").title).toBe(
      "Cluster sign-in required",
    );
    expect(describeError("cluster_login_required").title).toBe("Cluster sign-in required");
  });

  it("classifies TLS/certificate failures", () => {
    expect(describeError("x509: certificate signed by unknown authority").title).toBe(
      "Couldn't verify the cluster",
    );
  });

  it("falls back to the cleaned message for unrecognized errors", () => {
    const result = describeError("handler error: something weird happened");
    expect(result.title).toBe("Something went wrong");
    expect(result.detail).toBe("something weird happened");
  });

  it("gives a stable message when there is nothing to show", () => {
    expect(describeError("").detail).toBe("An unexpected error occurred.");
  });
});

describe("describeForbidden", () => {
  it("extracts verb/resource/namespace from an apiserver 403", () => {
    const raw = 'deployments.apps is forbidden: User "dev" cannot patch resource "deployments" in API group "apps" in the namespace "prod"';
    expect(describeForbidden(raw)).toBe("You don't have permission to patch deployments in prod.");
  });
  it("handles cluster-scoped denials", () => {
    const raw = 'nodes is forbidden: User "dev" cannot patch resource "nodes" in API group "" at the cluster scope';
    expect(describeForbidden(raw)).toBe("You don't have permission to patch nodes at the cluster scope.");
  });
  it("returns null when it can't parse", () => {
    expect(describeForbidden("some other error")).toBeNull();
  });
  it("describeError uses it for a forbidden error", () => {
    const raw = 'pods is forbidden: User "dev" cannot delete resource "pods" in API group "" in the namespace "prod"';
    expect(describeError(raw).detail).toContain("You don't have permission to delete pods in prod");
  });
});

describe("serviceAccountNamespace", () => {
  it("extracts the SA namespace from a forbidden error", () => {
    const raw =
      'namespaces is forbidden: User "system:serviceaccount:clavik-dev:clavik-dev" cannot list resource "namespaces" in API group "" at the cluster scope';
    expect(serviceAccountNamespace(raw)).toBe("clavik-dev");
  });
  it("returns null for a non-service-account forbidden error", () => {
    const raw = 'namespaces is forbidden: User "alice" cannot list resource "namespaces" in API group "" at the cluster scope';
    expect(serviceAccountNamespace(raw)).toBeNull();
  });
  it("returns null for unrelated text", () => {
    expect(serviceAccountNamespace("some other error")).toBeNull();
  });
});
