import { describe, it, expect } from "vitest";
import { avatarColor, avatarInitials } from "./avatar";

describe("avatarColor", () => {
  it("is deterministic and returns a hex colour", () => {
    expect(avatarColor("kind-dev")).toBe(avatarColor("kind-dev"));
    expect(avatarColor("kind-dev")).toMatch(/^#[0-9a-f]{6}$/i);
  });
});

describe("avatarInitials", () => {
  it("derives up to three initials, splitting on separators", () => {
    expect(avatarInitials("prod")).toBe("PR");
    expect(avatarInitials("kind-dev")).toBe("KD");
    expect(avatarInitials("my_staging_cluster")).toBe("MSC");
    expect(avatarInitials("")).toBe("?");
  });

  it("skips generated-id segments so similar contexts stay distinguishable (#209)", () => {
    // The issue's real-world set: two initials collapsed all of these to
    // DL/DL/DL/DL and ML/ML.
    expect(avatarInitials("dev-lon-nrtc-6bcb8b63")).toBe("DLN");
    expect(avatarInitials("dev-lon-nrtc-6bcb8b63-admin")).toBe("DLN");
    expect(avatarInitials("dev-lon-workload-15d9c530")).toBe("DLW");
    expect(avatarInitials("mbr-lon-nrtc-c678b415")).toBe("MLN");
    expect(avatarInitials("k3d-dev-ai")).toBe("KDA");
    expect(avatarInitials("k3d-nats")).toBe("KN");
  });

  it("keeps hex-alphabet words and digit-bearing short names as identity", () => {
    // Real words that happen to be hex letters are not ids...
    expect(avatarInitials("decade-cluster")).toBe("DC");
    // ...k3d/k8s-style segments have non-hex letters, so they count...
    expect(avatarInitials("k3d-7f3a9b21")).toBe("K3");
    // ...and hex-looking segments UNDER 8 chars are treated as chosen names,
    // not ids (segments are user-defined — `c0ffee` is somebody's cluster).
    expect(avatarInitials("prod-c0ffee")).toBe("PC");
    expect(avatarInitials("prod-babe42")).toBe("PB");
  });

  it("keeps short numeric segments — often the only distinguisher", () => {
    expect(avatarInitials("cluster-1")).toBe("C1");
    expect(avatarInitials("cluster-2")).toBe("C2");
    // Long digit runs are timestamps/serials, not names.
    expect(avatarInitials("prod-eu-20260813")).toBe("PE");
  });

  it("falls back to raw segments when everything looks generated", () => {
    expect(avatarInitials("7f3a9b21")).toBe("7F");
    expect(avatarInitials("123456-456789ab")).toBe("14");
  });
});
