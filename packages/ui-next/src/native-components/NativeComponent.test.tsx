import { fireEvent, render, screen, cleanup, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NativeComponent } from "./NativeComponent";
import * as link from "@srelens/core/lib/nativeComponentLink";
const p = (type: string, data: unknown) => ({ version: 1, type, data });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const samples = [
  p("KeyValue", { items: [{ label: "Namespace", value: "production" }] }),
  p("Badge", { label: "Ready", tone: "ok" }),
  p("Metric", { label: "Replicas", value: 0, unit: "pods" }),
  p("Conditions", { items: [{ type: "Ready", status: "False", reason: "Pending", message: "Waiting for pods" }] }),
  p("Events", { items: [{ type: "Warning", reason: "Failed", message: "Source unavailable", count: 0 }] }),
  p("Table", { columns: [{ key: "name", label: "Name" }], rows: [["web"]] }),
  p("Timeline", { items: [{ time: "2026-09-22T12:00:00Z", title: "Deployed", detail: "Revision 1" }] }),
  p("Markdown", { text: "# Notes\n\n**Ready**" }),
  p("Code", { text: "kind: Pod", language: "yaml" }),
  p("Timeseries", { label: "CPU", unit: "cores", range: { start: 0, end: 120_000 }, times: [0, 60_000], series: [{ name: "web", values: [0.1, 0.2] }] }),
];
describe("NativeComponent", () => {
  it.each(samples)("renders $type through the catalog", payload => {
    const { container } = render(<NativeComponent label="Component" payload={payload}/>);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(container.textContent).not.toBe("");
  });
  it.each(samples)("keeps loading, failed and empty $type distinct", payload => {
    const retry = vi.fn();
    const { rerender } = render(<NativeComponent label="Component" payload={payload} state={{status:"loading"}}/>);
    expect(screen.getByRole("status")).toBeTruthy();
    rerender(<NativeComponent label="Component" payload={payload} state={{status:"error",error:"Access denied"}} onRetry={retry}/>);
    expect(screen.getByRole("alert").textContent).toContain("Access denied");
    fireEvent.click(screen.getByRole("button",{name:"Retry"})); expect(retry).toHaveBeenCalledOnce();
    rerender(<NativeComponent label="Component" payload={payload} state={{status:"empty"}}/>);
    expect(screen.getByText("No data reported.")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("fails closed on unknown components and malformed data", () => {
    render(<NativeComponent label="Component" payload={p("HTML",{html:"<script>alert(1)</script>"})}/>);
    expect(screen.getByRole("alert").textContent).toContain("Unsupported");
  });
  it("distinguishes zero, false and unknown values", () => {
    render(<NativeComponent label="Facts" payload={p("KeyValue",{items:[{label:"Count",value:0},{label:"Enabled",value:false},{label:"Missing",value:null}]})}/>);
    expect(screen.getByText("0")).toBeTruthy(); expect(screen.getByText("No")).toBeTruthy();expect(screen.getByText("Unknown")).toBeTruthy();
  });
  it("shows counted expansion and resets it for replacement payloads", () => {
    const make = (prefix: string) => p("KeyValue",{items:Array.from({length:25},(_,i)=>({label:`${prefix}${i}`,value:i}))});
    const { rerender } = render(<NativeComponent label="Facts" payload={make("row")}/>);
    expect(screen.queryByText("row24")).toBeNull();
    expect(screen.getByRole("button",{name:/Show 5 more/}).getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(screen.getByRole("button",{name:/Show 5 more/})); expect(screen.getByText("row24")).toBeTruthy();
    expect(screen.getByRole("button",{name:/Show fewer/}).getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(screen.getByRole("button",{name:/Show fewer/})); expect(screen.queryByText("row24")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:/Show 5 more/}));
    rerender(<NativeComponent label="Facts" payload={make("new")}/>);
    expect(screen.queryByText("new24")).toBeNull();
  });
  it("always renders Code read-only with copy and no app editor options", () => {
    render(<NativeComponent label="Manifest" payload={p("Code",{text:"kind: Pod",language:"yaml"})}/>);
    expect(screen.getByRole("textbox",{name:"Manifest"}).getAttribute("contenteditable")).toBe("false");
    expect(screen.getByRole("button",{name:"Copy"})).toBeTruthy();
  });
  it("renders Markdown as safe host elements", () => {
    const { container } = render(<NativeComponent label="Notes" payload={p("Markdown",{text:'# Notes\n\n**Ready** `machine`\n\n<script>alert(1)</script>\n\n![image](https://example.com/x)\n\n[bad](javascript:alert(1))\n\n[Docs](https://example.com/docs)'})}/>);
    expect(screen.getByRole("heading",{name:"Notes"})).toBeTruthy();
    expect(container.querySelector("script, img, iframe")).toBeNull();
    expect(screen.queryByRole("link",{name:"bad"})).toBeNull();
    expect(screen.getByRole("link",{name:/Docs/}).getAttribute("rel")).toBe("noopener noreferrer");
  });
  it.each(["KeyValue","Conditions","Events","Timeline"])("shows a successful empty %s collection", type => {
    render(<NativeComponent label="Component" payload={p(type,{items:[]})}/>);
    expect(screen.getByText("No data reported.")).toBeTruthy();
  });
  it.each(["Markdown","Code"])("shows successful empty %s text",type=>{
    render(<NativeComponent label="Component" payload={p(type,type==="Code"?{text:"",language:"none"}:{text:""})}/>);
    expect(screen.getByText("No data reported.")).toBeTruthy();
  });
});

it("retains condition generation without treating every true condition as healthy", () => {
  render(<NativeComponent label="Conditions" payload={p("Conditions",{items:[{type:"Degraded",status:"True",observedGeneration:3}]})}/>);
  expect(screen.getByText("Observed generation: 3")).toBeTruthy();
  expect(screen.getByText("True").getAttribute("data-tone")).toBe("muted");
});

it("opens a Markdown link only on click and reports failures with retry", async () => {
  const open = vi.spyOn(link, "openNativeComponentLink").mockRejectedValueOnce(new Error("No default browser")).mockResolvedValue(undefined);
  render(<NativeComponent label="Notes" payload={p("Markdown", {text:"[Docs](HTTPS://example.com/docs)"})}/>);
  expect(open).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("link",{name:/Docs/}));
  expect(open).toHaveBeenCalledWith("https://example.com/docs");
  expect((await screen.findByRole("alert")).textContent).toContain("No default browser");
  fireEvent.click(screen.getByRole("button",{name:"Retry link"}));
  await waitFor(()=>expect(screen.queryByRole("alert")).toBeNull());
  expect(open).toHaveBeenCalledTimes(2);
});
it("supports the bounded Markdown subset including counted lists and code fences", () => {
  const text = "# Heading\n\n" + Array.from({length:22},(_,i)=>`- Item ${i}`).join("\n") + "\n\n1. First\n2. Second\n\n| Name | Value |\n|---|---|\n| web | 2 |\n\n```\nmachine: value\n```";
  render(<NativeComponent label="Notes" payload={p("Markdown",{text})}/>);
  expect(screen.getByRole("heading",{name:"Heading"})).toBeTruthy();
  expect(screen.queryByText("Item 21")).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:/Show 2 more/}));
  expect(screen.getByText("Item 21")).toBeTruthy();
  expect(screen.getByText("First")).toBeTruthy(); expect(screen.getByText("web")).toBeTruthy();
  expect(screen.getByRole("textbox",{name:"Code block"}).getAttribute("contenteditable")).toBe("false");
});
it("rejects oversized Markdown tables visibly", () => {
  const row = Array.from({length:21},(_,i)=>`c${i}`).join("|");
  render(<NativeComponent label="Notes" payload={p("Markdown",{text:row+"\n"+Array(21).fill("---").join("|")})}/>);
  expect(screen.getByRole("alert").textContent).toContain("at most 20 columns");
});

