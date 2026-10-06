import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
const host = vi.hoisted(() => ({ calls: [] as any[], answer: {} as unknown, error: "", pending: undefined as Promise<unknown> | undefined }));
const appState = vi.hoisted(() => ({ revision: 3, enabled: true, autoRun: false, stream: false }));
const streamState = vi.hoisted(() => ({ handlers: undefined as any, request: undefined as any, closed: 0, cancelled: 0 }));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  openExtensionView: () => ({ close: async () => { streamState.closed++; }, open: async (request: unknown, handlers: unknown) => { streamState.request = request; streamState.handlers = handlers; return { cancel: async () => { streamState.cancelled++; streamState.handlers.onEnd({ type: "close", reason: "cancelled" }); } }; } }),
}));
vi.mock("@srelens/core/transport", async (original) => ({
  ...(await original<typeof import("@srelens/core/transport")>()),
  invokeCapability: async (id: string, input: unknown) => {
    if (id !== "extensions.callOperation") throw new Error(`unexpected capability ${id}`);
    host.calls.push(input);
    if (host.error) throw new Error(host.error);
    return host.pending ?? host.answer;
  },
}));
vi.mock("../lib/clusters", () => ({
  useContexts: () => [{ name: "demo", stableId: "same-id", key: "config#demo", pinnedId: "srelens-context:config#demo" }],
  useContextsStatus: () => "loaded", useContextsError: () => "",
}));
vi.mock("../extensions/inventoryStore", () => ({
  useExtensions: () => ({ status: "ready", data: { plugins: [{ ...appState,
    manifest: { id: "org.srelens.trivy", name: "Trivy", sidecar: { operations: [{ name: "scan", title: "Scan image", view: { autoRun: appState.autoRun, stream: appState.stream }, inputs: [
      { name: "clusterId", type: "string", required: true }, { name: "image", title: "Image", type: "string", required: true, maxLength: 512 },
    ] }, { name: "findings", title: "Findings", view: { autoRun: true, hidden: true }, inputs: [{ name: "clusterId", type: "string", required: true }, { name: "reportId", type: "string", required: true }, { name: "cursor", type: "string" }] }] } },
  }] } }),
}));
import { ExtensionOperation } from "./ExtensionOperation";
import { screenFor } from "../lib/routes";
const route = "/extension-operation-contexts/config%23demo/org.srelens.trivy/3/scan";
const open = () => render(<ExtensionOperation route={route} ported={[]} onSwitchToClassic={() => {}} onLocked={() => {}} />);
beforeEach(() => { host.calls = []; host.error = ""; host.pending = undefined; host.answer = { state: "completed", source: "app", findings: [
  { id: "CVE-2019-1549", severity: "HIGH", package: "libssl1.1", installedVersion: "1.1.1b-r1", fixedVersion: "1.1.1d-r0" },
] }; appState.revision = 3; appState.enabled = true; appState.autoRun = false; appState.stream = false; streamState.closed = 0; streamState.cancelled = 0; });

it("registers a native screen and renders the declared operation's real result", async () => {
  expect(screenFor(route)).toBe(ExtensionOperation);
  open();
  fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
  fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
  expect(await screen.findByText("CVE-2019-1549")).toBeTruthy();
  expect(screen.getByRole("table").textContent).toContain("1.1.1d-r0");
  expect(host.calls).toEqual([{ id: "org.srelens.trivy", revision: 3, context: "srelens-context:config#demo", operation: "scan", params: { clusterId: "srelens-context:config#demo", image: "alpine:3.9" } }]);
});

it("keeps stream failure and cancellation visible and closes ownership on unmount", async () => {
  appState.stream = true;
  const mounted = open();
  fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
  fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
  await waitFor(() => expect(streamState.request.source).toEqual({ kind: "operation", method: "scan", params: { clusterId: "srelens-context:config#demo", image: "alpine:3.9" } }));
  streamState.handlers.onEnd({ type: "error", code: "source", message: "Database acquisition failed" });
  expect((await screen.findByRole("alert")).textContent).toContain("Database acquisition failed");
  expect(screen.queryByRole("table")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  expect((await screen.findByRole("alert")).textContent).toContain("cancelled");
  expect(streamState.cancelled).toBe(1);
  mounted.unmount();
  expect(streamState.closed).toBe(1);
});

it("shows a failed operation with retry and never presents it as clean", async () => {
  host.error = "Registry access denied";
  open();
  fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
  fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Registry access denied");
  expect(screen.queryByRole("table")).toBeNull();
  host.error = "";
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByText("CVE-2019-1549")).toBeTruthy();
});

it("refuses a tab opened on an app revision that has since changed", async () => {
  appState.revision = 4;
  open();
  expect(await screen.findByText(/updated/)).toBeTruthy();
  expect(screen.queryByLabelText("Image")).toBeNull();
  expect(host.calls).toHaveLength(0);
});

it("does not allow duplicate requests while an operation runs", async () => {
  host.pending = new Promise(() => {});
  open();
  fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
  fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
  await waitFor(() => expect((screen.getByRole("button", { name: "Running…" }) as HTMLButtonElement).disabled).toBe(true));
  expect(screen.queryByRole("table")).toBeNull();
});

