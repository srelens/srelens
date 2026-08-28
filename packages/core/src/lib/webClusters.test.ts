import { afterEach, describe, expect, it, vi } from "vitest";
import { clusterLogout, createCluster, listClusters, type OidcClusterRow } from "./webClusters";

function setDesktop(on: boolean) {
  if (on) {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
}

describe("webClusters.listClusters", () => {
  afterEach(() => vi.restoreAllMocks());

  it("GETs /api/clusters with credentials and returns data.clusters", async () => {
    const clusters: OidcClusterRow[] = [
      { key: "prod", issuer: "https://issuer", clientId: "abc", contexts: ["prod-ctx"], signedIn: true, expiresAt: 123 },
    ];
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ clusters }), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    expect(await listClusters()).toEqual(clusters);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/clusters");
    expect(init.credentials).toBe("include");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
  });

  it("returns an empty array when data.clusters is missing", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({}), { status: 200 })));
    expect(await listClusters()).toEqual([]);
  });

  it("throws the server error on a non-ok response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 })),
    );
    await expect(listClusters()).rejects.toThrow("unauthorized");
  });

  it("falls back to a status-based message when the error body isn't JSON", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 500 })));
    await expect(listClusters()).rejects.toThrow("request failed: 500");
  });
});

describe("webClusters.createCluster", () => {
  afterEach(() => vi.restoreAllMocks());

  it("POSTs the input as JSON, including the oidc sub-object", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    const input = {
      name: "prod",
      server: "https://prod.example.com",
      caCertPem: "-----BEGIN CERTIFICATE-----",
      insecureSkipTlsVerify: false,
      oidc: {
        issuer: "https://issuer.example.com",
        clientId: "client-1",
        clientSecret: "secret",
        extraScopes: ["groups"],
      },
    };

    await createCluster(input);

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/clusters");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
    expect(JSON.parse(init.body)).toEqual(input);
  });

  it("throws the server error on a non-ok response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "name must not be empty" }), { status: 400 })),
    );
    await expect(
      createCluster({ name: "", server: "https://x", insecureSkipTlsVerify: false }),
    ).rejects.toThrow("name must not be empty");
  });

  it("falls back to a status-based message when the error body isn't JSON", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 500 })));
    await expect(
      createCluster({ name: "prod", server: "https://x", insecureSkipTlsVerify: false }),
    ).rejects.toThrow("request failed: 500");
  });
});

describe("webClusters.clusterLogout", () => {
  afterEach(() => vi.restoreAllMocks());

  it("POSTs /api/clusters/<encoded-key>/logout", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await clusterLogout("prod cluster/1");

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/clusters/prod%20cluster%2F1/logout");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
  });

  it("throws the server error on a non-ok response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "not found" }), { status: 404 })),
    );
    await expect(clusterLogout("prod")).rejects.toThrow("not found");
  });
});

vi.mock("../transport/transport", () => ({ invokeCommand: vi.fn() }));

describe("webClusters on desktop", () => {
  afterEach(() => {
    setDesktop(false);
    vi.restoreAllMocks();
  });

  it("listClusters calls invokeCommand(list_clusters) and returns its result", async () => {
    setDesktop(true);
    const { invokeCommand } = await import("../transport/transport");
    const clusters: OidcClusterRow[] = [
      { key: "prod", issuer: "https://issuer", clientId: "abc", contexts: ["prod-ctx"], signedIn: true, expiresAt: 123 },
    ];
    vi.mocked(invokeCommand).mockResolvedValue(clusters);

    expect(await listClusters()).toEqual(clusters);
    expect(invokeCommand).toHaveBeenCalledWith("list_clusters");
  });

  it("clusterLogout calls invokeCommand(cluster_logout, { key })", async () => {
    setDesktop(true);
    const { invokeCommand } = await import("../transport/transport");
    vi.mocked(invokeCommand).mockResolvedValue(undefined);

    await clusterLogout("k");

    expect(invokeCommand).toHaveBeenCalledWith("cluster_logout", { key: "k" });
  });
});
