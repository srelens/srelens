import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { trimDevMeasures } from "./devPerformance";

const measures = () => performance.getEntriesByType("measure");

describe("trimDevMeasures", () => {
  // Only the interval is faked: the entries under test belong to the real
  // `performance`, which fake timers would otherwise swap for their own.
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    performance.clearMeasures();
  });
  afterEach(() => {
    vi.useRealTimers();
    performance.clearMeasures();
  });

  it("clears the measures React's dev build records, once an interval has passed", () => {
    const stop = trimDevMeasures(1000);
    performance.measure("⚛ App");
    expect(measures()).toHaveLength(1);

    vi.advanceTimersByTime(999);
    expect(measures()).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(measures()).toHaveLength(0);

    performance.measure("⚛ App");
    vi.advanceTimersByTime(1000);
    expect(measures()).toHaveLength(0);
    stop();
  });

  it("clears every second by default", () => {
    const stop = trimDevMeasures();
    performance.measure("⚛ App");
    // Up to the last millisecond of the second, so a shorter default is caught as well as a longer one.
    vi.advanceTimersByTime(999);
    expect(measures()).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(measures()).toHaveLength(0);
    stop();
  });

  it("stops clearing when asked", () => {
    const stop = trimDevMeasures(1000);
    stop();
    performance.measure("⚛ App");
    vi.advanceTimersByTime(5000);
    expect(measures()).toHaveLength(1);
  });
});

/**
 * The entry starts it, and only in a dev build. `main.tsx` cannot be imported
 * from a test (it boots the app), so this reads its source, as
 * `design.theme.test.ts` does. `import.meta.env.DEV` is statically false in a
 * production build, so the guarded call and the module behind it are dropped.
 */
describe("the entry's dev performance wiring", () => {
  const code = readFileSync(join(__dirname, "main.tsx"), "utf8")
    .split("\n")
    .filter((line) => !/^\s*(\/\/|\/\*|\*)/.test(line))
    .join("\n");

  it("imports the helper", () => {
    expect(code).toMatch(/import\s*\{\s*trimDevMeasures\s*\}\s*from\s*"\.\/devPerformance"/);
  });

  it("calls it only when import.meta.env.DEV", () => {
    expect(code).toMatch(/if\s*\(\s*import\.meta\.env\.DEV\s*\)\s*\{?\s*trimDevMeasures\(\)/);
  });
});
