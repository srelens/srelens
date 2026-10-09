import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { extensionClusterRoute, type ClusterContext, type InstalledExtension } from "@srelens/core";
import { resetContexts, setContexts } from "../../lib/clusters";
import { takeSettingsRequest } from "../../lib/settingsRequest";
import { defaultState } from "../../lib/tabs";
import { activeRoute, setState } from "../../lib/tabsStore";
import { Apps } from "./Apps";

const inventory = vi.hoisted(() => ({ snapshot: { status: "loading" } as Record<string, unknown> }));
vi.mock("../../extensions/inventoryStore", async (original) => ({
  ...(await original<typeof import("../../extensions/inventoryStore")>()),
  useExtensions: () => ({ ...inventory.snapshot, reload: () => {} }),
}));

const ctx = (stableId: string, name: string): ClusterContext => ({
  stableId, key: `${stableId}-key`, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const PROD = ctx("prod-id", "prod");

const app = (id: string, name: string, over: Partial<InstalledExtension> = {}, pages: { id: string; title: string }[] = []): InstalledExtension => ({
  manifest: { id, name, version: "1.0.0", contributions: { pages } },
  enabled: true, revision: 1, grants: [], settings: {}, source: "catalog", installedAt: 0, history: [], ...over,
} as unknown as InstalledExtension);
const ready = (...plugins: InstalledExtension[]) => ({ status: "ready", data: { schemaVersion: 1, nextRevision: 1, plugins } });

beforeEach(() => {
  inventory.snapshot = { status: "loading" };
  takeSettingsRequest(null);
  resetContexts();
  setContexts([PROD]);
  setState(defaultState([PROD]));
});

describe("Apps", () => {
  it("says what the inventory reports about each installed app", () => {
    inventory.snapshot = ready(
      app("io.a", "Trivy"),
      app("io.b", "Kyverno", { enabled: false }),
      app("io.c", "io.c", { enabled: false, quarantined: "its signature no longer verifies" }),
      app("io.d", "Unsigned thing", { enabled: false, policyBlocked: "this server allows only signed apps" }),
    );
    render(<Apps />);
    const list = screen.getByRole("list", { name: "Installed apps" });
    const chip = (name: string) => within(list).getByText(name).closest("li")?.querySelector(".badge")?.textContent;
    expect(chip("Trivy")).toBe("On");
    expect(chip("Kyverno")).toBe("Off");
    expect(chip("io.c")).toBe("Quarantined");
    expect(chip("Unsigned thing")).toBe("Blocked");
  });

  it("opens an app's page on the cluster in focus", async () => {
    inventory.snapshot = ready(app("io.a", "Trivy", {}, [{ id: "reports", title: "Reports" }]));
    render(<Apps />);
    await userEvent.click(screen.getByRole("button", { name: "Open Trivy on prod" }));
    expect(activeRoute()).toBe(extensionClusterRoute(PROD.key, "io.a", "reports"));
  });

  it("sends an app with no page here to its settings instead", async () => {
    inventory.snapshot = ready(app("io.b", "Kyverno", { contexts: ["other-key"] }, [{ id: "policies", title: "Policies" }]));
    render(<Apps />);
    await userEvent.click(screen.getByRole("button", { name: "Kyverno in Settings › Apps" }));
    expect(activeRoute()).toBe("/settings");
    expect(takeSettingsRequest(null)).toEqual({ section: "extensions", tabId: expect.any(String) });
  });

  it("browses the catalog in Settings", async () => {
    inventory.snapshot = ready();
    render(<Apps />);
    expect(screen.getByText("No apps installed")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Browse catalog" }));
    expect(activeRoute()).toBe("/settings");
    expect(takeSettingsRequest(null)).toEqual({ section: "extensions", tab: "catalog", tabId: expect.any(String) });
  });

  it("says the list could not be read, rather than that there are no apps", () => {
    inventory.snapshot = { status: "error", error: "connection refused" };
    render(<Apps />);
    expect(screen.getByText("Could not read your apps")).toBeTruthy();
    expect(screen.queryByText("No apps installed")).toBeNull();
  });
});
