// @vitest-environment node
// (reads files from disk; jsdom buys nothing here)
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

/**
 * Executable apps on Linux need the trusted sandbox launcher beside the
 * `srelens` binary, where the host looks for it
 * (`crates/registry/src/extensions/sidecars.rs`). Tauri merges
 * `tauri.linux.conf.json` into the config only when it bundles on Linux, so the
 * launcher is built and placed in the deb, the rpm and the AppImage, and every
 * `cargo build` elsewhere is untouched. `bundle.externalBin` would place it
 * too, but `tauri-build` resolves that at compile time and fails every build
 * of the desktop crate that runs before the launcher exists.
 */
const root = join(__dirname, "..");
const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.linux.conf.json"), "utf8"));
const launcher = "/usr/bin/srelens-sandbox-launch";
const built = "../../../target/release/srelens-sandbox-launch";

describe("the Linux bundles", () => {
  it.each(["deb", "rpm", "appimage"])("put the sandbox launcher beside srelens in the %s", (format) => {
    expect(config.bundle.linux[format].files).toEqual({ [launcher]: built });
  });

  it("build the launcher, in release, before bundling", () => {
    expect(config.build.beforeBundleCommand).toBe(
      "cargo build --release --locked -p srelens-plugin-host --bin srelens-sandbox-launch",
    );
  });

  it("take the launcher from the workspace's release directory", () => {
    expect(resolve(root, "src-tauri", built)).toBe(
      resolve(root, "..", "..", "target", "release", "srelens-sandbox-launch"),
    );
  });
});
