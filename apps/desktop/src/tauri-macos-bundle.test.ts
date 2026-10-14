// @vitest-environment node
import { expect, it, vi } from "vitest";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { parse } from "yaml";

const script = resolve(__dirname, "../../../packaging/macos/prepare-launcher.sh");
const config = JSON.parse(readFileSync(resolve(__dirname, "../src-tauri/macos-launcher.conf.json"), "utf8"));

function fixture(target: string) {
  const root = mkdtempSync(join(tmpdir(), "srelens-mac-bundle-"));
  mkdirSync(join(root, "packaging/macos"), { recursive: true });
  copyFileSync(script, join(root, "packaging/macos/prepare-launcher.sh"));
  mkdirSync(join(root, "fake-bin"));
  writeFileSync(join(root, "fake-bin/cargo"), `#!/bin/sh
set -eu
printf '%s\n' "$@" > cargo-args
mkdir -p "target/${target}/release"
printf 'launcher ${target}' > "target/${target}/release/srelens-sandbox-launch"
chmod +x "target/${target}/release/srelens-sandbox-launch"
`);
  chmodSync(join(root, "fake-bin/cargo"), 0o755);
  for (const command of ["codesign", "security", "uuidgen"]) {
    writeFileSync(join(root, "fake-bin", command), `#!/bin/sh
set -eu
printf '%s %s\n' '${command}' "$*" >> "$TEST_SIGN_TRACE"
if [ '${command}' = uuidgen ]; then printf 'fixture-password\n'; fi
if [ '${command}' = codesign ] && [ "$1" = --force ] && [ "$TEST_FAIL_SIGN" = true ]; then exit 1; fi
`);
    chmodSync(join(root, "fake-bin", command), 0o755);
  }
  return root;
}

function prepare(root: string, target: string, signing: NodeJS.ProcessEnv = {}) {
  return spawnSync("sh", [join(root, "packaging/macos/prepare-launcher.sh"), target], {
    env: {
      ...process.env,
      CARGO_TARGET_DIR: undefined,
      APPLE_SIGNING_IDENTITY: undefined,
      APPLE_CERTIFICATE: undefined,
      APPLE_CERTIFICATE_PASSWORD: undefined,
      PATH: join(root, "fake-bin") + delimiter + process.env.PATH,
      TEST_SIGN_TRACE: join(root, "signing-calls"),
      TEST_FAIL_SIGN: "false",
      ...signing,
    },
    encoding: "utf8",
  });
}

it.each(["aarch64-apple-darwin", "x86_64-apple-darwin"])("stages a %s launcher without inherited signing credentials", (target) => {
  const root = fixture(target);
  vi.stubEnv("APPLE_SIGNING_IDENTITY", "inherited-test-identity");
  vi.stubEnv("APPLE_CERTIFICATE", "ZmFrZQ==");
  vi.stubEnv("APPLE_CERTIFICATE_PASSWORD", "inherited-test-password");
  try {
    const prepared = prepare(root, target);
    expect(prepared.status, prepared.stderr).toBe(0);
    expect(existsSync(join(root, "signing-calls")), "staging must not use the caller's keychain").toBe(false);
    expect(readFileSync(join(root, "cargo-args"), "utf8").trim().split("\n")).toEqual(["build", "--release", "--locked", "--target", target, "-p", "srelens-plugin-host", "--bin", "srelens-sandbox-launch"]);
    const binary = config.bundle.macOS.files["MacOS/srelens-sandbox-launch"];
    expect(readFileSync(join(root, "apps/desktop/src-tauri", binary), "utf8")).toBe("launcher " + target);
    expect(prepare(root, "x86_64-unknown-linux-gnu").status).not.toBe(0);
  } finally {
    vi.unstubAllEnvs();
    rmSync(root, { recursive: true, force: true });
  }
});

it.each(["certificate", "existing-keychain", "failed-signing"])("checks signing and cleanup with %s", (mode) => {
  const root = fixture("aarch64-apple-darwin");
  try {
    const prepared = prepare(root, "aarch64-apple-darwin", {
      APPLE_SIGNING_IDENTITY: "fixture-identity",
      APPLE_CERTIFICATE: mode === "existing-keychain" ? undefined : "ZmFrZQ==",
      APPLE_CERTIFICATE_PASSWORD: "fixture-certificate-password",
      TEST_FAIL_SIGN: mode === "failed-signing" ? "true" : "false",
    });
    expect(prepared.status, prepared.stderr).toBe(mode === "failed-signing" ? 1 : 0);
    const calls = readFileSync(join(root, "signing-calls"), "utf8");
    expect(calls).toContain("codesign --force --options runtime --timestamp");
    expect(calls).toContain("--sign fixture-identity");
    expect(calls).not.toContain("--entitlements");
    if (mode === "failed-signing") expect(calls).not.toContain("codesign --verify");
    else expect(calls).toContain("codesign --verify --strict");
    if (mode === "existing-keychain") expect(calls).not.toContain("security ");
    else {
      expect(calls).toContain("security import ");
      expect(calls).toContain("security set-key-partition-list ");
      expect(calls).toContain("--keychain ");
      const keychain = calls.match(/security create-keychain -p fixture-password (.+)\n/)?.[1];
      expect(keychain).toBeTruthy();
      expect(calls).toContain("security delete-keychain " + keychain);
      expect(existsSync(resolve(keychain!, ".."))).toBe(false);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

it("passes launcher release targets through the environment", () => {
  const workflow = parse(readFileSync(resolve(__dirname, "../../../.github/workflows/release.yml"), "utf8"));
  for (const name of ["Prepare the macOS sandbox launcher", "Assert the macOS sandbox launcher is bundled and signed"]) {
    const step = workflow.jobs.build.steps.find((step: { name: string }) => step.name === name);
    expect(step.env?.SRELENS_BUNDLE_TARGET).toBe("${{ matrix.target }}");
    expect(step.run).not.toContain("${{");
    expect(step.run).toContain("$SRELENS_BUNDLE_TARGET");
  }
});
