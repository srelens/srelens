import { defineConfig } from "vitest/config";
import { TEST_TIMEOUT_MS, TEST_EXEC_ARGV } from "../../vitest.shared";

// No react plugin: esbuild compiles JSX from tsconfig's `jsx: react-jsx`, the
// same way apps/desktop's config does. No coverage block either — the root
// vitest.config.ts owns the gate, because the floors are measured across every
// package together.
export default defineConfig({
  test: {
    environment: "jsdom",
    /** 15s, not vitest's 5s default — see `vitest.shared.ts` for why. */
    testTimeout: TEST_TIMEOUT_MS,
    execArgv: TEST_EXEC_ARGV,
    setupFiles: ["./src/test-setup.ts"],
  },
});
