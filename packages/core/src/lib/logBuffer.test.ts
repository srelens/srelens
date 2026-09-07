import { describe, it, expect } from "vitest";
import { createLogBuffer, appendLogLines, clearLogBuffer, type LogLine } from "./logBuffer";

const line = (text: string, source = ""): LogLine => ({ source, text });

describe("createLogBuffer", () => {
  it("starts empty and reports nought dropped, not undefined", () => {
    const buffer = createLogBuffer(5);
    expect(buffer.lines).toEqual([]);
    expect(buffer.dropped).toBe(0);
  });
});

describe("appendLogLines", () => {
  it("holds the newest N lines once capacity is exceeded", () => {
    let buffer = createLogBuffer(3);
    buffer = appendLogLines(buffer, [line("a"), line("b"), line("c")]);
    expect(buffer.lines.map((l) => l.text)).toEqual(["a", "b", "c"]);
    buffer = appendLogLines(buffer, [line("d")]);
    // "a" is the oldest; it must be the one gone, and the rest keep order.
    expect(buffer.lines.map((l) => l.text)).toEqual(["b", "c", "d"]);
  });

  it("raises the drop count by exactly what was spliced off the head", () => {
    let buffer = createLogBuffer(3);
    buffer = appendLogLines(buffer, [line("a"), line("b"), line("c")]);
    expect(buffer.dropped).toBe(0);
    buffer = appendLogLines(buffer, [line("d")]); // 1 over -> 1 dropped
    expect(buffer.dropped).toBe(1);
    buffer = appendLogLines(buffer, [line("e"), line("f")]); // 2 more over -> +2
    expect(buffer.dropped).toBe(3);
  });

  it("a buffer that has never overflowed reports nought dropped", () => {
    let buffer = createLogBuffer(10);
    buffer = appendLogLines(buffer, [line("a"), line("b")]);
    expect(buffer.dropped).toBe(0);
  });

  it("keeps the source tag on each line", () => {
    let buffer = createLogBuffer(5);
    buffer = appendLogLines(buffer, [line("boot", "web-1/app"), line("boot", "web-2/app")]);
    expect(buffer.lines.map((l) => l.source)).toEqual(["web-1/app", "web-2/app"]);
  });

  it("appending a batch larger than the whole capacity keeps only the newest N and drops the honest remainder", () => {
    // Capacity 3, and a single append of 5: nothing was ever in the buffer
    // before this call, so the only honest count is "5 arrived, 3 fit" -> 2
    // dropped, keeping the 3 newest (c, d, e) in order.
    let buffer = createLogBuffer(3);
    buffer = appendLogLines(buffer, [line("a"), line("b"), line("c"), line("d"), line("e")]);
    expect(buffer.lines.map((l) => l.text)).toEqual(["c", "d", "e"]);
    expect(buffer.dropped).toBe(2);
  });

  it("a batch larger than capacity arriving on top of an existing buffer drops from the combined whole", () => {
    let buffer = createLogBuffer(3);
    buffer = appendLogLines(buffer, [line("a"), line("b"), line("c")]);
    // Combined would be 3 + 4 = 7; capacity 3 -> drop 4, keep the newest 3.
    buffer = appendLogLines(buffer, [line("d"), line("e"), line("f"), line("g")]);
    expect(buffer.lines.map((l) => l.text)).toEqual(["e", "f", "g"]);
    expect(buffer.dropped).toBe(4);
  });

  it("does nothing on an empty append", () => {
    const buffer = createLogBuffer(3);
    const next = appendLogLines(buffer, []);
    expect(next.lines).toEqual([]);
    expect(next.dropped).toBe(0);
  });
});

describe("clearLogBuffer", () => {
  it("resets both the lines and the drop count", () => {
    let buffer = createLogBuffer(2);
    buffer = appendLogLines(buffer, [line("a"), line("b"), line("c")]);
    expect(buffer.dropped).toBe(1);
    buffer = clearLogBuffer(buffer);
    expect(buffer.lines).toEqual([]);
    expect(buffer.dropped).toBe(0);
  });

  it("keeps the original capacity after clearing", () => {
    let buffer = createLogBuffer(4);
    buffer = clearLogBuffer(buffer);
    buffer = appendLogLines(buffer, [line("a"), line("b"), line("c"), line("d"), line("e")]);
    expect(buffer.lines).toHaveLength(4);
    expect(buffer.dropped).toBe(1);
  });
});
