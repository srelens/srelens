import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";

const { loadEditableManifestMock } = vi.hoisted(() => ({ loadEditableManifestMock: vi.fn() }));
vi.mock("@srelens/core/lib/manifestEdit", () => ({ loadEditableManifest: loadEditableManifestMock }));
vi.mock("@srelens/core/lib/manifest", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/manifest")>()),
  applyManifest: vi.fn(),
  validateManifest: vi.fn().mockResolvedValue({ valid: true }),
}));
vi.mock("@srelens/core/lib/schema", () => ({ openApiSchema: vi.fn().mockResolvedValue({ error: "n/a" }) }));
vi.mock("../ui/CodeEditor", () => ({
  CodeEditor: ({ value, ariaLabel }: { value: string; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} readOnly />
  ),
}));

import { EditResourceTab } from "./EditResourceTab";

describe("EditResourceTab", () => {
  it("preloads the resource's manifest into the editor with an Apply action", async () => {
    loadEditableManifestMock.mockResolvedValue({ yaml: "kind: ConfigMap\nmetadata:\n  name: web\n" });
    render(<EditResourceTab context="kind-dev" kind="ConfigMap" namespace="default" name="web" />);
    await waitFor(() =>
      expect(loadEditableManifestMock).toHaveBeenCalledWith("kind-dev", "ConfigMap", "default", "web"),
    );
    expect((await screen.findByLabelText("Edit resource YAML")) as HTMLTextAreaElement).toBeDefined();
    expect((screen.getByLabelText("Edit resource YAML") as HTMLTextAreaElement).value).toContain("kind: ConfigMap");
    expect(screen.getByText("Edit ConfigMap/web")).toBeDefined();
    expect(screen.getByRole("button", { name: "Apply" })).toBeDefined();
  });

  it("shows an error when the manifest can't be loaded", async () => {
    loadEditableManifestMock.mockResolvedValue({ error: "not found" });
    render(<EditResourceTab context="kind-dev" kind="Pod" namespace="default" name="ghost" />);
    expect(await screen.findByText(/not found/)).toBeDefined();
  });
});
