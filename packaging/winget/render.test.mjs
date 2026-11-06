// Tests for render.mjs.
//
//   node --test packaging/winget/render.test.mjs
//
// Offline throughout: every case passes `--release` with a release written
// here, in the shape `gh api repos/<repo>/releases/tags/<tag>` prints, so
// nothing depends on a release existing or on GitHub being reachable.
//
// The renderer builds every manifest that gets submitted to winget-pkgs, and a
// wrong hash there fails on a stranger's machine at install time — after
// moderation has already approved it, and with nothing pointing back at this
// repository. The failure paths matter more than the happy one.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const RENDER = join(HERE, "render.mjs");
const VERSION = "1.2.3";
const SRECTL = `srectl-${VERSION}-x86_64-pc-windows-msvc.zip`;
const DESKTOP = `srelens_${VERSION}_x64-setup.exe`;
// Different hashes, so a test can tell which asset's hash landed where.
const SRECTL_HASH = "a".repeat(64);
const DESKTOP_HASH = "b".repeat(64);

/** A release as the GitHub API reports it: each asset with GitHub's digest. */
const release = (...assets) =>
  JSON.stringify({ assets: assets.map(([name, digest]) => ({ name, digest })) });

const BOTH = release(
  [SRECTL, `sha256:${SRECTL_HASH}`],
  [`${SRECTL}.asc`, `sha256:${"c".repeat(64)}`],
  [DESKTOP, `sha256:${DESKTOP_HASH}`]
);

let work;
const workdir = () => {
  work = mkdtempSync(join(tmpdir(), "winget-render-"));
  return work;
};

test.afterEach(() => {
  if (work) rmSync(work, { recursive: true, force: true });
  work = undefined;
});

/** Run the renderer; returns { status, stderr, out }. */
function render(args, json) {
  const dir = workdir();
  const releasePath = join(dir, "release.json");
  writeFileSync(releasePath, json);
  const out = join(dir, "out");
  try {
    const stderr = execFileSync(
      process.execPath,
      [RENDER, ...args, "--out", out, "--release", releasePath],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }
    );
    return { status: 0, stderr, out };
  } catch (error) {
    return { status: error.status ?? 1, stderr: error.stderr ?? "", out };
  }
}

const manifests = (out, name) =>
  join(out, "manifests", "s", "srelens", name, VERSION);

const read = (out, name, file) =>
  readFileSync(join(manifests(out, name), file), "utf8");

const FILES = {
  srectl: [
    "srelens.srectl.yaml",
    "srelens.srectl.locale.en-US.yaml",
    "srelens.srectl.installer.yaml",
  ],
  srelens: [
    "srelens.srelens.yaml",
    "srelens.srelens.locale.en-US.yaml",
    "srelens.srelens.installer.yaml",
  ],
};

