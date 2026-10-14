import { describe, it, expect, vi } from "vitest";
import { DEFAULT_LOG_HISTORY_MB, LOG_HISTORY_KEY, logHistoryMb, megabytes, saveLogHistoryMb } from "./logHistory";

function storage(initial?: string) {
  const m = new Map<string, string>(initial === undefined ? [] : [[LOG_HISTORY_KEY, initial]]);
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
}

describe("log history setting", () => {
  it("defaults to 256 MB and reads back what was saved", () => {
    const s = storage();
    expect(logHistoryMb(s)).toBe(DEFAULT_LOG_HISTORY_MB);
    saveLogHistoryMb(1024, s);
    expect(logHistoryMb(s)).toBe(1024);
  });

  it("reads anything that is not one of the choices as the default", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(logHistoryMb(storage("7"))).toBe(256);
      expect(error).not.toHaveBeenCalled();
      expect(logHistoryMb(storage("not json"))).toBe(256);
      expect(error).toHaveBeenCalledTimes(1);
    } finally {
      error.mockRestore();
    }
  });

  it("converts to bytes for the stream", () => {
    expect(megabytes(256)).toBe(256 * 1024 * 1024);
  });
});
