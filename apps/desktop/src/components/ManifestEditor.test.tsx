import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import React, { useState } from "react";

const { applyManifestMock, diffManifestMock, notifyMock } = vi.hoisted(() => ({
  applyManifestMock: vi.fn(),
  diffManifestMock: vi.fn(),
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn(), updateAvailable: vi.fn() },
}));
vi.mock("../lib/manifest", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/manifest")>()),
  applyManifest: applyManifestMock,
  diffManifest: diffManifestMock,
  validateManifest: vi.fn().mockResolvedValue({ valid: true }),
}));
vi.mock("../lib/notify", () => ({ notify: notifyMock }));
vi.mock("../lib/schema", () => ({ openApiSchema: vi.fn().mockResolvedValue({ error: "n/a" }) }));
vi.mock("../ui/CodeEditor", () => ({
  CodeEditor: ({ value, onChange, ariaLabel }: { value: string; onChange?: (v: string) => void; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

import { ManifestEditor } from "./ManifestEditor";

function Harness({ mode }: { mode: "create" | "edit" }) {
  const [yaml, setYaml] = useState("kind: ConfigMap\nmetadata:\n  name: web\n");
  return (
    <ManifestEditor
      context="kind-dev"
      yaml={yaml}
      onYamlChange={setYaml}
      applyLabel={mode === "create" ? "Create" : "Apply"}
      confirm={mode === "edit" ? { kind: "ConfigMap", name: "web" } : undefined}
    />
  );
}

beforeEach(() => {
  applyManifestMock.mockReset();
  diffManifestMock.mockReset();
  diffManifestMock.mockResolvedValue({ documents: [] });
  notifyMock.success.mockReset();
  notifyMock.error.mockReset();
});

describe("ManifestEditor", () => {
  it("create mode applies immediately (no confirm) and toasts success", async () => {
    applyManifestMock.mockResolvedValue({ applied: true, documents: [{ kind: "ConfigMap", name: "web", applied: true }] });
    render(<Harness mode="create" />);
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(applyManifestMock).toHaveBeenCalledWith("kind-dev", expect.stringContaining("ConfigMap"), false));
    expect(notifyMock.success).toHaveBeenCalled();
  });

  it("edit mode confirms before applying", async () => {
    applyManifestMock.mockResolvedValue({ applied: true, documents: [{ kind: "ConfigMap", name: "web", applied: true }] });
    render(<Harness mode="edit" />);
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    // Apply doesn't fire until the confirm dialog is accepted.
    expect(applyManifestMock).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByText("Apply manifest?")).toBeDefined());
    // Two "Apply" buttons now (header + dialog); the dialog's is last.
    fireEvent.click(screen.getAllByRole("button", { name: "Apply" }).at(-1)!);
    await waitFor(() => expect(applyManifestMock).toHaveBeenCalled());
    expect(notifyMock.success).toHaveBeenCalled();
  });

  it("toasts an error when apply fails", async () => {
    applyManifestMock.mockResolvedValue({ error: "conflict" });
    render(<Harness mode="create" />);
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(notifyMock.error).toHaveBeenCalled());
  });

  it("offers Force apply on a conflict, then re-applies with force", async () => {
    applyManifestMock
      .mockResolvedValueOnce({
        applied: false,
        documents: [
          {
            kind: "Deployment",
            name: "web",
            applied: false,
            conflict: { managers: ["kubectl"], fields: [".spec.replicas"], message: "conflict" },
          },
        ],
      })
      .mockResolvedValueOnce({ applied: true, documents: [{ kind: "Deployment", name: "web", applied: true }] });

    render(
      <ManifestEditor
        context="ctx"
        yaml={'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  resourceVersion: "1"\n'}
        onYamlChange={() => {}}
        fill
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(screen.getByText("Apply manifest?")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Apply" })); // confirm dialog's — outside content is aria-hidden
    const force = await screen.findByRole("button", { name: /force apply/i });
    expect(screen.getByText(/kubectl/)).toBeDefined();
    fireEvent.click(force);
    await waitFor(() => expect(applyManifestMock).toHaveBeenLastCalledWith("ctx", expect.any(String), true));
  });

  it("drawer (non-fill) surfaces conflicts too: Force apply appears and re-applies with force", async () => {
    applyManifestMock
      .mockResolvedValueOnce({
        applied: false,
        documents: [
          {
            kind: "Deployment",
            name: "web",
            applied: false,
            conflict: { managers: ["kubectl"], fields: [".spec.replicas"], message: "conflict" },
          },
        ],
      })
      .mockResolvedValueOnce({ applied: true, documents: [{ kind: "Deployment", name: "web", applied: true }] });

    render(
      <ManifestEditor
        context="ctx"
        yaml={'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  resourceVersion: "1"\n'}
        onYamlChange={() => {}}
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(screen.getByText("Apply manifest?")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Apply" })); // confirm dialog's — outside content is aria-hidden
    const force = await screen.findByRole("button", { name: /force apply/i });
    expect(screen.getByText(/kubectl/)).toBeDefined();
    fireEvent.click(force);
    await waitFor(() => expect(applyManifestMock).toHaveBeenLastCalledWith("ctx", expect.any(String), true));
  });

  it("multi-doc conflict banner names each conflicting document, not the applied one", async () => {
    applyManifestMock.mockResolvedValue({
      applied: false,
      documents: [
        { kind: "Deployment", name: "web", applied: true },
        {
          kind: "ConfigMap",
          name: "cfg",
          applied: false,
          conflict: { managers: ["kubectl"], fields: [".data.x"], message: "conflict on cfg" },
        },
      ],
    });
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n"}
        onYamlChange={() => {}}
        fill
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    const banner = await screen.findByRole("alert");
    expect(within(banner).getByText(/ConfigMap\/cfg/)).toBeDefined();
    expect(within(banner).queryByText(/Deployment\/web/)).toBeNull();
    expect(within(banner).getByText(/kubectl/)).toBeDefined();
    expect(within(banner).getByTitle(/conflict on cfg/)).toBeDefined();
  });

  it("shows a stale badge when the live resourceVersion differs", async () => {
    diffManifestMock.mockResolvedValue({
      documents: [
        { kind: "Deployment", name: "web", namespace: null, exists: true, changed: true, rows: [], currentResourceVersion: "9" },
      ],
    });
    render(
      <ManifestEditor
        context="ctx"
        yaml={'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  resourceVersion: "1"\n'}
        onYamlChange={() => {}}
        fill
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Changes" }));
    expect(await screen.findByText(/changed elsewhere/i, {}, { timeout: 2000 })).toBeDefined();
  });

  it("fill mode: diff pane is hidden by default; Changes reveals a resizable split, Hide changes collapses it", async () => {
    const { container } = render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n"}
        onYamlChange={() => {}}
        fill
      />,
    );
    // Hidden by default: full-width editor, no diff content, no resize handle.
    expect(screen.queryByText("No changes")).toBeNull();
    expect(container.querySelector(".fl-changes-panel__resize")).toBeNull();
    expect(container.querySelector(".fl-changes-panel")).toBeNull();
    expect(screen.getByRole("button", { name: "Changes" })).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Changes" }));
    await waitFor(() => expect(diffManifestMock).toHaveBeenCalled());
    expect(await screen.findByText("No changes")).toBeDefined();
    expect(container.querySelector(".fl-changes-panel__resize")).not.toBeNull();
    expect(container.querySelector(".fl-changes-panel")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Hide changes" })).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Hide changes" }));
    expect(screen.queryByText("No changes")).toBeNull();
    expect(container.querySelector(".fl-changes-panel__resize")).toBeNull();
    expect(container.querySelector(".fl-changes-panel")).toBeNull();
  });
});
