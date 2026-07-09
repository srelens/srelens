import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const { getManifestMock, applyManifestMock } = vi.hoisted(() => ({
  getManifestMock: vi.fn(),
  applyManifestMock: vi.fn(),
}));
vi.mock("../lib/manifest", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/manifest")>();
  return { ...actual, getManifest: getManifestMock, applyManifest: applyManifestMock };
});
// CodeMirror needs real layout (unavailable in jsdom); stand in a controlled
// textarea that mirrors the editor's value/onChange/aria-label contract.
vi.mock("../ui/CodeEditor", () => ({
  CodeEditor: ({
    value,
    onChange,
    ariaLabel,
  }: {
    value: string;
    onChange?: (v: string) => void;
    ariaLabel?: string;
  }) => (
    <textarea aria-label={ariaLabel} value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

import { YamlView } from "./YamlView";

beforeEach(() => {
  getManifestMock.mockReset();
  applyManifestMock.mockReset();
});

describe("YamlView", () => {
  it("renders the fetched manifest in an editor", async () => {
    getManifestMock.mockResolvedValue({ yaml: "apiVersion: v1\nkind: Pod\n" });
    render(<YamlView context="kind-dev" kind="Pod" namespace="default" name="web-1" />);
    await waitFor(() =>
      expect((screen.getByLabelText("Manifest YAML") as HTMLTextAreaElement).value).toContain(
        "kind: Pod",
      ),
    );
    expect(getManifestMock).toHaveBeenCalledWith("kind-dev", "Pod", "default", "web-1", undefined, undefined);
  });

  it("shows a load error", async () => {
    getManifestMock.mockResolvedValue({ error: "not found" });
    render(<YamlView context="kind-dev" kind="Pod" namespace={null} name="x" />);
    await waitFor(() => expect(screen.getByText(/not found/)).toBeDefined());
  });

  it("edits and applies the manifest behind a confirm", async () => {
    getManifestMock.mockResolvedValue({ yaml: "kind: ConfigMap\n" });
    applyManifestMock.mockResolvedValue({
      applied: true,
      documents: [{ kind: "ConfigMap", name: "cm", applied: true }],
    });
    render(<YamlView context="kind-dev" kind="ConfigMap" namespace="default" name="cm" />);

    const textarea = await screen.findByLabelText("Manifest YAML");
    // Apply is disabled until edited.
    expect((screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(textarea, { target: { value: "kind: ConfigMap\ndata:\n  k: v\n" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    // confirm dialog — outside content is aria-hidden, so the dialog's Apply
    // is the only reachable one.
    expect(screen.getByRole("dialog")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() =>
      expect(applyManifestMock).toHaveBeenCalledWith("kind-dev", "kind: ConfigMap\ndata:\n  k: v\n", false),
    );
  });
});
