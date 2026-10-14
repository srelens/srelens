import { expect, it } from "vitest";

// The desktop project's `setupFiles` must list `vitest.offline-websocket.ts`
// too; its behaviour is tested in packages/ui-next/src/test-setup.test.ts.
// Checked by name rather than by importing the setup file, which would install
// it and so pass even when `setupFiles` no longer lists it.
it("gives tests the offline WebSocket, not jsdom's", () => {
  expect(WebSocket.name).toBe("OfflineWebSocket");
});
