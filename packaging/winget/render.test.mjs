// Tests for render.mjs.
//
//   node --test packaging/winget/render.test.mjs
//
// Offline throughout: every case passes `--sums` with a checksum file written
// here, so nothing depends on a release existing or on GitHub being reachable.
//
// The renderer builds every manifest that gets submitted to winget-pkgs, and a
// wrong hash there fails on a stranger's machine at install time — after
// moderation has already approved it, and with nothing pointing back at this
// repository. The failure paths matter more than the happy one.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const RENDER = join(HERE, "render.mjs");
const VERSION = "1.2.3";
const ARCHIVE = `srelens-tui-${VERSION}-x86_64-pc-windows-msvc.zip`;
const HASH = "a".repeat(64);

let work;
const workdir = () => {
  work = mkdtempSync(join(tmpdir(), "winget-render-"));
  return work;
};

test.afterEach(() => {
  if (work) rmSync(work, { recursive: true, force: true });
  work = undefined;
});

/** Run the renderer; returns { status, stderr, dir }. */
function render(args, sums) {
  const dir = workdir();
  const sumsPath = join(dir, "SHA256SUMS.txt");
  if (sums !== null) writeFileSync(sumsPath, sums);
  const out = join(dir, "out");
  try {
    const stderr = execFileSync(
      process.execPath,
      [RENDER, ...args, "--out", out, "--sums", sumsPath],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }
    );
    return { status: 0, stderr, out };
  } catch (error) {
    return { status: error.status ?? 1, stderr: error.stderr ?? "", out };
  }
}

const manifests = (out) =>
  join(out, "manifests", "s", "srelens", "srelens-tui", VERSION);

test("renders all three manifests with the published checksum", () => {
  const { status, out } = render([VERSION], `${HASH}  ${ARCHIVE}\n`);
  assert.equal(status, 0);

  const dir = manifests(out);
  const version = readFileSync(join(dir, "srelens.srelens-tui.yaml"), "utf8");
  const locale = readFileSync(join(dir, "srelens.srelens-tui.locale.en-US.yaml"), "utf8");
  const installer = readFileSync(join(dir, "srelens.srelens-tui.installer.yaml"), "utf8");

  for (const text of [version, locale, installer]) {
    assert.match(text, /^# GENERATED for srelens-v1\.2\.3 -- do not edit by hand\./);
    assert.match(text, /PackageIdentifier: srelens\.srelens-tui/);
    assert.match(text, /PackageVersion: 1\.2\.3/);
  }

  // Uppercase is what wingetcreate writes and what the winget-pkgs corpus
  // carries; a lowercase hash would read as a hand edit.
  assert.match(installer, new RegExp(`InstallerSha256: ${"A".repeat(64)}`));
  assert.match(
    installer,
    /InstallerUrl: https:\/\/github\.com\/srelens\/srelens\/releases\/download\/srelens-v1\.2\.3\/srelens-tui-1\.2\.3-x86_64-pc-windows-msvc\.zip/
  );
  assert.match(installer, /NestedInstallerType: portable/);
  assert.match(installer, /RelativeFilePath: srelens-tui\.exe/);
});

test("the rendered manifests are pure ASCII", () => {
  // They are consumed by Microsoft's tooling and reviewed in a repository
  // that is not ours; an em dash in a generated banner is a needless variable.
  const { status, out } = render([VERSION], `${HASH}  ${ARCHIVE}\n`);
  assert.equal(status, 0);
  const dir = manifests(out);
  for (const name of [
    "srelens.srelens-tui.yaml",
    "srelens.srelens-tui.locale.en-US.yaml",
    "srelens.srelens-tui.installer.yaml",
  ]) {
    const text = readFileSync(join(dir, name), "utf8");
    // eslint-disable-next-line no-control-regex
    assert.equal(/[^\x00-\x7F]/.test(text), false, `${name} carries a non-ASCII byte`);
  }
});

test("refuses a version that is not X.Y.Z", () => {
  // winget is a catalogue of stable software, and a dev pre-release is not
  // what someone typing `winget install` is asking for.
  const { status, stderr } = render(["0.8.1-154"], `${HASH}  ${ARCHIVE}\n`);
  assert.equal(status, 1);
  assert.match(stderr, /not a stable X\.Y\.Z version/);
});

test("refuses a checksum file that does not list the archive", () => {
  const { status, stderr } = render(
    [VERSION],
    `${HASH}  srelens-tui-${VERSION}-x86_64-unknown-linux-musl.tar.gz\n`
  );
  assert.equal(status, 1);
  assert.match(stderr, /does not list srelens-tui-1\.2\.3-x86_64-pc-windows-msvc\.zip/);
});

test("refuses a checksum that is not a sha256", () => {
  // A truncated or mangled line must stop the render rather than travel into
  // a manifest, where it becomes someone else's failed install.
  const { status, stderr } = render([VERSION], `deadbeef  ${ARCHIVE}\n`);
  assert.equal(status, 1);
  assert.match(stderr, /is not a sha256/);
});

test("refuses an empty checksum file", () => {
  const { status, stderr } = render([VERSION], "");
  assert.equal(status, 1);
  assert.match(stderr, /does not list/);
});

test("accepts the asterisk form sha256sum writes for binary mode", () => {
  const { status, out } = render([VERSION], `${HASH} *${ARCHIVE}\n`);
  assert.equal(status, 0);
  const installer = readFileSync(
    join(manifests(out), "srelens.srelens-tui.installer.yaml"),
    "utf8"
  );
  assert.match(installer, new RegExp(`InstallerSha256: ${"A".repeat(64)}`));
});

test("reports a missing version argument rather than rendering nothing", () => {
  const { status, stderr } = render([], `${HASH}  ${ARCHIVE}\n`);
  assert.equal(status, 1);
  assert.match(stderr, /usage: render\.mjs/);
});
