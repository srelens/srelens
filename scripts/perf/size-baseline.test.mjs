// Unit tests for the size-baseline classifier (#31). Run with:
//   node --test scripts/perf/size-baseline.test.mjs
//
// The classifier is what makes the published table trustworthy: a wrong bucket
// would compare a macOS installer against a Linux one, and a silent null would
// drop a platform from the comparison entirely.

import { test } from "node:test";
import assert from "node:assert/strict";
import { bucketAssets, classifyAsset, compareBuckets, describeRatio, growthPct, mib } from "./size-baseline.mjs";

const bucket = (bytes) => ({ name: "x", bytes });

test("classifies srelens release assets", () => {
  assert.equal(classifyAsset("srelens_0.5.0_aarch64.dmg").key, "macOS arm64 .dmg");
  assert.equal(classifyAsset("srelens_0.5.0_x64.dmg").key, "macOS x64 .dmg");
  assert.equal(classifyAsset("srelens_0.5.0_amd64.deb").key, "Linux x64 .deb");
  assert.equal(classifyAsset("srelens-0.5.0-1.x86_64.rpm").key, "Linux x64 .rpm");
  assert.equal(classifyAsset("srelens_0.5.0_amd64.AppImage").key, "Linux x64 .AppImage");
  assert.equal(classifyAsset("srelens_0.5.0_x64-setup.exe").key, "Windows x64 .exe");
  assert.equal(classifyAsset("srelens_0.5.0_x64_en-US.msi").key, "Windows x64 .msi");
});

test("classifies Freelens assets into the same buckets", () => {
  // Different naming scheme, same buckets — otherwise the table would compare
  // artifacts that aren't counterparts.
  assert.equal(classifyAsset("Freelens-1.10.3-macos-arm64.dmg").key, "macOS arm64 .dmg");
  assert.equal(classifyAsset("Freelens-1.10.3-linux-amd64.deb").key, "Linux x64 .deb");
  assert.equal(classifyAsset("Freelens-1.10.3-linux-amd64.AppImage").key, "Linux x64 .AppImage");
  assert.equal(classifyAsset("Freelens-1.10.3-windows-amd64.exe").key, "Windows x64 .exe");
});

test("rejects everything that is not an installer download", () => {
  for (const name of [
    "srelens_0.5.0_amd64.deb.sig",
    "Freelens-1.10.3-linux-amd64.deb.sha256",
    "Freelens-1.10.3-linux-amd64-sbom.spdx.json",
    "latest.json",
    "srelens_x64.app.tar.gz",
    "Freelens-1.10.3-windows-portable-amd64.exe",
    "Packages.xz",
  ]) {
    assert.equal(classifyAsset(name), null, `${name} should not be a bucket`);
  }
});

test("an unclassifiable artifact is reported, not silently dropped", () => {
  const { sizes, skipped } = bucketAssets([
    { name: "srelens_0.5.0_x64.dmg", size: 19149790 },
    { name: "srelens_0.5.0_amd64.deb.sig", size: 412 },
    { name: "mystery-build.tar.zst", size: 100 },
  ]);
  assert.deepEqual(Object.keys(sizes), ["macOS x64 .dmg"]);
  // Signatures are known non-artifacts; the genuinely unknown one surfaces.
  assert.deepEqual(skipped, ["mystery-build.tar.zst"]);
});

test("passes a release within the growth budget", () => {
  const verdict = compareBuckets(
    { "macOS x64 .dmg": bucket(1000), "Linux x64 .deb": bucket(1000) },
    { "macOS x64 .dmg": bucket(1050), "Linux x64 .deb": bucket(1100) },
    15,
  );
  assert.equal(verdict.ok, true);
  assert.equal(verdict.worst, 10);
});

test("fails a release that blows the growth budget", () => {
  const verdict = compareBuckets({ "macOS x64 .dmg": bucket(1000) }, { "macOS x64 .dmg": bucket(1200) }, 15);
  assert.equal(verdict.ok, false);
  assert.match(verdict.reason, /grew 20%/);
});

test("fails CLOSED when a bucket the previous release had disappears", () => {
  // A bundler renaming an installer, or an upload silently dropping one, must
  // not leave that artifact quietly unguarded.
  const verdict = compareBuckets(
    { "macOS x64 .dmg": bucket(1000), "Linux x64 .deb": bucket(1000) },
    { "macOS x64 .dmg": bucket(1000) },
    15,
  );
  assert.equal(verdict.ok, false);
  assert.match(verdict.reason, /Linux x64 \.deb/);
});

test("fails CLOSED when nothing can be compared at all", () => {
  // The dangerous case: if the classifier stops recognizing every artifact,
  // an empty comparison must not report success and let the release through.
  const verdict = compareBuckets({}, {}, 15);
  assert.equal(verdict.ok, false);
  assert.match(verdict.reason, /no installers could be compared/);
});

test("a brand-new installer is reported, not treated as a regression", () => {
  const verdict = compareBuckets(
    { "macOS x64 .dmg": bucket(1000) },
    { "macOS x64 .dmg": bucket(1000), "Linux arm64 .deb": bucket(5000) },
    15,
  );
  assert.equal(verdict.ok, true);
  assert.deepEqual(verdict.appeared, ["Linux arm64 .deb"]);
});

test("mib and growthPct round the way the published table does", () => {
  assert.equal(mib(17882551), 17.1);
  assert.equal(growthPct(18.7, 19.8), 5.9);
  assert.equal(growthPct(100, 90), -10);
  // No previous size means no percentage to report, not a divide-by-zero.
  assert.equal(growthPct(0, 50), null);
});

test("describes the ratio in the correct direction", () => {
  assert.equal(describeRatio(100, 1000), "10× smaller");
  // The case that used to print a misleading "0.8× smaller": if a future
  // comparison release is smaller than ours, say so plainly.
  assert.equal(describeRatio(1000, 800), "25% larger");
  assert.equal(describeRatio(1000, 1000), "same size");
  // Near-parity rounds to 1.0×, which is not a meaningful multiple.
  assert.equal(describeRatio(1000, 1050), "4.8% smaller");
  assert.equal(describeRatio(1000, undefined), "—");
});
