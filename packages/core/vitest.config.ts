import { defineConfig } from "vitest/config";
import { TEST_TIMEOUT_MS } from "../../vitest.shared";

export default defineConfig({
  test: {
    /** 15s, not vitest's 5s default — see `vitest.shared.ts` for why. */
    testTimeout: TEST_TIMEOUT_MS,
    // jsdom, not node: the transport and several service modules touch
    // localStorage, window and WebSocket, and their tests assert that.
    environment: "jsdom",
  },
});
