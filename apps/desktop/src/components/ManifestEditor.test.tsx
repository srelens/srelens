import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React, { useState } from "react";

const { applyManifestMock, diffManifestMock, notifyMock } = vi.hoisted(() => ({
  applyManifestMock: vi.fn(),
  diffManifestMock: vi.fn(),
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn(), updateAvailable: vi.fn() },
}));
vi.mock("@srelens/core/lib/manifest", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/manifest")>()),
  applyManifest: applyManifestMock,
  diffManifest: diffManifestMock,
  validateManifest: vi.fn().mockResolvedValue({ valid: true }),
}));
vi.mock("@srelens/core/lib/notify", () => ({ notify: notifyMock }));
vi.mock("@srelens/core/lib/schema", () => ({ openApiSchema: vi.fn().mockResolvedValue({ error: "n/a" }) }));
vi.mock("../ui/CodeEditor", () => ({
  CodeEditor: ({ value, onChange, ariaLabel }: { value: string; onChange?: (v: string) => void; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));
vi.mock("@srelens/core/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/access")>();
  return { ...actual, useAccess: vi.fn() };
});
import { useAccess } from "@srelens/core/react";

import { ManifestEditor } from "./ManifestEditor";

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM properties instead of `toBeDisabled` sugar.
function isDisabled(el: HTMLElement): boolean {
  return (el as HTMLButtonElement).disabled;
}

// A disabled control explains itself through a Radix tooltip, not a native
// title (#376): hover its trigger — the wrapper around a disabled button,
// which is what still receives pointer events — and read the tooltip.
async function tooltipOf(el: HTMLElement): Promise<string | null> {
  const trigger = el.closest<HTMLElement>('[data-slot="tooltip-trigger"]') ?? el;
  await userEvent.hover(trigger);
  return (await screen.findByRole("tooltip")).textContent;
}

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
  // Default: allowed, so pre-existing behavioural tests (written before RBAC
  // gating existed) keep exercising an enabled Apply button.
  vi.mocked(useAccess).mockReturnValue({
    allowed: () => true,
    reason: () => "",
    known: () => true,
    loading: false,
  });
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

  it("shows a stale badge when the live resourceVersion differs, without opening the Changes panel", async () => {
    diffManifestMock.mockResolvedValue({
      documents: [
        { kind: "Deployment", name: "web", namespace: null, exists: true, changed: true, rows: [], currentResourceVersion: "9" },
      ],
    });
    const { container } = render(
      <ManifestEditor
        context="ctx"
        yaml={'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  resourceVersion: "1"\n'}
        onYamlChange={() => {}}
        fill
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    // The badge must appear WITHOUT clicking "Changes" first — the diff runs
    // in the background so a concurrently-modified resource is flagged even
    // while the panel is collapsed.
    expect(await screen.findByText(/changed elsewhere/i, {}, { timeout: 2000 })).toBeDefined();
    // The panel itself stays collapsed until the user opts in.
    expect(container.querySelector(".fl-changes-panel")).toBeNull();
    expect(screen.getByRole("button", { name: "Changes" })).toBeDefined();
  });

  it("surfaces a hard error even when another document conflicts (first response)", async () => {
    applyManifestMock.mockResolvedValue({
      applied: false,
      documents: [
        {
          kind: "ConfigMap",
          name: "cfg",
          applied: false,
          conflict: { managers: ["kubectl"], fields: [".data.x"], message: "conflict on cfg" },
        },
        { kind: "Deployment", name: "web", applied: false, error: "deploy blew up" },
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
    // The conflict banner appears for the conflicting document…
    const banner = await screen.findByRole("alert");
    expect(within(banner).getByText(/ConfigMap\/cfg/)).toBeDefined();
    // …AND the OTHER document's hard error is surfaced on the same response,
    // not swallowed until after the user clicks Force.
    await waitFor(() => expect(notifyMock.error).toHaveBeenCalled());
    expect(screen.getByText(/Failed to apply/)).toBeDefined();
  });

  it("does not flag the user's own apply as changed elsewhere (rebaselines rv after apply)", async () => {
    let liveRv = "1";
    let diffName = "web-before";
    diffManifestMock.mockImplementation(async () => ({
      documents: [
        {
          kind: "Deployment",
          name: diffName,
          namespace: null,
          exists: true,
          changed: true,
          rows: [{ tag: "insert", left: null, right: "x" }],
          currentResourceVersion: liveRv,
        },
      ],
    }));
    applyManifestMock.mockImplementation(async () => {
      liveRv = "9"; // a successful apply bumps the live resourceVersion
      return { applied: true, documents: [{ kind: "Deployment", name: "web", applied: true }] };
    });

    function Harness() {
      const [yaml, setYaml] = useState(
        'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  resourceVersion: "1"\n',
      );
      return <ManifestEditor context="ctx" yaml={yaml} onYamlChange={setYaml} fill />;
    }
    render(<Harness />);

    // Open the Changes panel so each diff resolution is observable via DiffView.
    fireEvent.click(screen.getByRole("button", { name: "Changes" }));
    // The first diff sees live rv "1" (matches the loaded manifest) → no badge.
    expect(await screen.findByText("Deployment/web-before", {}, { timeout: 3000 })).toBeDefined();
    expect(screen.queryByText(/changed elsewhere/i)).toBeNull();

    // Apply — this bumps the live rv to "9" for the user's OWN change.
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(applyManifestMock).toHaveBeenCalled());

    // A later edit triggers a fresh diff that now reports live rv "9".
    diffName = "web-after";
    fireEvent.change(screen.getByLabelText("Manifest YAML"), {
      target: {
        value: 'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  resourceVersion: "1"\n  labels:\n    a: b\n',
      },
    });
    expect(await screen.findByText("Deployment/web-after", {}, { timeout: 3000 })).toBeDefined();

    // The stale badge must NOT appear: the rv bump was our own apply, so the
    // baseline was rebased to "9" rather than flagged as an external change.
    expect(screen.queryByText(/changed elsewhere/i)).toBeNull();
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

  it("disables Apply when the user can't patch the edited resource", async () => {
    vi.mocked(useAccess).mockReturnValue({ allowed: () => false, reason: () => "", known: () => true, loading: false });
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\n"}
        onYamlChange={() => {}}
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    const btn = screen.getByRole("button", { name: /apply/i });
    expect(isDisabled(btn)).toBe(true);
    expect(await tooltipOf(btn)).toBe("You don't have permission to patch deployments in prod");
  });

  it("edit mode: disables Apply while the access check is still loading (fail-closed, no title yet)", async () => {
    // Check not yet resolved: known() false. Fail-closed ⇒ Apply is disabled,
    // but the permission title isn't shown until the check resolves as denied.
    vi.mocked(useAccess).mockReturnValue({ allowed: () => false, reason: () => "", known: () => false, loading: true });
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\n"}
        onYamlChange={() => {}}
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    const btn = screen.getByRole("button", { name: /apply/i });
    expect(isDisabled(btn)).toBe(true);
    expect(btn.getAttribute("title")).toBeNull();
  });

  it("new-resource mode (no confirm): does not access-gate Apply even when denied", async () => {
    // The New-resource tab creates (verb `create`), not patches — so a user who
    // can't patch must still be able to Apply. Only edit mode (confirm set) gates.
    vi.mocked(useAccess).mockReturnValue({ allowed: () => false, reason: () => "", known: () => true, loading: false });
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\n"}
        onYamlChange={() => {}}
      />,
    );
    const btn = screen.getByRole("button", { name: /apply/i });
    expect(isDisabled(btn)).toBe(false);
    expect(btn.getAttribute("title")).toBeNull();
  });

  it("keeps Apply enabled for an unknown/CRD kind", async () => {
    vi.mocked(useAccess).mockReturnValue({ allowed: () => false, reason: () => "", known: () => true, loading: false });
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: example.com/v1\nkind: Widget\nmetadata:\n  name: w\n"}
        onYamlChange={() => {}}
      />,
    );
    expect(isDisabled(screen.getByRole("button", { name: /apply/i }))).toBe(false);
  });

  it("edit mode: disables Apply for a MULTI-document manifest whose first doc is a denied Deployment", async () => {
    // `parse()` throws on multi-document YAML — the identity must instead be
    // derived from the FIRST document via `parseAllDocuments`, or Apply is left
    // ungated (fail-open). With useAccess denying the Deployment, Apply must be
    // DISABLED with the permission title.
    vi.mocked(useAccess).mockReturnValue({ allowed: () => false, reason: () => "", known: () => true, loading: false });
    render(
      <ManifestEditor
        context="ctx"
        yaml={
          "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n  namespace: prod\n"
        }
        onYamlChange={() => {}}
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    const btn = screen.getByRole("button", { name: /apply/i });
    expect(isDisabled(btn)).toBe(true);
    expect(await tooltipOf(btn)).toBe("You don't have permission to patch deployments in prod");
  });

  it("edit mode: does NOT gate Apply when the manifest declares no namespace (avoids a wrong-scope false-disable)", async () => {
    // A namespaced resource whose YAML omits metadata.namespace relies on the
    // context's default namespace. Building the SSAR with an empty namespace
    // makes it cluster-scoped and can FALSE-disable Apply. With no declared
    // namespace we skip gating (server still enforces) — Apply stays ENABLED
    // even though useAccess would deny.
    vi.mocked(useAccess).mockReturnValue({ allowed: () => false, reason: () => "", known: () => true, loading: false });
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n"}
        onYamlChange={() => {}}
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    const btn = screen.getByRole("button", { name: /apply/i });
    expect(isDisabled(btn)).toBe(false);
    expect(btn.getAttribute("title")).toBeNull();
  });

  it("names the authorized action in the edit-apply confirm dialog", async () => {
    render(
      <ManifestEditor
        context="ctx"
        yaml={"apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\n"}
        onYamlChange={() => {}}
        confirm={{ kind: "Deployment", name: "web" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(screen.getByText("Apply manifest?")).toBeDefined());
    expect(screen.getByText(/This authorizes patch on deployments in prod/)).toBeDefined();
  });
});