it("loads a pinned report automatically, pages findings and opens report links", async () => {
  host.answer = { source: "operator", items: [{ reportId: "report-a", image: "alpine:3.9", severity: "HIGH" }], nextCursor: "page-two" };
  render(<ExtensionOperation route={"/extension-operation-contexts/config%23demo/org.srelens.trivy/3/findings/" + encodeURIComponent('{"reportId":"report-a"}')} ported={[]} onSwitchToClassic={() => {}} onLocked={() => {}} />);
  expect(await screen.findByText("alpine:3.9")).toBeTruthy();
  expect(host.calls[0].params.reportId).toBe("report-a");
  fireEvent.click(screen.getByRole("button", { name: "Next page" }));
  await waitFor(() => expect(host.calls[1].params.cursor).toBe("page-two"));
});

it("keeps report navigation in the first column and lets scope prose wrap", async () => {
 host.answer = { scope: "Only workload templates are inventoried; vulnerability scanning has not completed.", items: [{ reportId: "a".repeat(64), image: "alpine:3.9" }] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 expect(await screen.findByRole("button", { name: "Findings" })).toBeTruthy();
 expect(screen.getAllByRole("columnheader")[0].textContent).toBe("Details");
 expect(screen.getByText((host.answer as { scope: string }).scope).className).toContain("whitespace-normal");
});

it("shows shared cluster identity once above the table so container data stays visible", async () => {
 const clusterId = "srelens-context:config#demo";
 host.answer = { clusterId, items: [
  { clusterId, container: "coredns", image: "registry.k8s.io/coredns/coredns:v1.14.6" },
  { clusterId, container: "kube-proxy", image: "registry.k8s.io/kube-proxy:v1.37.0" },
 ] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 expect(await screen.findByText("coredns")).toBeTruthy();
 expect(screen.queryByRole("columnheader", { name: /Cluster Id/ })).toBeNull();
 expect(screen.getAllByText(clusterId)).toHaveLength(1);
 expect(screen.getByText("kube-proxy")).toBeTruthy();
});

it("keeps a row identity visible when it differs from the result metadata", async () => {
 host.answer = { clusterId: "cluster-a", items: [
  { clusterId: "cluster-a", container: "web" },
  { clusterId: "cluster-b", container: "api" },
 ] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 expect(await screen.findByText("api")).toBeTruthy();
 expect(screen.getByRole("columnheader", { name: /Cluster Id/ })).toBeTruthy();
 fireEvent.change(screen.getByRole("textbox", { name: "Filter results" }), { target: { value: "web" } });
 expect(screen.getByRole("columnheader", { name: /Cluster Id/ })).toBeTruthy();
});

it("keeps technical inventory identity behind details and leads with the image", async () => {
 host.answer = { clusterId: "srelens-context:config#demo", source: "workload templates", scope: "OS and supported language packages", items: [
  { container: "ollama", containerType: "regular", image: "ollama/ollama:latest", kind: "Deployment", name: "ollama-gpu", namespace: "ai-services", resourceVersion: "2885780069", uid: "workload-uid", scope: "OS and supported language packages" },
 ] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 expect(await screen.findByText("ollama/ollama:latest")).toBeTruthy();
 expect(screen.queryByRole("columnheader", { name: /Resource Version/ })).toBeNull();
 expect(screen.queryByRole("columnheader", { name: /Uid/ })).toBeNull();
 expect(screen.getByText("workload-uid").closest("details")?.open).toBe(false);
 expect(screen.getByText("srelens-context:config#demo").closest("details")?.open).toBe(false);
 expect(screen.getAllByRole("columnheader")[0].textContent).toBe("Image");
 expect(screen.queryByRole("columnheader", { name: /Container Type/ })).toBeNull();
 expect(screen.queryByRole("columnheader", { name: /Kind/ })).toBeNull();
 expect(screen.getByText("Deployment").closest("td")).toBe(screen.getByText("ollama-gpu").closest("td"));
 expect(screen.getByText("regular").closest("td")).toBe(screen.getByText("ollama").closest("td"));
});

it("sorts the displayed images when the reader clicks the image column", async () => {
 host.answer = { items: [{ image: "zebra:1" }, { image: "alpine:3" }] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 await screen.findByText("zebra:1");
 fireEvent.click(within(screen.getByRole("columnheader", { name: /Image/ })).getByRole("button"));
 expect(screen.getAllByRole("row")[1].textContent).toContain("alpine:3");
});

it("keeps discovered API names visible beside their unknown or absent status", async () => {
 host.answer = { source: "unknown", bindings: [{ binding: "vulnerability-reports", state: "unknown", reason: "Discovery permission denied" }] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 const binding = await screen.findByText("vulnerability-reports");
 expect(screen.getByRole("columnheader", { name: /Binding/ })).toBeTruthy();
 expect(binding.closest("details")).toBeNull();
 expect(screen.getByRole("table").textContent).toContain("Discovery permission denied");
});
