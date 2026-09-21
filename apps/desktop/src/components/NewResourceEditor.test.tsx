import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const { applyManifestMock } = vi.hoisted(() => ({ applyManifestMock: vi.fn() }));
vi.mock("@srelens/core/lib/manifest", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/manifest")>()),
  applyManifest: applyManifestMock,
}));
vi.mock("../ui/CodeEditor", () => ({
  // `copy` rides on a data attribute: the real control is the kit's, tested
  // there, and what matters at a CALL site is that the pane asked for one.
  CodeEditor: ({
    value,
    onChange,
    ariaLabel,
    copy,
  }: {
    value: string;
    onChange?: (v: string) => void;
    ariaLabel?: string;
    copy?: boolean;
  }) => (
    <textarea
      aria-label={ariaLabel}
      value={value}
      onChange={(e) => onChange?.(e.target.value)}
      data-copy={String(!!copy)}
    />
  ),
}));

import { NewResourceEditor } from "./NewResourceEditor";

beforeEach(() => applyManifestMock.mockReset());

function StatefulNewResourceEditor(
  props: Omit<React.ComponentProps<typeof NewResourceEditor>, "draft" | "onDraftChange">,
) {
  const [draft, setDraft] = React.useState<React.ComponentProps<typeof NewResourceEditor>["draft"]>();
  return <NewResourceEditor {...props} draft={draft} onDraftChange={setDraft} />;
}

describe("NewResourceEditor", () => {
  it("prefills a template and applies it, staying open on success", async () => {
    applyManifestMock.mockResolvedValue({
      applied: true,
      documents: [{ kind: "Service", name: "my-app", applied: true }],
    });
    const onCreated = vi.fn();
    render(
      <StatefulNewResourceEditor
        context="kind-dev"
        namespace="prod"
        initialKind="Service"
        onCreated={onCreated}
      />,
    );

    const editor = (await screen.findByLabelText("New resource YAML")) as HTMLTextAreaElement;
    expect(editor.value).toContain("kind: Service");
    expect(editor.value).toContain("namespace: prod");

    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(applyManifestMock).toHaveBeenCalledWith("kind-dev", editor.value, false, "prod"));
    expect(await screen.findByText(/Applied Service/)).toBeDefined();
    expect(onCreated).toHaveBeenCalled();
    // Editor is still present (tab stays open to create more).
    expect(screen.getByLabelText("New resource YAML")).toBeDefined();
  });

  it("offers to copy the draft", () => {
    // The reader's own draft, from a template or their typing: nothing the
    // cluster handed over is in it. (#656 review)
    render(<StatefulNewResourceEditor context="kind-dev" />);
    expect(screen.getByLabelText("New resource YAML").dataset.copy).toBe("true");
  });

  it("surfaces an apply error", async () => {
    applyManifestMock.mockResolvedValue({ error: "invalid manifest" });
    render(<StatefulNewResourceEditor context="kind-dev" />);
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(await screen.findByText(/invalid manifest/)).toBeDefined();
  });

  it("renders and updates the draft owned by its tab", () => {
    const onDraftChange = vi.fn();
    const draft = {
      template: "Secret",
      yaml: "apiVersion: v1\nkind: Secret\nmetadata:\n  name: hand-written\n",
    };
    render(
      <NewResourceEditor
        context="kind-dev"
        initialKind="Deployment"
        draft={draft}
        onDraftChange={onDraftChange}
      />,
    );

    const editor = screen.getByLabelText("New resource YAML") as HTMLTextAreaElement;
    expect(editor.value).toBe(draft.yaml);
    fireEvent.change(editor, { target: { value: `${draft.yaml}stringData:\n  token: changed\n` } });
    expect(onDraftChange).toHaveBeenCalledWith({
      template: "Secret",
      yaml: `${draft.yaml}stringData:\n  token: changed\n`,
    });
  });
});
