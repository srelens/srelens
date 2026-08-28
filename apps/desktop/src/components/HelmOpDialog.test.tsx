import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { HelmOpDialog } from "./HelmOpDialog";
import type { HelmChartRef } from "@srelens/core";

const helmSearchRepoMock = vi.fn(
  async (_context: string, _chart: string): Promise<{ entries?: HelmChartRef[]; error?: string }> => ({ entries: [] }),
);

vi.mock("@srelens/core/lib/helm", async (orig) => {
  const actual = await orig<typeof import("@srelens/core/lib/helm")>();
  return {
    ...actual,
    helmTemplate: vi.fn(async () => ({ output: "apiVersion: v1\nkind: ConfigMap\ndata:\n  x: '2'" })),
    helmSearchRepo: (context: string, chart: string) => helmSearchRepoMock(context, chart),
  };
});

// CodeMirror needs real layout, so jsdom can't run it — stand in a controlled textarea.
vi.mock("../ui/CodeEditor", () => ({
  CodeEditor: ({ value, onChange, ariaLabel }: { value: string; onChange?: (v: string) => void; ariaLabel?: string }) => (
    <textarea aria-label={ariaLabel} value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

const release = {
  name: "web",
  namespace: "apps",
  chart: "nginx",
  chartVersion: "18.0.0",
  valuesYaml: "replicaCount: 1",
  manifest: "apiVersion: v1\nkind: ConfigMap\ndata:\n  x: '1'",
};

beforeEach(() => {
  helmSearchRepoMock.mockReset();
  helmSearchRepoMock.mockResolvedValue({ entries: [] });
});

describe("HelmOpDialog (upgrade)", () => {
  it("preloads current values and previews a diff", async () => {
    render(<HelmOpDialog context="ctx" mode="upgrade" release={release} onRun={vi.fn()} onClose={vi.fn()} />);
    const editor = (await screen.findByLabelText("Values YAML")) as HTMLTextAreaElement;
    expect(editor.value).toBe("replicaCount: 1");

    fireEvent.click(screen.getByRole("button", { name: /Preview/i }));
    await waitFor(() => expect(screen.getAllByText(/kind: ConfigMap/).length).toBeGreaterThan(0));
  });

  it("calls onRun with edited values and built args when no repo match is resolved", async () => {
    const onRun = vi.fn();
    render(<HelmOpDialog context="ctx" mode="upgrade" release={release} onRun={onRun} onClose={vi.fn()} />);
    await waitFor(() => expect(helmSearchRepoMock).toHaveBeenCalledWith("ctx", "nginx"));
    fireEvent.change(await screen.findByLabelText("Values YAML"), { target: { value: "replicaCount: 2" } });
    fireEvent.click(screen.getByRole("button", { name: /^Upgrade$/ }));
    expect(onRun).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "web",
        chart: "nginx",
        namespace: "apps",
        values: "replicaCount: 2",
        helmArgs: expect.arrayContaining(["upgrade", "web", "nginx", "--namespace", "apps"]),
      }),
    );
  });

  it("resolves the chart ref from configured repos and includes it plus the selected version in helmArgs", async () => {
    helmSearchRepoMock.mockResolvedValue({
      entries: [
        { name: "bitnami/cert-manager", version: "1.15.0", appVersion: "1.15.0", description: "d" },
        { name: "bitnami/cert-manager", version: "1.14.0", appVersion: "1.14.0", description: "d" },
      ],
    });
    const certRelease = { ...release, chart: "cert-manager", chartVersion: "1.14.0" };
    const onRun = vi.fn();
    render(<HelmOpDialog context="ctx" mode="upgrade" release={certRelease} onRun={onRun} onClose={vi.fn()} />);

    await waitFor(() => expect(helmSearchRepoMock).toHaveBeenCalledWith("ctx", "cert-manager"));
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Chart reference" }).textContent).toContain("bitnami/cert-manager"),
    );

    fireEvent.click(screen.getByRole("button", { name: /^Upgrade$/ }));
    expect(onRun).toHaveBeenCalledWith(
      expect.objectContaining({
        chart: "bitnami/cert-manager",
        helmArgs: expect.arrayContaining(["upgrade", "web", "bitnami/cert-manager", "--version", "1.14.0"]),
      }),
    );
    // The bare chart name from the release must never reach helm directly.
    expect(onRun.mock.calls[0][0].helmArgs).not.toContain("cert-manager");
  });

  it("keeps the free-text chart input and hint when no repo match is found", async () => {
    const certRelease = { ...release, chart: "cert-manager", chartVersion: "1.14.0" };
    render(<HelmOpDialog context="ctx" mode="upgrade" release={certRelease} onRun={vi.fn()} onClose={vi.fn()} />);
    await waitFor(() => expect(helmSearchRepoMock).toHaveBeenCalledWith("ctx", "cert-manager"));

    const input = screen.getByLabelText("Chart reference") as HTMLInputElement;
    expect(input.tagName).toBe("INPUT");
    expect(input.value).toBe("cert-manager");
    expect(screen.getByText(/Helm doesn't record the chart's source/)).toBeTruthy();
  });

  it("defaults the version to the release's current chart version when available", async () => {
    helmSearchRepoMock.mockResolvedValue({
      entries: [
        { name: "bitnami/cert-manager", version: "1.15.0", appVersion: "1.15.0", description: "d" },
        { name: "bitnami/cert-manager", version: "1.14.0", appVersion: "1.14.0", description: "d" },
      ],
    });
    const certRelease = { ...release, chart: "cert-manager", chartVersion: "1.14.0" };
    render(<HelmOpDialog context="ctx" mode="upgrade" release={certRelease} onRun={vi.fn()} onClose={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Version" }).textContent).toContain("1.14.0 (current)"),
    );
  });
});

describe("HelmOpDialog (install)", () => {
  it("blocks confirming with an empty chart and shows a required-fields error", () => {
    const onRun = vi.fn();
    render(<HelmOpDialog context="ctx" mode="install" onRun={onRun} onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Release name"), { target: { value: "web" } });
    fireEvent.click(screen.getByRole("button", { name: /^Install$/ }));
    expect(onRun).not.toHaveBeenCalled();
    expect(screen.getByText(/Release name and chart reference are required/)).toBeTruthy();
  });

  it("renders visible labels for Release name, Chart reference and Namespace", () => {
    render(<HelmOpDialog context="ctx" mode="install" onRun={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByText("Release name")).toBeDefined();
    expect(screen.getByText("Chart reference")).toBeDefined();
    expect(screen.getByText("Namespace")).toBeDefined();
  });

  it("does not search repos and keeps the free-text chart input", () => {
    render(<HelmOpDialog context="ctx" mode="install" onRun={vi.fn()} onClose={vi.fn()} />);
    expect(helmSearchRepoMock).not.toHaveBeenCalled();
    expect((screen.getByLabelText("Chart reference") as HTMLInputElement).tagName).toBe("INPUT");
  });
});
