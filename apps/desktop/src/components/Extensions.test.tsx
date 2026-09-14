import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => true,
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  readExtension: vi.fn(),
}));
import {
  listExtensions,
  configureExtensions,
  readExtension,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionManager, ExtensionResourceSlot } from "./Extensions";
import manifest from "../../../../examples/extensions/argocd.json";
const plugin = {
  manifest,
  enabled: true,
  revision: 1,
  grants: manifest.permissions,
  settings: {},
} as InstalledExtension;
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: true,
    nextRevision: 2,
    plugins: [plugin],
  });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
});
it("opens backend-owned extension settings through classic controls", async () => {
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  fireEvent.change(screen.getByLabelText("Extension settings (JSON object)"), {
    target: { value: '{"team":"classic"}' },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "settings",
      id: manifest.id,
      settings: { team: "classic" },
    }),
  );
});
it("renders the same namespace contribution using classic resource tabs", async () => {
  render(
    <ExtensionResourceSlot
      context="classic-cluster"
      kind="Namespace"
      namespace={null}
      name="argo"
    />,
  );
  expect(await screen.findByRole("tab", { name: "Argo CD" })).toBeTruthy();
  await waitFor(() =>
    expect(readExtension).toHaveBeenCalledWith(
      manifest.id,
      1,
      "applications",
      "classic-cluster",
      "argo",
    ),
  );
  fireEvent.click(screen.getByText("Extension actions"));
  expect(
    screen.getByRole("button", { name: "Inspect Argo CD resources" }),
  ).toBeTruthy();
});
