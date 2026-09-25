import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), listExtensionCatalog: vi.fn(), reviewCatalogExtension: vi.fn(), openExternal: vi.fn() }));
import { listExtensionCatalog, reviewCatalogExtension, openExternal } from "@srelens/core";
import { ExtensionCatalog } from "./ExtensionCatalog";
const entry = { id: "org.srelens.flux", name: "Flux", description: "Flux resources", repository: "https://github.com/srelens/extension-flux", license: "MIT", release: { version: "0.2.0", sha256: "abc", srelensApiVersion: "^0.1", prerelease: true } };
const snapshot = { catalog: { extensions: [entry] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0", "0.2.0"], incompatible: [] };
beforeEach(() => { vi.resetAllMocks(); vi.mocked(listExtensionCatalog).mockResolvedValue(snapshot as any); });
it("browses on demand, searches, and reviews exact verified bytes before any install", async () => {
  const review = vi.fn();
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: '{"name":"Flux","permissions":[]}' });
  render(<ExtensionCatalog onReview={review} installed={[]} />);
  expect(listExtensionCatalog).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Browse catalog"));
  expect(await screen.findByText("Flux")).toBeTruthy();
  expect(screen.getByText(/Preview/)).toBeTruthy();
  expect(screen.getByText(/Host API 0.1.0, 0.2.0/)).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Find an app"), { target: { value: "argo" } });
  expect(screen.getByText("No matching apps.")).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Find an app"), { target: { value: "flux" } });
  fireEvent.click(screen.getByText("Review installation"));
  await waitFor(() => expect(review).toHaveBeenCalledWith('{"name":"Flux","permissions":[]}', undefined));
  expect(reviewCatalogExtension).toHaveBeenCalledWith(entry.id, "abc");
});
it("shows cached refresh failures and keeps incompatible releases disabled", async () => {
  vi.mocked(listExtensionCatalog).mockResolvedValue({ ...snapshot, stale: true, error: "offline", incompatible: [entry.id] } as any);
  render(<ExtensionCatalog onReview={vi.fn()} installed={[]} />);
  fireEvent.click(screen.getByText("Browse catalog"));
  expect(await screen.findByText(/offline/)).toBeTruthy();
  expect((screen.getByText("Review installation") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByText("Refresh catalog"));
  await waitFor(() => expect(listExtensionCatalog).toHaveBeenLastCalledWith(true));
});
it("surfaces download and browser errors without a developer-mode gate", async () => {
  render(<ExtensionCatalog onReview={vi.fn()} installed={[]} />);
  fireEvent.click(screen.getByText("Browse catalog"));
  await screen.findByText("Flux");
  expect((screen.getByText("Review installation") as HTMLButtonElement).disabled).toBe(false);
  vi.mocked(reviewCatalogExtension).mockRejectedValue(new Error("checksum mismatch"));
  fireEvent.click(screen.getByText("Review installation"));
  expect(await screen.findByText("checksum mismatch")).toBeTruthy();
  vi.mocked(openExternal).mockRejectedValue(new Error("browser unavailable"));
  fireEvent.click(screen.getByText("Repository"));
  expect(await screen.findByText("browser unavailable")).toBeTruthy();
});
it("reports a first-load failure with retry, not an empty catalog", async () => {
  vi.mocked(listExtensionCatalog).mockRejectedValueOnce(new Error("offline"));
  render(<ExtensionCatalog onReview={vi.fn()} installed={[]} />);
  fireEvent.click(screen.getByText("Browse catalog"));
  expect(await screen.findByText("offline")).toBeTruthy();
  expect(screen.queryByText("No matching apps.")).toBeNull();
  fireEvent.click(screen.getByText("Browse catalog"));
  expect(await screen.findByText("Flux")).toBeTruthy();
});

it("passes the backend-verified signature into installation review",async()=>{
 const review=vi.fn();
 vi.mocked(reviewCatalogExtension).mockResolvedValue({manifest:'{"name":"Flux","permissions":[]}',signature:[1,2,3]});
 render(<ExtensionCatalog onReview={review} installed={[]} autoLoad/>);
 fireEvent.click(await screen.findByText("Review installation"));
 await waitFor(()=>expect(review).toHaveBeenCalledWith('{"name":"Flux","permissions":[]}',[1,2,3]));
});

/** On the web the catalog is the server's shared copy (#515): a Refresh there reads it, never fetches. */
it("says on the web that the server keeps and fetches the one shared catalog", async () => {
  const shared = /one catalog for everyone who signs in/;
  render(<ExtensionCatalog onReview={vi.fn()} installed={[]} autoLoad />);
  expect(await screen.findByText(shared)).toBeTruthy();
  (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
  try {
    const { unmount } = render(<ExtensionCatalog onReview={vi.fn()} installed={[]} autoLoad />);
    await waitFor(() => expect(screen.getAllByText("Flux")).toHaveLength(2));
    expect(screen.getAllByText(shared)).toHaveLength(1);
    unmount();
  } finally {
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  }
});
