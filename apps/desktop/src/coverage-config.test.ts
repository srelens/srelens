// @vitest-environment node
// (importing vitest/config pulls in esbuild, which refuses to run under the
// jsdom environment the rest of the suite uses — node is fine)
import { describe, expect, it } from "vitest";
// The gate moved to the workspace root when @srelens/core was extracted:
// each package alone sits below floors calibrated for the combined codebase,
// so the two are measured together. This canary follows it there.
import config from "../../../vitest.config";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// Canary for issue #29: vitest 1.x silently ignored thresholds placed at the
// top level of `coverage` — they only bite under `coverage.thresholds`. This
// asserts on the exported config object itself, so a floor that is commented
// out, moved under `test`, or otherwise outside `test.coverage.thresholds`
// fails here instead of silently turning the coverage gate off.
//
// It also catches the gate being dropped entirely while both packages still
// report green, which is what would happen if the root config lost its
// coverage block.
describe("coverage threshold config", () => {
  type CoverageShape = {
    lines?: number;
    branches?: number;
    functions?: number;
    thresholds?: { lines?: number; branches?: number; functions?: number };
  };
  const coverage = (config as { test?: { coverage?: CoverageShape } }).test?.coverage;

  it("keeps the enforced floors in the shape vitest actually reads", () => {
    expect(coverage?.thresholds?.lines).toBeGreaterThanOrEqual(85);
    expect(coverage?.thresholds?.branches).toBeGreaterThanOrEqual(80);
    expect(coverage?.thresholds?.functions).toBeGreaterThanOrEqual(76);
  });

  it("has no floors in the silently-ignored top-level spot", () => {
    expect(coverage?.lines).toBeUndefined();
    expect(coverage?.branches).toBeUndefined();
    expect(coverage?.functions).toBeUndefined();
  });

  it("measures every workspace package, not a hardcoded pair", () => {
    // Naming the packages here is how packages/ui-next came to be silently
    // untested: it was added to the workspace, its suite passed locally under
    // its own script, and the root run never loaded it while reporting green.
    // Enumerating them from disk means the next package cannot repeat that.
    const projects = (config as { test?: { projects?: string[] } }).test?.projects ?? [];
    const dir = join(__dirname, "../../../packages");
    const members = readdirSync(dir).filter((name) =>
      existsSync(join(dir, name, "vitest.config.ts")),
    );
    expect(members.length).toBeGreaterThan(0);
    for (const name of members) {
      expect(projects, `packages/${name} is not in the root vitest projects`).toContain(
        `packages/${name}`,
      );
    }
    expect(projects).toContain("apps/desktop");
  });

});
