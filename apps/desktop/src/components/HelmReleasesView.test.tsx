import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { listHelmReleasesMock, getHelmReleaseMock, useNamespaceOptionsMock } = vi.hoisted(() => ({
  listHelmReleasesMock: vi.fn(),
  getHelmReleaseMock: vi.fn(),
  useNamespaceOptionsMock: vi.fn(),
}));
vi.mock("@srelens/core/lib/helm", async (importOriginal) => ({
  listHelmReleases: listHelmReleasesMock,
  // The real per-namespace fan-out, run over the mock above.
  listHelmReleasesIn: (context: string, selection: string[]) =>
    importOriginal<typeof import("@srelens/core/lib/helm")>().then((real) =>
      real.listHelmReleasesIn(context, selection, (...a) => listHelmReleasesMock(...a)),
    ),
  getHelmRelease: getHelmReleaseMock,
  helmVersion: vi.fn().mockResolvedValue({ version: "v3.14.0" }),
  helmRepoUpdate: vi.fn().mockResolvedValue({ output: "" }),
  helmRepoAdd: vi.fn().mockResolvedValue({ output: "" }),
}));
// Namespace dropdown options come from this hook now (not the loaded
// releases) — stub it so the dropdown has options without exercising the
// real listNamespaces/listContexts backend calls.
vi.mock("@srelens/core/lib/useNamespaceOptions", () => ({
  useNamespaceOptions: useNamespaceOptionsMock,
}));
// CodeMirror needs real layout; stand in a textarea.
vi.mock("../ui/CodeEditor", () => ({
  // `copy` rides on a data attribute: the real control is the kit's, tested
  // there, and what matters at a CALL site is that the pane asked for one.
  CodeEditor: ({ value, ariaLabel, copy }: { value: string; ariaLabel?: string; copy?: boolean }) => (
    <textarea aria-label={ariaLabel} value={value} readOnly data-copy={String(!!copy)} />
  ),
}));

import { HelmReleasesView } from "./HelmReleasesView";

const release = {
  name: "redis",
  namespace: "cache",
  revision: 2,
  status: "deployed",
  chart: "redis",
  chartVersion: "19.0.1",
  appVersion: "7.2.4",
  updated: "2026-07-01T00:00:00Z",
};

const otherRelease = {
  name: "nginx-ingress",
  namespace: "ingress",
  revision: 1,
  status: "deployed",
  chart: "nginx-ingress",
  chartVersion: "4.1.0",
  appVersion: "1.9.0",
  updated: "2026-07-01T00:00:00Z",
};

const releaseDetail = {
  ...release,
  valuesYaml: "replicas: 1\n",
  manifest: "kind: Service\n",
  notes: "",
  history: [
    { revision: 2, status: "deployed", updated: "", chartVersion: "19.0.1", description: "Upgrade complete" },
    { revision: 1, status: "superseded", updated: "", chartVersion: "18.0.0", description: "Install complete" },
  ],
};

beforeEach(() => {
  listHelmReleasesMock.mockReset();
  getHelmReleaseMock.mockReset();
  useNamespaceOptionsMock.mockReset();
  listHelmReleasesMock.mockResolvedValue({ releases: [release] });
  getHelmReleaseMock.mockResolvedValue({ release: releaseDetail });
  useNamespaceOptionsMock.mockReturnValue({ namespaces: ["cache", "ingress"], scope: "", error: "" });
});

