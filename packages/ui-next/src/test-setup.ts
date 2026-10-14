import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// Unmount React trees between tests so repeated render() calls don't stack —
// without this a query for a button finds every previous test's copy too.
afterEach(() => cleanup());

// The design preference lives in localStorage, so one test's writes must not
// leak into the next.
afterEach(() => {
  if (typeof localStorage !== "undefined") localStorage.clear();
});

// A test may pin process.env.TZ, so put back the zone the run started in —
// by name: `delete process.env.TZ` does not bring the machine's zone back on
// Windows, where Node keeps the last zone set.
const zone = process.env.TZ ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
afterEach(() => { process.env.TZ = zone; });

// jsdom has no PointerEvent, so Testing Library falls back to a bare Event and a
// test's `{ button: 2 }` or `clientX` never reaches the handler: a click on any
// button looks like a click on none. A MouseEvent carries both. A file that opts
// into the node environment has no window, and no MouseEvent to extend.
const host = (typeof window === "undefined" ? undefined : window) as { PointerEvent?: unknown } | undefined;
if (host !== undefined && host.PointerEvent === undefined) {
  class PointerEventPolyfill extends MouseEvent {
    readonly pointerId: number;
    readonly pointerType: string;
    readonly isPrimary: boolean;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 0;
      this.pointerType = init.pointerType ?? "mouse";
      this.isPrimary = init.isPrimary ?? true;
    }
  }
  host.PointerEvent = PointerEventPolyfill;
}
