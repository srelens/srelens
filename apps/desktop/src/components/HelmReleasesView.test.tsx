import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { listHelmReleasesMock, getHelmReleaseMock, useNamespaceOptionsMock } = vi.hoisted(() => ({
  listHelmReleasesMock: vi.fn(),
  getHelmReleaseMock: vi.fn(),
  useNamespaceOptionsMock: vi.fn(),
}));
vi.mock("@srelens/core/lib/helm", () => ({
  listHelmReleases: listHelmReleasesMock,
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
  CodeEditor: ({ value, ariaLabel }: { value: string; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} readOnly />
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

  it("fetches all releases when no namespace is selected", async () => {
    render(<HelmReleasesView context="kind-dev" />);
    await waitFor(() => expect(listHelmReleasesMock).toHaveBeenCalled());

    expect(listHelmReleasesMock.mock.calls[0][0]).toBe("kind-dev");
    expect(listHelmReleasesMock.mock.calls[0][1]).toBeNull();
  });
});
