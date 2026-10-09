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
