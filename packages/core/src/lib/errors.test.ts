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

  it("strips the class name String() prints in front of a stringified Error", () => {
    // `podCount`, `getManifest`, `clusters` and every other lib wrapper report
    // a rejection as `{ error: String(e) }`, and `String(new Error(m))` is
    // `Error: ${m}`. The handler prefix is anchored at the start, so that one
    // word was enough to stop it matching — which is how the overview's Fleet
    // rows came to print `Error: handler error: ApiError: …` verbatim.
    expect(cleanErrorMessage("Error: handler error: list pods timed out")).toBe(
      "list pods timed out",
    );
  });

  it("keeps stripping while the prefixes stack", () => {
    expect(cleanErrorMessage("Error: Error: handler error: nope")).toBe("nope");
  });

  it("strips a prefix, not a word that happens to start the message", () => {
    // Only the two known prefixes, and only with their colon. A message that
    // opens with a word ending in "error" is the message, not a wrapper.
    expect(cleanErrorMessage("Errors were found in the manifest")).toBe(
      "Errors were found in the manifest",
    );
    expect(cleanErrorMessage("ApiError: Unauthorized")).toBe("ApiError: Unauthorized");
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
    expect(result.title).toBe("Request timed out");
    expect(result.detail).toMatch(/didn't respond in time/);
    expect(result.detail).not.toMatch(/handler error/);
    expect(result.raw).toBe("list namespaces timed out");
  });

  it("points each platform at the timeout it can actually change (#238)", () => {
    // Web (jsdom default): the Settings slider doesn't exist there, so the
    // server-side env var is the only real remedy.
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
    expect(describeError("list pods timed out").detail).toMatch(/SRELENS_TIMEOUT_SECS/);
    expect(describeError("list pods timed out").detail).not.toMatch(/Settings → Kubernetes/);

    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    expect(describeError("list pods timed out").detail).toMatch(/Request timeout in Settings/);
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("classifies a refused connection", () => {
    expect(describeError("tcp connect error: connection refused").title).toBe(
      "Can't reach the cluster",
    );
  });

  it("classifies hyper's own opaque connect failure, which is what a cluster being down looks like", () => {
    // Verbatim from `k8s.listNodes` against a kind cluster whose container is
    // stopped. `kubectl` says "connection refused"; the backend's stack has
    // already thrown that away by the time the string reaches the UI, and
    // "Something went wrong" for the most ordinary failure there is was the
    // least useful answer this function gave.
    const result = describeError("handler error: ServiceError: client error (Connect)");
    expect(result.title).toBe("Can't reach the cluster");
    expect(result.detail).toMatch(/Make sure the cluster is running/);
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

  it("classifies the apiserver's whole 401 as a 401, and keeps the struct only in raw", () => {
    // Verbatim from a real context in the user's kubeconfig, as `podCount`
    // hands it to the overview's Fleet rows: `String(e)` over a
    // `CapabilityError` wrapping kube-rs's `ApiError` Display. Every earlier
    // branch has to decline it — `unreachable`, `dns` and `self.signed` are
    // all substring tests, and this string is 300 characters of struct.
    const raw =
      'Error: handler error: ApiError: Unauthorized: Unauthorized (Status { status: Some("Failure"), ' +
      "metadata: Some(ListMeta { continue_: None, remaining_item_count: None, resource_version: None, " +
      'self_link: None }), reason: Some("Unauthorized"), code: Some(401), message: Some("Unauthorized") })';
    const result = describeError(raw);
    expect(result.title).toBe("Not authorized");
    expect(result.detail).toMatch(/rejected your credentials/);
    // The reader is never shown the struct in the copy — but it is not thrown
    // away either, and it no longer carries either wrapper prefix.
    expect(result.detail).not.toMatch(/ListMeta|handler error/);
    expect(result.raw).toContain("ListMeta");
    expect(result.raw.startsWith("ApiError:")).toBe(true);
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
