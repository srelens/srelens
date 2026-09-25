import path from "node:path";
import { defineConfig } from "vitest/config";
import { TEST_TIMEOUT_MS, TEST_EXEC_ARGV } from "../../vitest.shared";

export default defineConfig({
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  test: {
    environment: "jsdom",
    /** 15s, not vitest's 5s default — see `vitest.shared.ts` for why. */
    testTimeout: TEST_TIMEOUT_MS,
    execArgv: TEST_EXEC_ARGV,
    /** No real WebSocket in a test — see `vitest.offline-websocket.ts` for why. */
    setupFiles: ["../../vitest.offline-websocket.ts", "./src/test-setup.ts"],
  },
});
