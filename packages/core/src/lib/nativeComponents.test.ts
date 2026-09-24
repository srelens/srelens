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

describe("Timeseries wire contract", () => {
  const start = Date.UTC(2026, 8, 24, 12);
  const minute = 60_000;
  // An `undefined` override removes the field, so "missing" is tested as absent JSON.
  const series = (data: Record<string, unknown> = {}) => payload("Timeseries", Object.fromEntries(Object.entries({
    label: "CPU usage", unit: "cores", range: { start, end: start + 10 * minute },
    times: [start, start + minute, start + 2 * minute],
    series: [{ name: "web", values: [0.2, null, 0.4] }, { name: "api", values: [0, 0.1, 0.3] }],
    thresholds: [{ label: "Limit", value: 0.5, direction: "above", tone: "sev" }],
    ...data,
  }).filter(([, value]) => value !== undefined)));
  it("accepts one or more series with gaps, units, a range and thresholds", () => {
    expect(validateNativeComponent(series()).ok).toBe(true);
    expect(validateNativeComponent(series({ thresholds: undefined })).ok).toBe(true);
    expect(validateNativeComponent(series({ times: [], series: [{ name: "web", values: [] }] })).ok).toBe(true);
  });
  it.each(["number", "percent", "ratio", "bytes", "bytesPerSecond", "seconds", "cores", "perSecond"])("accepts the %s unit", unit =>
    expect(validateNativeComponent(series({ unit })).ok).toBe(true));
  it.each([
    ["an unknown unit format", { unit: "furlongs" }],
    ["a unit with a custom format string", { unit: "{value} ms" }],
    ["no unit", { unit: undefined }],
    ["no series", { series: [] }],
    ["a series with no name", { series: [{ name: "", values: [1, 2, 3] }] }],
    ["duplicate series names", { series: [{ name: "web", values: [1, 2, 3] }, { name: "web", values: [1, 2, 3] }] }],
    ["a series with fewer values than times", { series: [{ name: "web", values: [1, 2] }] }],
    ["a series with a string value", { series: [{ name: "web", values: [1, "2", 3] }] }],
    ["a series colour", { series: [{ name: "web", values: [1, 2, 3], color: "red" }] }],
    ["more than eight series", { series: Array.from({ length: 9 }, (_, i) => ({ name: `s${i}`, values: [1, 2, 3] })) }],
    ["more than 1,000 samples", { times: Array.from({ length: 1001 }, (_, i) => start + i), series: [{ name: "web", values: Array(1001).fill(1) }], range: { start, end: start + 2000 } }],
    ["times out of order", { times: [start, start + 2 * minute, start + minute] }],
    ["repeated times", { times: [start, start, start + minute] }],
    ["a time outside the range", { times: [start, start + minute, start + 11 * minute] }],
    ["fractional times", { times: [start + 0.5, start + minute, start + 2 * minute] }],
    ["an empty range", { range: { start, end: start } }],
    ["a backwards range", { range: { start: start + minute, end: start } }],
    ["ISO timestamps", { range: { start: "2026-09-24T12:00:00Z", end: "2026-09-24T13:00:00Z" } }],
    ["a threshold with no value", { thresholds: [{ label: "Limit", direction: "above", tone: "sev" }] }],
    ["a threshold with an unknown direction", { thresholds: [{ label: "Limit", value: 1, direction: "outside", tone: "sev" }] }],
    ["a threshold with an arbitrary colour", { thresholds: [{ label: "Limit", value: 1, direction: "above", tone: "#ff0000" }] }],
    ["a threshold with no label", { thresholds: [{ label: "", value: 1, direction: "above", tone: "warn" }] }],
    ["duplicate threshold labels", { thresholds: [{ label: "Limit", value: 1, direction: "above", tone: "warn" }, { label: "Limit", value: 2, direction: "above", tone: "sev" }] }],
    ["more than four thresholds", { thresholds: Array.from({ length: 5 }, (_, i) => ({ label: `t${i}`, value: i, direction: "above", tone: "warn" })) }],
  ])("refuses %s", (_, data) => expect(validateNativeComponent(series(data)).ok).toBe(false));
  it("refuses non-finite samples and thresholds", () => {
    expect(validateNativeComponent(series({ series: [{ name: "web", values: [1, NaN, 3] }] })).ok).toBe(false);
    expect(validateNativeComponent(series({ thresholds: [{ label: "Limit", value: Infinity, direction: "above", tone: "sev" }] })).ok).toBe(false);
  });
  it("accepts eight full series at the sample bound", () => {
    const times = Array.from({ length: 1000 }, (_, i) => start + i * 1000);
    const full = series({ range: { start, end: start + 1000 * 1000 }, times,
      series: Array.from({ length: 8 }, (_, s) => ({ name: `s${s}`, values: times.map((_, i) => Math.sin(i + s)) })) });
    expect(validateNativeComponent(full).ok).toBe(true);
  });
});
