import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // jsdom, not node: the transport and several service modules touch
    // localStorage, window and WebSocket, and their tests assert that.
    environment: "jsdom",
  },
});
