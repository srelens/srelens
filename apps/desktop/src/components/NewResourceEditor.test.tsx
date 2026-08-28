import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const { applyManifestMock } = vi.hoisted(() => ({ applyManifestMock: vi.fn() }));
vi.mock("@srelens/core/lib/manifest", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/manifest")>()),
  applyManifest: applyManifestMock,
}));
vi.mock("../ui/CodeEditor", () => ({
  CodeEditor: ({ value, onChange, ariaLabel }: { value: string; onChange?: (v: string) => void; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

import { NewResourceEditor } from "./NewResourceEditor";

beforeEach(() => applyManifestMock.mockReset());

describe("NewResourceEditor", () => {
  it("prefills a template and applies it, staying open on success", async () => {
    applyManifestMock.mockResolvedValue({
      applied: true,
      documents: [{ kind: "Service", name: "my-app", applied: true }],
    });
    const onCreated = vi.fn();
    render(<NewResourceEditor context="kind-dev" namespace="prod" initialKind="Service" onCreated={onCreated} />);

    const editor = (await screen.findByLabelText("New resource YAML")) as HTMLTextAreaElement;
    expect(editor.value).toContain("kind: Service");
    expect(editor.value).toContain("namespace: prod");

    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(applyManifestMock).toHaveBeenCalledWith("kind-dev", editor.value, false));
    expect(await screen.findByText(/Applied Service/)).toBeDefined();
    expect(onCreated).toHaveBeenCalled();
    // Editor is still present (tab stays open to create more).
    expect(screen.getByLabelText("New resource YAML")).toBeDefined();
  });

  it("surfaces an apply error", async () => {
    applyManifestMock.mockResolvedValue({ error: "invalid manifest" });
    render(<NewResourceEditor context="kind-dev" />);
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(await screen.findByText(/invalid manifest/)).toBeDefined();
  });
});
