import { render, screen, waitFor, act } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  readFreelensExtension: vi.fn(),
}));
import { readFreelensExtension } from "@srelens/core";
import { FreelensView } from "./FreelensView";
const plugin = {
  manifest: { id: "org.freelensapp.fluxcd" },
  revision: 7,
} as any;
beforeEach(() => {
  vi.resetAllMocks();
  URL.createObjectURL = vi.fn(() => "blob:test");
  URL.revokeObjectURL = vi.fn();
  vi.mocked(readFreelensExtension).mockResolvedValue({
    source: "",
    crds: [{ kind: "GitRepository" }],
    namespaces: ["flux-system"],
  });
});
it("runs an opaque-origin frame and rejects foreign messages and injected scope", async () => {
  const { unmount } = render(
    <FreelensView plugin={plugin} context="staging" />,
  );
  const iframe = (await screen.findByTitle(
    "Freelens FluxCD",
  )) as HTMLIFrameElement;
  expect(iframe.getAttribute("sandbox")).toBe("allow-scripts");
  await act(async () =>
    window.dispatchEvent(
      new MessageEvent("message", {
        source: window,
        origin: "null",
        data: { type: "read", sequence: 1, request: { operation: "events" } },
      }),
    ),
  );
  expect(readFreelensExtension).toHaveBeenCalledTimes(1);
  await act(async () =>
    window.dispatchEvent(
      new MessageEvent("message", {
        source: iframe.contentWindow,
        origin: "null",
        data: {
          type: "read",
          sequence: 2,
          request: {
            operation: "resource",
            group: "source.toolkit.fluxcd.io",
            version: "v1",
            plural: "gitrepositories",
            kind: "GitRepository",
            context: "production",
            revision: 99,
          },
        },
      }),
    ),
  );
  await waitFor(() =>
    expect(readFreelensExtension).toHaveBeenLastCalledWith(
      plugin.manifest.id,
      7,
      "staging",
      {
        operation: "resource",
        group: "source.toolkit.fluxcd.io",
        version: "v1",
        plural: "gitrepositories",
        kind: "GitRepository",
      },
    ),
  );
  unmount();
  expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:test");
});
it("does not run a package when discovery fails", async () => {
  vi.mocked(readFreelensExtension).mockRejectedValue(
    new Error("CRD discovery forbidden"),
  );
  render(<FreelensView plugin={plugin} context="staging" />);
  expect((await screen.findByRole("alert")).textContent).toContain(
    "CRD discovery forbidden",
  );
  expect(screen.queryByTitle("Freelens FluxCD")).toBeNull();
});
it("only reports absence after successful discovery and never connects without a context", async () => {
  vi.mocked(readFreelensExtension).mockResolvedValue({
    source: "",
    crds: [],
    namespaces: [],
  });
  const { rerender } = render(<FreelensView plugin={plugin} context="" />);
  expect(readFreelensExtension).not.toHaveBeenCalled();
  rerender(<FreelensView plugin={plugin} context="staging" />);
  expect(
    await screen.findByText(
      "This cluster serves no Flux custom resource APIs.",
    ),
  ).toBeTruthy();
  expect(URL.createObjectURL).not.toHaveBeenCalled();
});
