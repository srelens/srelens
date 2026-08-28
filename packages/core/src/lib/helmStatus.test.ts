import { describe, it, expect } from "vitest";
import { helmStatus } from "./helmStatus";

describe("helmStatus", () => {
  it("reads a deployed release as healthy", () => {
    expect(helmStatus("deployed")).toEqual({ word: "deployed", health: "success" });
  });

  it("reads a failed release as danger", () => {
    expect(helmStatus("failed")).toEqual({ word: "failed", health: "danger" });
  });

  it.each(["pending-install", "pending-upgrade", "pending-rollback"])(
    "reads %s as in-progress — neither healthy nor broken",
    (status) => {
      const { health } = helmStatus(status);
      expect(health).not.toBe("success");
      expect(health).not.toBe("danger");
      expect(health).toBe("warning");
    },
  );

  it("reads uninstalling as in-progress, the same tone as the pending-* states", () => {
    expect(helmStatus("uninstalling").health).toBe("warning");
  });

  it("does not read superseded as a failure — an older revision replaced is ordinary", () => {
    const { health } = helmStatus("superseded");
    expect(health).not.toBe("danger");
    expect(health).toBe("neutral");
  });

  it("does not read uninstalled as a failure either — the release is simply gone", () => {
    const { health } = helmStatus("uninstalled");
    expect(health).not.toBe("danger");
    expect(health).toBe("neutral");
  });

  it("renders an unfamiliar status as the word Helm gave, toned neutral", () => {
    // Deliberately free of "fail" and "deploy" as substrings: a fixture that
    // happened to contain either would pass even if the fallback secretly
    // matched on substring rather than on the full word.
    expect(helmStatus("canary-preview")).toEqual({ word: "canary-preview", health: "neutral" });
  });

  it("does not crash on the empty string, and reads it neutral", () => {
    expect(helmStatus("")).toEqual({ word: "", health: "neutral" });
  });

  it("never invents a word — the word is always exactly what was passed in", () => {
    expect(helmStatus("deployed").word).toBe("deployed");
    expect(helmStatus("pending-upgrade").word).toBe("pending-upgrade");
    expect(helmStatus("something-new").word).toBe("something-new");
  });
});