test("renders srectl as a portable zip with the archive's checksum", () => {
  const { status, out } = render([VERSION], BOTH);
  assert.equal(status, 0);

  for (const file of FILES.srectl) {
    const text = read(out, "srectl", file);
    assert.match(text, /^# GENERATED for srelens-v1\.2\.3 -- do not edit by hand\./);
    assert.match(text, /PackageIdentifier: srelens\.srectl\n/);
    assert.match(text, /PackageVersion: 1\.2\.3/);
  }

  const installer = read(out, "srectl", "srelens.srectl.installer.yaml");
  // Uppercase is what wingetcreate writes and what the winget-pkgs corpus
  // carries; a lowercase hash would read as a hand edit.
  assert.match(installer, new RegExp(`InstallerSha256: ${"A".repeat(64)}`));
  assert.match(
    installer,
    /InstallerUrl: https:\/\/github\.com\/srelens\/srelens\/releases\/download\/srelens-v1\.2\.3\/srectl-1\.2\.3-x86_64-pc-windows-msvc\.zip/
  );
  assert.match(installer, /NestedInstallerType: portable/);
  assert.match(installer, /RelativeFilePath: srectl\.exe/);
  assert.match(installer, /PortableCommandAlias: srectl\n/);
});

test("renders the desktop app as a per-user NSIS installer with its own checksum", () => {
  const { status, out } = render([VERSION], BOTH);
  assert.equal(status, 0);

  for (const file of FILES.srelens) {
    const text = read(out, "srelens", file);
    assert.match(text, /^# GENERATED for srelens-v1\.2\.3 -- do not edit by hand\./);
    assert.match(text, /PackageIdentifier: srelens\.srelens\n/);
    assert.match(text, /PackageVersion: 1\.2\.3/);
  }

  const installer = read(out, "srelens", "srelens.srelens.installer.yaml");
  assert.match(installer, new RegExp(`InstallerSha256: ${"B".repeat(64)}`));
  assert.match(
    installer,
    /InstallerUrl: https:\/\/github\.com\/srelens\/srelens\/releases\/download\/srelens-v1\.2\.3\/srelens_1\.2\.3_x64-setup\.exe/
  );
  assert.match(installer, /InstallerType: nullsoft/);
  assert.match(installer, /Scope: user/);
});

test("the rendered manifests are pure ASCII", () => {
  // They are consumed by Microsoft's tooling and reviewed in a repository
  // that is not ours; an em dash in a generated banner is a needless variable.
  const { status, out } = render([VERSION], BOTH);
  assert.equal(status, 0);
  for (const [name, files] of Object.entries(FILES)) {
    for (const file of files) {
      const text = read(out, name, file);
      // eslint-disable-next-line no-control-regex
      assert.equal(/[^\x00-\x7F]/.test(text), false, `${file} carries a non-ASCII byte`);
    }
  }
});

test("refuses a version that is not X.Y.Z", () => {
  // winget is a catalogue of stable software, and a dev pre-release is not
  // what someone typing `winget install` is asking for.
  const { status, stderr } = render(["0.8.1-154"], BOTH);
  assert.equal(status, 1);
  assert.match(stderr, /not a stable X\.Y\.Z version/);
});

test("refuses a release without the srectl archive", () => {
  // v0.15.0 and earlier published the terminal UI as srelens-tui only.
  const { status, stderr } = render(
    [VERSION],
    release(
      [`srelens-tui-${VERSION}-x86_64-pc-windows-msvc.zip`, `sha256:${SRECTL_HASH}`],
      [DESKTOP, `sha256:${DESKTOP_HASH}`]
    )
  );
  assert.equal(status, 1);
  assert.match(stderr, /has no srectl-1\.2\.3-x86_64-pc-windows-msvc\.zip/);
});

test("refuses a release without the desktop installer, and writes nothing", () => {
  // All or nothing: half a render on disk is a half submission waiting to
  // happen, and the next run would find the srectl manifests already there.
  const { status, stderr, out } = render(
    [VERSION],
    release([SRECTL, `sha256:${SRECTL_HASH}`])
  );
  assert.equal(status, 1);
  assert.match(stderr, /has no srelens_1\.2\.3_x64-setup\.exe/);
  assert.equal(existsSync(manifests(out, "srectl")), false);
});

test("refuses a digest that is not a sha256", () => {
  // A truncated or mangled digest must stop the render rather than travel
  // into a manifest, where it becomes someone else's failed install.
  const { status, stderr } = render(
    [VERSION],
    release([SRECTL, "sha256:deadbeef"], [DESKTOP, `sha256:${DESKTOP_HASH}`])
  );
  assert.equal(status, 1);
  assert.match(stderr, /GitHub records no sha256 for srectl-1\.2\.3/);
});

test("refuses an asset GitHub recorded no digest for", () => {
  // Assets uploaded before GitHub started computing digests carry null.
  const { status, stderr } = render(
    [VERSION],
    release([SRECTL, `sha256:${SRECTL_HASH}`], [DESKTOP, null])
  );
  assert.equal(status, 1);
  assert.match(stderr, /GitHub records no sha256 for srelens_1\.2\.3_x64-setup\.exe/);
});

test("--package renders that package alone, without the other's asset", () => {
  // A desktop-only submission for a release whose CLI archive is missing or
  // named differently -- v0.15.0 and earlier shipped it as srelens-tui.
  const { status, out } = render(
    [VERSION, "--package", "srelens"],
    release([DESKTOP, `sha256:${DESKTOP_HASH}`])
  );
  assert.equal(status, 0);
  const installer = read(out, "srelens", "srelens.srelens.installer.yaml");
  assert.match(installer, new RegExp(`InstallerSha256: ${"B".repeat(64)}`));
  assert.equal(existsSync(manifests(out, "srectl")), false);
});

test("refuses a --package it does not know", () => {
  const { status, stderr } = render([VERSION, "--package", "srelens-tui"], BOTH);
  assert.equal(status, 1);
  assert.match(stderr, /unknown package 'srelens-tui'; expected srectl or srelens/);
});

test("reports a missing version argument rather than rendering nothing", () => {
  const { status, stderr } = render([], BOTH);
  assert.equal(status, 1);
  assert.match(stderr, /usage: render\.mjs/);
});
