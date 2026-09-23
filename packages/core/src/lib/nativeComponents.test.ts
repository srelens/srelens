import { describe, expect, it } from "vitest";
import { validateNativeComponent } from "./nativeComponents";

const payload = (type: string, data: unknown) => ({ version: 1, type, data });
const samples = [
  payload("KeyValue", { items: [{ label: "Namespace", value: "production" }] }),
  payload("Badge", { label: "Ready", tone: "ok" }),
  payload("Metric", { label: "Replicas", value: 0, unit: "pods" }),
  payload("Conditions", { items: [{ type: "Ready", status: "True", reason: "Available", message: "Running" }] }),
  payload("Events", { items: [{ type: "Warning", reason: "Failed", message: "Unavailable", count: 2 }] }),
  payload("Table", { columns: [{ key: "name", label: "Name" }], rows: [["web"]] }),
  payload("Timeline", { items: [{ time: "2026-09-22T12:00:00Z", title: "Deployed", detail: "Revision 1" }] }),
  payload("Markdown", { text: "# Notes\n\n**Ready**" }),
  payload("Code", { text: "kind: Pod", language: "yaml" }),
];

describe("native component wire contract", () => {
  it.each(samples)("accepts $type v1 data", value => expect(validateNativeComponent(value).ok).toBe(true));
  it.each([
    null, [], {}, { ...samples[0], version: 2 }, { ...samples[0], type: "HTML" },
    { ...samples[0], style: "color:red" }, payload("Badge", { label: "Ready", tone: "green" }),
    payload("Metric", { label: "Count", value: Infinity }),
    payload("Code", { text: "x", language: "javascript", onChange: "run()" }),
    payload("Table", { columns: [{ key: "name", label: "Name" }], rows: [["one", "extra"]] }),
    payload("Table", { columns: [{ key: "name", label: "Name" }, { key: "name", label: "Again" }], rows: [] }),
    payload("KeyValue", { items: [{ label: "x", value: { html: "<script>" } }] }),
    payload("Conditions", { items: [{ type: "Ready", status: "yes" }] }),
    payload("Events", { items: [{ type: "Normal", reason: "Fine", message: "ok", count: -1 }] }),
    payload("Timeline", { items: [{ time: "yesterday", title: "Done" }] }),
  ])("refuses unsupported or malformed input %#", value => expect(validateNativeComponent(value).ok).toBe(false));
  it("bounds strings and collections before rendering", () => {
    expect(validateNativeComponent(payload("Code", { text: "x".repeat(262145), language: "none" })).ok).toBe(false);
    expect(validateNativeComponent(payload("KeyValue", { items: Array.from({ length: 1001 }, () => ({ label: "x", value: "y" })) })).ok).toBe(false);
    expect(validateNativeComponent(payload("KeyValue", { items: Array.from({ length: 1000 }, () => ({ label: "x", value: "y".repeat(4096) })) })).ok).toBe(false);
  });
  it("fails closed on cyclic and non-JSON input", () => {
    const cycle: any = {}; cycle.self = cycle;
    expect(validateNativeComponent(cycle).ok).toBe(false);
    expect(validateNativeComponent({ ...samples[0], data: new Date() }).ok).toBe(false);
  });
});

it("accepts observed generation on a condition", () => {
  expect(validateNativeComponent(payload("Conditions", {items:[{type:"Ready",status:"True",observedGeneration:3}]})).ok).toBe(true);
});
it("rejects sparse arrays and accessors without invoking them", () => {
  expect(validateNativeComponent(payload("KeyValue", {items: new Array(1000000000)})).ok).toBe(false);
  let accessed = false;
  const input = { get version() { accessed = true; return 1; }, type:"Badge", data:{label:"Ready",tone:"ok"}};
  expect(validateNativeComponent(input).ok).toBe(false);
  expect(accessed).toBe(false);
});
