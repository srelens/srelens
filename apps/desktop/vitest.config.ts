import path from "node:path";
import { defineConfig } from "vitest/config";
import { TEST_TIMEOUT_MS } from "../../vitest.shared";

export default defineConfig({
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  test: {
    environment: "jsdom",
    /** 15s, not vitest's 5s default — see `vitest.shared.ts` for why. */
    testTimeout: TEST_TIMEOUT_MS,
    setupFiles: ["./src/test-setup.ts"],
  },
});
