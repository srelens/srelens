import { describe, it, expect, vi } from "vitest";
import { checkForUpdateAndNotify } from "./updateNotifier";
import type { UpdateMeta } from "./updater";

const update: UpdateMeta = {
  version: "0.3.0",
  currentVersion: "0.2.0",
  notes: "",
  external: false,
  elevates: false,
};

describe("checkForUpdateAndNotify", () => {
  it("notifies when a newer version is available", async () => {
    const onAvailable = vi.fn();
    await checkForUpdateAndNotify("stable", onAvailable, { check: async () => update });
    expect(onAvailable).toHaveBeenCalledWith(update);
  });

  it("does not notify when already up to date", async () => {
    const onAvailable = vi.fn();
    await checkForUpdateAndNotify("stable", onAvailable, { check: async () => null });
    expect(onAvailable).not.toHaveBeenCalled();
  });

  it("stays silent when the check fails (an automatic check must not nag)", async () => {
    const onAvailable = vi.fn();
    await checkForUpdateAndNotify("stable", onAvailable, {
      check: async () => {
        throw new Error("offline");
      },
    });
    expect(onAvailable).not.toHaveBeenCalled();
  });

  it("does not re-notify a version the user was already told about", async () => {
    const onAvailable = vi.fn();
    await checkForUpdateAndNotify("stable", onAvailable, {
      check: async () => update,
      alreadyNotified: (v) => v === "0.3.0",
    });
    expect(onAvailable).not.toHaveBeenCalled();
  });
});
