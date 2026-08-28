import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("@srelens/core/lib/helm", () => ({
  listHelmReleases: vi.fn().mockResolvedValue({ releases: [] }),
  getHelmRelease: vi.fn().mockResolvedValue({ release: null }),
  helmVersion: vi.fn().mockResolvedValue({ error: "helm not found on PATH" }),
  helmRepoUpdate: vi.fn(),
  helmRepoAdd: vi.fn(),
}));
vi.mock("@srelens/core/lib/useNamespaceOptions", () => ({
  useNamespaceOptions: () => ({ namespaces: [], scope: "", error: "" }),
}));
vi.mock("../ui/CodeEditor", () => ({ CodeEditor: () => null }));

import { HelmReleasesView } from "./HelmReleasesView";

describe("HelmReleasesView helm-missing notice", () => {
  it("shows an install-Helm notice when helmVersion errors", async () => {
    render(<HelmReleasesView context="ctx" />);
    await waitFor(() => expect(screen.getByText(/install Helm/i)).toBeDefined());
  });
});