describe("HelmReleasesView", () => {
  it("lists releases and opens values/manifest/history detail", async () => {
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(screen.getByText("redis")).toBeDefined());
    expect(screen.getByText("redis-19.0.1")).toBeDefined();

    fireEvent.click(screen.getByText("redis"));

    // Values tab (default) shows the user values.
    await waitFor(() =>
      expect((screen.getByLabelText("Release values") as HTMLTextAreaElement).value).toContain(
        "replicas: 1",
      ),
    );
    expect(getHelmReleaseMock).toHaveBeenCalledWith("kind-dev", "cache", "redis");

    // Manifest tab.
    await userEvent.click(screen.getByRole("tab", { name: "Manifest" }));
    await waitFor(() =>
      expect((screen.getByLabelText("Release manifest") as HTMLTextAreaElement).value).toContain(
        "kind: Service",
      ),
    );

    // History tab.
    await userEvent.click(screen.getByRole("tab", { name: /History/ }));
    expect(await screen.findByText("Upgrade complete")).toBeDefined();
  });

  it("offers to copy the values and the manifest", async () => {
    // Both panes are read-only text a reader opens in order to take it away,
    // and neither had any way to do it but a chord the browser would not aim
    // at a `contenteditable`. Asserted at the CALL site because that is what
    // a later edit would silently drop. (#656 review)
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(screen.getByText("redis")).toBeDefined());
    fireEvent.click(screen.getByText("redis"));

    expect((await screen.findByLabelText("Release values")).dataset.copy).toBe("true");
    await userEvent.click(screen.getByRole("tab", { name: "Manifest" }));
    expect((await screen.findByLabelText("Release manifest")).dataset.copy).toBe("true");
  });

  it("shows an empty state when no releases", async () => {
    listHelmReleasesMock.mockResolvedValue({ releases: [] });
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(screen.getByText(/No Helm releases/)).toBeDefined());
  });

  it("filters the table by typing in the search box", async () => {
    listHelmReleasesMock.mockResolvedValue({ releases: [release, otherRelease] });
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(screen.getByText("redis")).toBeDefined());
    expect(screen.getByText("nginx-ingress")).toBeDefined();

    await userEvent.type(screen.getByLabelText("Search resources"), "redis");

    await waitFor(() => expect(screen.queryByText("nginx-ingress")).toBeNull());
    expect(screen.getByText("redis")).toBeDefined();
  });

  it("narrows rows when a namespace is selected in the filter", async () => {
    listHelmReleasesMock.mockResolvedValue({ releases: [release, otherRelease] });
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(screen.getByText("redis")).toBeDefined());
    expect(screen.getByText("nginx-ingress")).toBeDefined();

    await userEvent.click(screen.getByLabelText("Namespace"));
    await userEvent.click(await screen.findByRole("option", { name: "cache" }));

    await waitFor(() => expect(screen.queryByText("nginx-ingress")).toBeNull());
    expect(screen.getByText("redis")).toBeDefined();
  });

  it("exposes release actions as icon buttons in the drawer header", async () => {
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(screen.getByText("redis")).toBeDefined());

    fireEvent.click(screen.getByText("redis"));

    await waitFor(() => expect(screen.getByRole("button", { name: "Upgrade" })).toBeDefined());
    expect(screen.getByRole("button", { name: "Rollback" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Uninstall" })).toBeDefined();
  });

  it("scopes the release fetch to the selected namespace (perf: no all-namespace listing)", async () => {
    render(<HelmReleasesView context="kind-dev" initialNamespace="cache" />);
    await waitFor(() => expect(listHelmReleasesMock).toHaveBeenCalled());

    expect(listHelmReleasesMock.mock.calls[0][0]).toBe("kind-dev");
    expect(listHelmReleasesMock.mock.calls[0][1]).toBe("cache");
    expect(listHelmReleasesMock.mock.calls[0][1]).not.toBeNull();
  });

  // #688: an unscoped `helm list` reads release Secrets cluster-wide, which a
  // credential scoped to a few namespaces is refused.
  it("lists each selected namespace on its own when several are selected", async () => {
    listHelmReleasesMock.mockImplementation(async (_c: string, ns: string | null) => ({
      releases: [release, otherRelease].filter((r) => r.namespace === ns),
    }));
    render(<HelmReleasesView context="kind-dev" initialNamespace={`${release.namespace},${otherRelease.namespace}`} />);
    await waitFor(() => expect(listHelmReleasesMock).toHaveBeenCalledTimes(2));
    expect(listHelmReleasesMock.mock.calls.map((c) => c[1])).toEqual([release.namespace, otherRelease.namespace]);
    expect(await screen.findByText(release.name)).toBeDefined();
    expect(screen.getByText(otherRelease.name)).toBeDefined();
  });

  it("keeps the namespace that answered and names the one that was refused", async () => {
    listHelmReleasesMock.mockImplementation(async (_c: string, ns: string | null) =>
      ns === otherRelease.namespace
        ? { error: `secrets is forbidden: cannot list resource "secrets" in the namespace "${ns}"` }
        : { releases: [release] },
    );
    render(<HelmReleasesView context="kind-dev" initialNamespace={`${release.namespace},${otherRelease.namespace}`} />);
    expect(await screen.findByText(release.name)).toBeDefined();
    expect(screen.getByText(new RegExp(`Could not list releases in ${otherRelease.namespace}`))).toBeDefined();
    expect(screen.queryByText(/^Error:/)).toBeNull();
  });

  it("gives each refused namespace its own reason", async () => {
    listHelmReleasesMock.mockImplementation(async (_c: string, ns: string | null) => {
      if (ns === "team-b") return { error: 'secrets is forbidden: User "dev" cannot list resource "secrets" in API group "" in the namespace "team-b"' };
      if (ns === "team-c") return { error: "dial tcp 10.1.2.3:6443: connect: connection refused" };
      return { releases: [release] };
    });
    render(<HelmReleasesView context="kind-dev" initialNamespace={`${release.namespace},team-b,team-c`} />);
    expect(await screen.findByText(release.name)).toBeDefined();
    const notice = screen.getByText(/Could not list releases in team-b and team-c/);
    expect(notice.textContent).toMatch(/team-b: You don.t have permission to list secrets in team-b/);
    expect(notice.textContent).toMatch(/team-c: .*reach|team-c: .*connect/i);
  });

  it("fetches all releases when no namespace is selected", async () => {
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(listHelmReleasesMock).toHaveBeenCalled());

    expect(listHelmReleasesMock.mock.calls[0][0]).toBe("kind-dev");
    expect(listHelmReleasesMock.mock.calls[0][1]).toBeNull();
  });
});
