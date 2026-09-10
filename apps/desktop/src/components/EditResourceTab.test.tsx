import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  CodeEditor: ({ value, onChange, ariaLabel }: { value: string; onChange?: (v: string) => void; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

import { EditResourceTab } from "./EditResourceTab";

function StatefulEditResourceTab(
  props: Omit<React.ComponentProps<typeof EditResourceTab>, "draft" | "onDraftChange">,
) {
  const [draft, setDraft] = React.useState<string | null>(null);
  return <EditResourceTab {...props} draft={draft} onDraftChange={setDraft} />;
}

describe("EditResourceTab", () => {
  it("preloads the resource's manifest into the editor with an Apply action", async () => {
    loadEditableManifestMock.mockResolvedValue({ yaml: "kind: ConfigMap\nmetadata:\n  name: web\n" });
    render(
      <StatefulEditResourceTab
        context="kind-dev"
        kind="ConfigMap"
        namespace="default"
        name="web"
      />,
    );
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
    render(
      <StatefulEditResourceTab
        context="kind-dev"
        kind="Pod"
        namespace="default"
        name="ghost"
      />,
    );
    expect(await screen.findByText(/not found/)).toBeDefined();
  });

  it("uses the tab's existing draft without fetching over it", () => {
    loadEditableManifestMock.mockClear();
    const onDraftChange = vi.fn();
    const draft = "kind: ConfigMap\nmetadata:\n  name: unsaved\n";
    render(
      <EditResourceTab
        context="kind-dev"
        kind="ConfigMap"
        namespace="default"
        name="web"
        draft={draft}
        onDraftChange={onDraftChange}
      />,
    );

    const editor = screen.getByLabelText("Edit resource YAML") as HTMLTextAreaElement;
    expect(editor.value).toBe(draft);
    fireEvent.change(editor, { target: { value: `${draft}data:\n  key: changed\n` } });
    expect(onDraftChange).toHaveBeenCalledWith(`${draft}data:\n  key: changed\n`);
    expect(loadEditableManifestMock).not.toHaveBeenCalled();
  });
});
