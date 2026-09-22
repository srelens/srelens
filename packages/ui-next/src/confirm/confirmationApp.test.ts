import { describe, it, expect } from "vitest";
import type { InstalledExtension } from "@srelens/core";
import { appIdentity } from "./confirmationApp";

function installed(over: Partial<InstalledExtension> & { id: string; name: string }): InstalledExtension {
  const { id, name, ...rest } = over;
  return {
    manifest: {
      id,
      name,
      version: "1.0.0",
      srelensApiVersion: "1",
      kind: "declarative",
      permissions: [],
      capabilities: [],
      contributions: { pages: [], detailTabs: [], detailLinks: [] },
    },
    enabled: true,
    revision: 3,
    grants: [],
    settings: {},
    source: "local",
    installedAt: 0,
    history: [],
    ...rest,
  } as InstalledExtension;
}

describe("who the host says asked", () => {
  it("names a signed app by its manifest name and its publisher", () => {
    const plugins = [
      installed({ id: "flux", name: "Flux Tools", signatureProof: { manifest: "{}", signature: [1] } }),
    ];
    expect(appIdentity(plugins, { id: "flux", revision: 3 })).toEqual({
      name: "Flux Tools",
      publisher: "srelens",
    });
  });

  it("reports an app with no signature as unsigned", () => {
    const plugins = [installed({ id: "draft", name: "Local Draft" })];
    expect(appIdentity(plugins, { id: "draft", revision: 3 })).toEqual({
      name: "Local Draft",
      publisher: null,
    });
  });

  /**
   * A quarantined app's stored name is one the host no longer accepts and may
   * display as another app's — `extensionLabel`'s rule, applied here because
   * this is the sentence where being taken for another app pays best.
   */
  it("names a quarantined app by its ID, and does not call its signature good", () => {
    const plugins = [
      installed({
        id: "flux",
        name: "srelens Core",
        quarantined: "App publisher signature is invalid",
        signatureProof: { manifest: "{}", signature: [1] },
      }),
    ];
    expect(appIdentity(plugins, { id: "flux", revision: 3 })).toEqual({
      name: "flux",
      publisher: null,
    });
  });

  /**
   * The revision is part of the identity, not decoration. `resolve`
   * (`crates/registry/src/extensions/resource.rs`) will only run an action
   * for `p.enabled && p.manifest.id == selection.id && p.revision ==
   * selection.revision`, so a review raised under revision 4 and answered
   * after an update to revision 5 is one the host would refuse anyway — and
   * labelling it with revision 5's name and signature state would vouch for
   * an app that is not the one that asked.
   */
  it("says nothing when the app has been replaced since the review was raised", () => {
    const plugins = [
      installed({
        id: "flux",
        name: "Flux Tools",
        revision: 5,
        signatureProof: { manifest: "{}", signature: [1] },
      }),
    ];
    expect(appIdentity(plugins, { id: "flux", revision: 4 })).toBeNull();
    expect(appIdentity(plugins, { id: "flux", revision: 5 })).toEqual({
      name: "Flux Tools",
      publisher: "srelens",
    });
  });

  it("says nothing rather than inventing a requester it cannot find", () => {
    expect(appIdentity([installed({ id: "flux", name: "Flux Tools" })], { id: "gone", revision: 1 })).toBeNull();
    expect(appIdentity([], { id: "flux", revision: 1 })).toBeNull();
    expect(appIdentity([installed({ id: "flux", name: "Flux Tools" })], null)).toBeNull();
  });
});
