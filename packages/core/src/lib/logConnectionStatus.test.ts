import { describe, it, expect } from "vitest";
import { logConnectionStatus } from "./logConnectionStatus";

describe("logConnectionStatus", () => {
  it("reads 'connecting' as an info-toned 'Connecting'", () => {
    expect(logConnectionStatus("connecting")).toEqual({ label: "Connecting", health: "info" });
  });

  it("reads 'live' as a success-toned 'Following'", () => {
    expect(logConnectionStatus("live")).toEqual({ label: "Following", health: "success" });
  });

  it("reads 'reconnecting' as a warning-toned 'Reconnecting'", () => {
    expect(logConnectionStatus("reconnecting")).toEqual({ label: "Reconnecting", health: "warning" });
  });

  it("reads 'error' as a danger-toned 'Stream stopped'", () => {
    expect(logConnectionStatus("error")).toEqual({ label: "Stream stopped", health: "danger" });
  });

  it("never pairs the live word with anything but the success tone", () => {
    // Guards against a copy-paste that hands 'live' the wrong constant: every
    // other state must resolve to a DIFFERENT (label, health) pair than this
    // one, or the verdict has stopped distinguishing "following" from
    // whatever the other state is.
    const live = logConnectionStatus("live");
    for (const other of ["connecting", "reconnecting", "error"] as const) {
      expect(logConnectionStatus(other)).not.toEqual(live);
    }
  });
});
