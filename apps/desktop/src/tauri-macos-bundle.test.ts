// @vitest-environment node
import { expect, it } from "vitest";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

it.each(["aarch64-apple-darwin", "x86_64-apple-darwin"])("prepares a %s launcher for the signed macOS bundle", (target) => {
  const root = mkdtempSync(join(tmpdir(), "srelens-mac-bundle-"));
  try {
    mkdirSync(join(root, "packaging/macos"), {recursive:true});
    const script = resolve(__dirname, "../../../packaging/macos/prepare-launcher.sh");
    expect(existsSync(script), "macOS releases need a launcher preparation step").toBe(true);
    copyFileSync(script, join(root, "packaging/macos/prepare-launcher.sh"));
    const config = JSON.parse(readFileSync(resolve(__dirname, "../src-tauri/macos-launcher.conf.json"), "utf8"));
    mkdirSync(join(root, "fake-bin"));
    writeFileSync(join(root, "fake-bin/cargo"), `#!/bin/sh
set -eu
printf '%s\n' "$@" > cargo-args
mkdir -p "target/${target}/release"
printf 'launcher ${target}' > "target/${target}/release/srelens-sandbox-launch"
chmod +x "target/${target}/release/srelens-sandbox-launch"
`);
    chmodSync(join(root, "fake-bin/cargo"), 0o755);
    const prepared = spawnSync("sh", [join(root, "packaging/macos/prepare-launcher.sh"), target], {env:{...process.env,PATH:join(root,"fake-bin")+":"+process.env.PATH},encoding:"utf8"});
    expect(prepared.status, prepared.stderr).toBe(0);
    expect(readFileSync(join(root,"cargo-args"),"utf8").trim().split("\n")).toEqual(["build","--release","--locked","--target",target,"-p","srelens-plugin-host","--bin","srelens-sandbox-launch"]);
    const binary = config.bundle.macOS?.files?.["MacOS/srelens-sandbox-launch"];
    expect(binary, "the helper must keep its own signature and entitlements").toBeTypeOf("string");
    expect(readFileSync(join(root,"apps/desktop/src-tauri", binary), "utf8")).toBe("launcher "+target);
    const failed = spawnSync("sh", [join(root, "packaging/macos/prepare-launcher.sh"), "x86_64-unknown-linux-gnu"], {env:{...process.env,PATH:join(root,"fake-bin")+":"+process.env.PATH},encoding:"utf8"});
    expect(failed.status).not.toBe(0);
  } finally { rmSync(root, {recursive:true,force:true}); }
});
