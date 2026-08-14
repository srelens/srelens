import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// Canary for issue #29: vitest 1.x silently ignored thresholds placed at the
// top level of `coverage` — they only bite under `coverage.thresholds`. This
// pins the enforced shape and floors so a refactor can't quietly turn
// coverage enforcement back off (the suite would pass while the gate is gone).
// The config is asserted on as text: importing vitest/config here would pull
// esbuild into the jsdom test environment, which it refuses to run in.
describe("coverage threshold config", () => {
  const source = fs.readFileSync(path.resolve(__dirname, "../vitest.config.ts"), "utf8");

  it("keeps the enforced floors in the shape vitest actually reads", () => {
    const thresholds = /thresholds:\s*\{([^}]*)\}/.exec(source)?.[1] ?? "";
    const floor = (key: string) => Number(new RegExp(`${key}:\\s*(\\d+)`).exec(thresholds)?.[1]);
    expect(floor("lines")).toBeGreaterThanOrEqual(85);
    expect(floor("branches")).toBeGreaterThanOrEqual(80);
    expect(floor("functions")).toBeGreaterThanOrEqual(76);
  });

  it("declares each floor exactly once, inside the thresholds block", () => {
    // A second `lines:`/`branches:`/`functions:` would be the silently-ignored
    // top-level spot creeping back in.
    for (const key of ["lines", "branches", "functions"]) {
      expect(source.match(new RegExp(`${key}:`, "g"))).toHaveLength(1);
    }
  });
});
