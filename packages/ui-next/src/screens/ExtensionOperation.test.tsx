import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
const host = vi.hoisted(() => ({ calls: [] as any[], answer: {} as unknown, error: "", pending: undefined as Promise<unknown> | undefined }));
const appState = vi.hoisted(() => ({ revision: 3, enabled: true, autoRun: false, stream: false, namespace: false, boolean: false, inventoryError: false, contextError: false, reloads: 0, refreshes: 0 }));
const streamState = vi.hoisted(() => ({ handlers: undefined as any, request: undefined as any, closed: 0, cancelled: 0, pending: undefined as Promise<any> | undefined }));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  listContexts: async () => { appState.refreshes++; return {contexts:[]}; },
  openExtensionView: () => ({ close: async () => { streamState.closed++; }, open: async (request: unknown, handlers: unknown) => { streamState.request = request; streamState.handlers = handlers; if (streamState.pending) return await streamState.pending; return { cancel: async () => { streamState.cancelled++; streamState.handlers.onEnd({ type: "close", reason: "cancelled" }); } }; } }),
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
vi.mock("@srelens/core/react", () => ({ useNamespaceOptions: () => ({ namespaces: ["team", "default"], scope: null, error: "" }) }));
vi.mock("../lib/clusters", () => ({
  useContexts: () => appState.contextError ? [] : [{ name: "demo", stableId: "same-id", key: "config#demo", pinnedId: "srelens-context:config#demo" }],
  useContextsStatus: () => appState.contextError ? "failed" : "loaded", useContextsError: () => "Kubeconfig read denied", getContexts: () => [], getKubeconfigFiles: () => [], setContexts: () => { appState.contextError = false; },
}));
vi.mock("../extensions/inventoryStore", () => ({
  useExtensions: () => ({ status: appState.inventoryError ? "error" : "ready", error: "Inventory read denied", reload: () => { appState.reloads++; appState.inventoryError = false; }, data: { plugins: [{ ...appState,
    manifest: { id: "org.srelens.trivy", name: "Trivy", sidecar: { operations: [{ name: "scan", title: "Scan image", view: { autoRun: appState.autoRun, stream: appState.stream }, inputs: [
      { name: "clusterId", type: "string", required: true }, ...(appState.namespace ? [{ name: "namespace", title: "Namespace", type: "string", required: true }] : []), { name: "image", title: "Image", type: "string", required: true, maxLength: 512 }, ...(appState.boolean ? [{name:"includeFixed",title:"Include fixed",type:"boolean",required:true}] : []),
    ] }, { name: "findings", title: "Findings", view: { autoRun: true, hidden: true }, inputs: [{ name: "clusterId", type: "string", required: true }, { name: "reportId", type: "string", required: true }, { name: "cursor", type: "string" }] }, {name:"scan-namespace",title:"Scan namespace",view:{stream:true},inputs:[{name:"clusterId",type:"string",required:true},{name:"namespace",type:"string",required:true}]}, {name:"list-images",title:"Images",view:{autoRun:true},inputs:[{name:"clusterId",type:"string",required:true},{name:"namespace",type:"string"},{name:"cursor",type:"string"}]}] } },
  }] } }),
}));
if (!("ResizeObserver" in globalThis)) {
 (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
}
HTMLElement.prototype.scrollIntoView ??= () => {};
import { ExtensionOperation } from "./ExtensionOperation";
import { screenFor } from "../lib/routes";
const route = "/extension-operation-contexts/config%23demo/org.srelens.trivy/3/scan";
const open = () => render(<ExtensionOperation route={route} ported={[]} onSwitchToClassic={() => {}} onLocked={() => {}} />);
beforeEach(() => { host.calls = []; host.error = ""; host.pending = undefined; host.answer = { state: "completed", source: "app", findings: [
  { id: "CVE-2019-1549", severity: "HIGH", package: "libssl1.1", installedVersion: "1.1.1b-r1", fixedVersion: "1.1.1d-r0" },
] }; appState.revision = 3; appState.enabled = true; appState.autoRun = false; appState.stream = false; appState.namespace = false; appState.boolean = false; appState.inventoryError = false; appState.contextError = false; appState.reloads = 0; appState.refreshes = 0; streamState.request = undefined; streamState.pending = undefined; streamState.closed = 0; streamState.cancelled = 0; });

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

it("keeps report navigation available and lets scope prose wrap", async () => {
 host.answer = { scope: "Only workload templates are inventoried; vulnerability scanning has not completed.", items: [{ reportId: "a".repeat(64), image: "alpine:3.9" }] };
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.9" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 expect(await screen.findByRole("button", { name: "Findings" })).toBeTruthy();
 expect(screen.getAllByRole("columnheader")[0].textContent).toBe("Image");
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


it("offers a stream row action without starting a scan", async () => {
 appState.stream = true;
 host.answer = { items: [{ image: "alpine:3.10", reportId: "report-a" }] };
 render(<ExtensionOperation route={"/extension-operation-contexts/config%23demo/org.srelens.trivy/3/findings/" + encodeURIComponent('{"reportId":"report-a"}')} ported={[]} onSwitchToClassic={() => {}} onLocked={() => {}} />);
 expect(await screen.findByRole("button", { name: "Scan image" })).toBeTruthy();
 expect(streamState.request).toBeUndefined();
});

it("shows prefilled scan inputs, requires one searchable namespace and waits for Run", async () => {
 appState.stream = true; appState.namespace = true;
 render(<ExtensionOperation route={route + "/" + encodeURIComponent('{"image":"alpine:3.10","namespace":"team"}')} ported={[]} onSwitchToClassic={() => {}} onLocked={() => {}} />);
 expect((await screen.findByLabelText("Image") as HTMLInputElement).value).toBe("alpine:3.10");
 const picker = screen.getByRole("combobox", { name: "Namespace" });
 fireEvent.click(picker);
 expect(screen.queryByText("All namespaces")).toBeNull();
 expect(screen.getByPlaceholderText("Find a namespace…")).toBeTruthy();
 expect(streamState.request).toBeUndefined();
});

it("can cancel while the host is still opening a scan stream", async () => {
 appState.stream = true; streamState.pending = new Promise(() => {});
 open();
 fireEvent.change(await screen.findByLabelText("Image"), { target: { value: "alpine:3.10" } });
 fireEvent.click(screen.getByRole("button", { name: "Scan image" }));
 fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
 expect((await screen.findByRole("alert")).textContent).toContain("cancelled");
 expect(streamState.closed).toBe(1);
});

it("keeps partial report data visible with an actionable warning", async () => {
 host.answer = { items: [{ reportId: "retained", image: "alpine:3.10" }], warnings: ["sbom-reports discovery failed: permission denied"] };
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 expect((await screen.findByRole("alert")).textContent).toContain("permission denied");
 expect(screen.getByText("alpine:3.10")).toBeTruthy();
});

it("keeps short coverage labels compact above the findings table", async () => {
 host.answer={coverage:["image vulnerabilities","namespace configuration"],items:[{reportId:"saved",findings:17}]};
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 await screen.findByText("image vulnerabilities");expect(screen.getAllByRole("table")).toHaveLength(1);
});

it("renders nested report totals as badges and folds scanner provenance", async () => {
 const scannerImage="aquasec/trivy@sha256:"+"a".repeat(64);
 host.answer={metadata:{summary:{CRITICAL:1,HIGH:3},scannerImage},items:[{id:"CVE-one"}]};
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 expect(await screen.findByText(/Critical 1/i)).toBeTruthy();
 expect(screen.getByText(scannerImage).closest("details")?.open).toBe(false);
});

it("leads report rows with their subject, visible severity totals and readable scan times", async () => {
 host.answer={totalReports:1,items:[{reportId:"report-a",subject:"payments",namespace:"production",category:"namespace",source:"app",freshness:"current",engineVersion:"0.75.0",reportedAt:"2026-10-06T10:04:53.061529Z",findings:31,summary:{CRITICAL:2,HIGH:5,LOW:24},image:""}]};
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 await screen.findByRole("table");
 expect(screen.getAllByRole("columnheader")[0].textContent).toBe("Report");
 expect(screen.queryByRole("columnheader",{name:/Engine Version/})).toBeNull();
 expect(screen.queryByRole("columnheader",{name:/Category/})).toBeNull();
 const row=screen.getAllByRole("row")[1];
 expect(within(row).getByText(/Critical 2/i)).toBeTruthy();
 expect(row.textContent).not.toContain("061529");
 expect(within(row).getByRole("button",{name:"Findings"})).toBeTruthy();
 expect(within(row).queryByRole("button",{name:"Scan image"})).toBeNull();
});

it("filters reports by source without combining retained scan severity counts", async () => {
 host.answer={items:[{reportId:"saved-a",subject:"payments",category:"namespace",source:"app",findings:2,summary:{HIGH:2}},{reportId:"operator-b",subject:"web",category:"vulnerability",source:"operator",findings:7,summary:{HIGH:7}}]};
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 fireEvent.click(await screen.findByRole("button",{name:/Operator reports/}));
 expect(screen.getAllByRole("row")).toHaveLength(2);
 expect(screen.getByRole("table").textContent).toContain("web");
 expect(screen.getByRole("table").textContent).not.toContain("payments");
 expect(screen.queryByText(/High 9/i)).toBeNull();
});

it("keeps a completed scan's terminal status visible beside its report action", async () => {
 host.answer={state:"completed",items:[{reportId:"scan-a",subject:"payments",category:"namespace",source:"app",findings:2,summary:{HIGH:2}}]};
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 expect(await screen.findByText("Completed")).toBeTruthy();
 expect(screen.getByRole("button",{name:"Findings"})).toBeTruthy();
});

it("leads paged image inventories with image identity and the correct primary scan action", async () => {
 host.answer={items:[{image:"alpine:3.10",namespace:"team",name:"web",kind:"Deployment"}],nextCursor:"page-two"};
 render(<ExtensionOperation route="/extension-operation-contexts/config%23demo/org.srelens.trivy/3/list-images" ported={[]} onSwitchToClassic={()=>{}} onLocked={()=>{}}/>);
 await screen.findByRole("table");
 expect(screen.getAllByRole("columnheader")[0].textContent).toBe("Image");
 expect(screen.getAllByRole("columnheader").at(-1)?.textContent).toBe("Actions");
 const row=screen.getAllByRole("row")[1];
 expect(within(row).getByRole("button",{name:"Scan image"})).toBeTruthy();
 expect(within(row).queryByRole("button",{name:"Scan namespace"})).toBeNull();
 fireEvent.click(screen.getByRole("button",{name:"Next page"}));
 await waitFor(()=>expect(host.calls.at(-1).params.cursor).toBe("page-two"));
});

it("restarts paging cleanly after an expired page", async () => {
 host.answer={items:[{image:"alpine:3.10"}],nextCursor:"page-two"};
 render(<ExtensionOperation route="/extension-operation-contexts/config%23demo/org.srelens.trivy/3/list-images" ported={[]} onSwitchToClassic={()=>{}} onLocked={()=>{}}/>);
 await screen.findByRole("table");host.error="Continuation expired; refresh Images";
 fireEvent.click(screen.getByRole("button",{name:"Next page"}));
 await screen.findByRole("alert");host.error="";
 fireEvent.click(screen.getByRole("button",{name:"Start from first page"}));
 await screen.findByRole("table");
 expect(host.calls.at(-1).params.cursor).toBeUndefined();
 expect((screen.getByRole("button",{name:"Previous page"}) as HTMLButtonElement).disabled).toBe(true);
});

it("retries the current page after a transient inventory failure", async () => {
 host.answer={items:[{image:"alpine:3.10"}],nextCursor:"page-two"};
 render(<ExtensionOperation route="/extension-operation-contexts/config%23demo/org.srelens.trivy/3/list-images" ported={[]} onSwitchToClassic={()=>{}} onLocked={()=>{}}/>);
 await screen.findByRole("table");host.error="Connection timed out";
 fireEvent.click(screen.getByRole("button",{name:"Next page"}));
 await screen.findByRole("alert");host.error="";
 fireEvent.click(screen.getByRole("button",{name:"Retry"}));
 await screen.findByRole("table");
 expect(host.calls.at(-1).params.cursor).toBe("page-two");
 expect((screen.getByRole("button",{name:"Previous page"}) as HTMLButtonElement).disabled).toBe(false);
});

it("distinguishes Operator resources sharing an image and renders Operator severity keys", async () => {
 host.answer={totalReports:2,items:["web","api"].map(subject=>({reportId:subject,subject,namespace:"team",category:"vulnerability",source:"operator",image:"alpine:3.10",findings:8,summary:{lowCount:1,criticalCount:2,unknownCount:1,highCount:4}}))};
 open();fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.10"}});fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
 await screen.findByRole("table");
 const rows=screen.getAllByRole("row").slice(1);
 expect(within(rows[0]).getByText("web")).toBeTruthy();
 expect(within(rows[1]).getByText("api")).toBeTruthy();
 for(const row of rows){
  expect(within(row).getByText("alpine:3.10")).toBeTruthy();
  const critical=within(row).getByText("Critical 2");
  expect(critical.getAttribute("data-tone")).toBe("sev");
  expect(row.textContent?.indexOf("Critical 2")).toBeLessThan(row.textContent?.indexOf("High 4") ?? 0);
  expect(within(row).getByText("Low 1")).toBeTruthy();
  expect(within(row).getByText("Unknown 1")).toBeTruthy();
 }
});

it("submits a required boolean's visible unchecked choice on the first run", async () => {
  appState.boolean = true;
  open();
  expect((await screen.findByLabelText("Include fixed") as HTMLInputElement).checked).toBe(false);
  fireEvent.change(screen.getByLabelText("Image"), {target:{value:"alpine:3.9"}});
  fireEvent.click(screen.getByRole("button", {name:"Scan image"}));
  await screen.findByText("CVE-2019-1549");
  expect(host.calls[0].params.includeFixed).toBe(false);
});

it("preserves a route-provided true boolean when initializing the form", async () => {
  appState.boolean = true;
  render(<ExtensionOperation route={route + "/" + encodeURIComponent(JSON.stringify({image:"alpine:3.9",includeFixed:true}))} ported={[]} onSwitchToClassic={() => {}} onLocked={() => {}} />);
  fireEvent.click(await screen.findByRole("button", {name:"Scan image"}));
  await screen.findByText("CVE-2019-1549");
  expect(host.calls[0].params.includeFixed).toBe(true);
});

it("renders malformed report collections as generic results with independent row identity", async () => {
  host.answer = {totalReports:2,items:[{image:"alpine:a"},{image:"alpine:b"}]};
  open(); fireEvent.change(await screen.findByLabelText("Image"),{target:{value:"alpine:3.9"}});
  fireEvent.click(screen.getByRole("button",{name:"Scan image"}));
  await screen.findByText("alpine:a");
  expect(screen.getByRole("table").textContent).toContain("alpine:b");
  expect(screen.getByRole("columnheader",{name:/Image/})).toBeTruthy();
  expect(screen.queryByText("Undefined report")).toBeNull();
});

it("offers retry for failed inventory and context loads", async () => {
  appState.inventoryError = true;
  let mounted = open();
  expect((await screen.findByRole("alert")).textContent).toContain("Inventory read denied");
  fireEvent.click(screen.getByRole("button",{name:"Retry"}));
  expect(appState.reloads).toBe(1);
  mounted.unmount();
  appState.contextError = true;
  mounted = open();
  expect((await screen.findByRole("alert")).textContent).toContain("Kubeconfig read denied");
  fireEvent.click(screen.getByRole("button",{name:"Retry"}));
  expect(appState.refreshes).toBe(1);
});
