#!/usr/bin/env node
// Render the Homebrew formula for a published release.
//
//   node packaging/homebrew/render.mjs 1.2.3 [--out Formula/srelens-tui.rb]
//                                           [--sums path/to/SHA256SUMS.txt]
//
// `--sums` reads the checksums from a file instead of the release, for an
// offline render or to try one before the release is public.
//
// Reads that release's own `srelens-tui-<version>-SHA256SUMS.txt` and writes
// the four URLs and checksums into the template. Deliberately NOT a
// `brew bump-formula-pr`-style fetch-and-hash: the release already publishes
// the checksums, and re-hashing a download here would only prove that whatever
// this machine received hashes to itself.
//
// Fails rather than guesses. A missing archive, a checksum file that does not
// list one, or a hash that is not a hash all stop the render, because a
// formula with a wrong checksum fails on a user's machine at install time with
// nothing pointing back here.

import { writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = process.env.GITHUB_REPOSITORY || "srelens/srelens";

/** The four archives the formula offers, keyed by the placeholder they replace. */
const TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-unknown-linux-gnu",
  "x86_64-unknown-linux-gnu",
];

function die(message) {
  console.error(`render: ${message}`);
  process.exit(1);
}

const version = process.argv[2];
if (!version) die("usage: render.mjs <version> [--out <path>]");
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  // Homebrew carries one version per formula, and a dev pre-release
  // (`0.8.1-152`) is not what someone typing `brew install` is asking for.
  die(`'${version}' is not a stable X.Y.Z version; the tap is stable-only`);
}

const flag = (name) => {
  const at = process.argv.indexOf(name);
  return at === -1 ? null : process.argv[at + 1];
};
const out = flag("--out");
const sumsFile = flag("--sums");

const base = `https://github.com/${REPO}/releases/download/srelens-v${version}`;
const sumsUrl = `${base}/srelens-tui-${version}-SHA256SUMS.txt`;

let sums;
if (sumsFile) {
  sums = readFileSync(sumsFile, "utf8");
} else {
  const response = await fetch(sumsUrl);
  if (!response.ok) {
    die(`${response.status} fetching ${sumsUrl} — is srelens-v${version} published, and does it carry the TUI archives?`);
  }
  sums = await response.text();
}

/** The hash `sha256sum` recorded for one file, in either output format. */
function checksumFor(file) {
  for (const line of sums.split("\n")) {
    const [hash, ...rest] = line.trim().split(/\s+/);
    if (rest.length === 0) continue;
    const name = rest.join(" ").replace(/^\*/, "");
    if (name !== file) continue;
    if (!/^[0-9a-f]{64}$/i.test(hash)) {
      die(`the checksum listed for ${file} is not a sha256: ${hash}`);
    }
    return hash.toLowerCase();
  }
  die(`${sumsUrl} does not list ${file}`);
}

let formula = readFileSync(join(HERE, "srelens-tui.rb"), "utf8");

// Substitute per target, so a placeholder left behind is a bug that shows up
// here rather than as an install failure.
for (const target of TARGETS) {
  const file = `srelens-tui-${version}-${target}.tar.gz`;
  const placeholderUrl = `https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-${target}.tar.gz`;
  if (!formula.includes(placeholderUrl)) {
    die(`the template no longer contains a URL for ${target}`);
  }
  formula = formula.replace(placeholderUrl, `${base}/${file}`);

  // Replace the zeroed checksum that follows THIS url, not any other.
  const marker = `${base}/${file}"\n      sha256 "${"0".repeat(64)}"`;
  if (!formula.includes(marker)) {
    die(`could not find the checksum line for ${target}`);
  }
  formula = formula.replace(marker, `${base}/${file}"\n      sha256 "${checksumFor(file)}"`);
}

formula = formula.replace('version "0.0.0"', `version "${version}"`);

// Nothing rendered may still carry a placeholder.
if (formula.includes("0.0.0") || formula.includes("0".repeat(64))) {
  die("a placeholder survived the render — refusing to publish it");
}

// The committed file explains that it is a template; the published one is not.
formula = formula.replace(
  /^# The version and checksums below are placeholders[\s\S]*?# which is why it names a version that does not exist\.\n/m,
  `# GENERATED for srelens-v${version} — do not edit by hand.\n` +
    "# Rendered from packaging/homebrew/srelens-tui.rb in srelens/srelens by\n" +
    "# packaging/homebrew/render.mjs, using that release's published SHA256SUMS.\n"
);

if (out) {
  writeFileSync(out, formula);
  console.error(`wrote ${out} for ${version}`);
} else {
  process.stdout.write(formula);
}
