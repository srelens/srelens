import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const { getManifestMock, applyManifestMock } = vi.hoisted(() => ({
  getManifestMock: vi.fn(),
  applyManifestMock: vi.fn(),
}));
vi.mock("@srelens/core/lib/manifest", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/manifest")>();
  return { ...actual, getManifest: getManifestMock, applyManifest: applyManifestMock };
});
// CodeMirror needs real layout (unavailable in jsdom); stand in a controlled
// textarea that mirrors the editor's value/onChange/aria-label contract.
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
// ManifestEditor gates Apply (fail-closed) on a preflight access check in edit
// mode; stub the hook to "allowed" so these tests exercise the apply flow
// rather than the RBAC gate (covered in ManifestEditor.test.tsx).
vi.mock("@srelens/core/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/access")>();
  return {
    ...actual,
    useAccess: () => ({ allowed: () => true, reason: () => "", known: () => true, loading: false }),
  };
});

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

  it("offers to copy the manifest — except over a Secret, which this view shows in the clear", async () => {
    // Two renders in ONE case, because what is being pinned is that the answer
    // DEPENDS on the kind. Split in two, the Secret half passes against a
    // version that never asks for a copy at all, and the Pod half against one
    // that always does; neither alone says the view discriminates. Nor would a
    // sentinel for an omitted prop help — `ManifestEditor` defaults `copy` to
    // `false` before it forwards, so an omitted prop and a declined one reach
    // the editor identically by construction. (#656 review)
    //
    // The rule itself: this view loads through `getManifest`, which redacts
    // nothing — unlike the new design's pane (`redactSecretManifest`) and
    // unlike the Edit tab (`loadEditableManifest`, which routes a Secret
    // through the consent-gated `getSecret`). A one-click copy of unredacted
    // Secret material is not an affordance to add on top of that gap; the gap
    // itself is #659.
    getManifestMock.mockResolvedValue({ yaml: "kind: Pod" });
    const pod = render(<YamlView context="kind-dev" kind="Pod" namespace="default" name="web-1" />);
    const forPod = (await pod.findByLabelText("Manifest YAML")).dataset.copy;
    pod.unmount();

    getManifestMock.mockResolvedValue({ yaml: "kind: Secret" });
    const secret = render(<YamlView context="kind-dev" kind="Secret" namespace="default" name="api" />);
    const forSecret = (await secret.findByLabelText("Manifest YAML")).dataset.copy;

    expect(forPod).toBe("true");
    expect(forSecret).toBe("false");
    expect(forPod).not.toBe(forSecret);
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
      expect(applyManifestMock).toHaveBeenCalledWith("kind-dev", "kind: ConfigMap\ndata:\n  k: v\n", false, "default"),
    );
  });
});
