import { afterEach, describe, expect, it, vi } from "vitest";
import { onExtensionActionRequested, requestExtensionAction, takeExtensionAction } from "./actionRequests";

const selection = { id: "io.fluxcd", revision: 3, capability: "helmreleases", context: "c-1", namespace: "flux", name: "web" };
const request = { id: "io.fluxcd", capability: "helmreleases", context: "c-1", namespace: "flux", name: "web", action: "reconcile" };

afterEach(() => {
  takeExtensionAction(selection);
  vi.useRealTimers();
});

describe("palette action requests (#544)", () => {
  it("is taken once, by the view of the resource it names", () => {
    requestExtensionAction(request);
    expect(takeExtensionAction({ ...selection, name: "other" })).toBeNull();
    expect(takeExtensionAction({ ...selection, context: "c-2" })).toBeNull();
    expect(takeExtensionAction({ ...selection, capability: "kustomizations" })).toBeNull();
    expect(takeExtensionAction(selection)).toBe("reconcile");
    expect(takeExtensionAction(selection)).toBeNull();
  });

  it("tells views already open, so the current tab answers without remounting", () => {
    const heard = vi.fn();
    const stop = onExtensionActionRequested(heard);
    requestExtensionAction(request);
    expect(heard).toHaveBeenCalledTimes(1);
    stop();
    requestExtensionAction(request);
    expect(heard).toHaveBeenCalledTimes(1);
  });

  it("does not wait forever for a view that never opened", () => {
    vi.useFakeTimers();
    requestExtensionAction(request);
    vi.advanceTimersByTime(10_001);
    expect(takeExtensionAction(selection)).toBeNull();
  });

  it("keeps only the newest request", () => {
    requestExtensionAction({ ...request, action: "suspend" });
    requestExtensionAction(request);
    expect(takeExtensionAction(selection)).toBe("reconcile");
  });
});