it("clears a previous opening error when refreshed Markdown changes the link URL", async () => {
  vi.spyOn(link, "openNativeComponentLink").mockRejectedValue(new Error("Old link unavailable"));
  const { rerender } = render(<NativeComponent label="Notes" payload={p("Markdown", {text:"[Docs](https://old.example/docs)"})}/>);
  fireEvent.click(screen.getByRole("link",{name:/Docs/}));
  expect((await screen.findByRole("alert")).textContent).toContain("Old link unavailable");
  rerender(<NativeComponent label="Notes" payload={p("Markdown", {text:"[Docs](https://new.example/docs)"})}/>);
  expect(screen.getByRole("link",{name:/Docs/}).getAttribute("href")).toBe("https://new.example/docs");
  expect(screen.queryByRole("alert")).toBeNull();
});
it("preserves Markdown list expansion across unchanged parent rerenders", () => {
  const text = Array.from({length:22},(_,i)=>`- Item ${i}`).join("\n");
  const { rerender } = render(<NativeComponent label="Notes" payload={p("Markdown", {text})}/>);
  fireEvent.click(screen.getByRole("button",{name:/Show 2 more/}));
  expect(screen.getByText("Item 21")).toBeTruthy();
  rerender(<NativeComponent label="Updated host label" payload={p("Markdown", {text})}/>);
  expect(screen.getByText("Item 21")).toBeTruthy();
  expect(screen.getByRole("button",{name:"Show fewer"})).toBeTruthy();
});
