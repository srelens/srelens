#!/usr/bin/env node
// Render the winget manifests for a published release.
//
//   node packaging/winget/render.mjs 1.2.3 [--out <dir>] [--release <path>]
//
// Writes two packages, three files each -- version, defaultLocale, installer
// -- into the paths they must occupy inside a microsoft/winget-pkgs checkout:
//
//   <out>/manifests/s/srelens/srectl/<version>/    srelens.srectl, the CLI
//   <out>/manifests/s/srelens/srelens/<version>/   srelens.srelens, the desktop app
//
// `<out>` defaults to `./winget-out`.
//
// The checksums are the SHA-256 GitHub records for each release asset, read
// from the release API, rather than re-downloading and hashing here -- which
// would only prove that whatever this machine received hashes to itself.
// Not the release's SHA256SUMS: that file lists the srectl archives only, and
// the desktop installer is in none.
//
// `--release` reads the release from a file instead -- the JSON
// `gh api repos/<repo>/releases/tags/srelens-v<version>` prints -- for an
// offline render or to try one before the release is public.
//
// Fails rather than guesses, and writes nothing unless both packages render.
// A missing asset, an asset without a digest, or a surviving placeholder all
// stop the render -- a manifest with a wrong hash fails on a stranger's
// machine at install time, after moderation has already waved it through.

import { writeFileSync, readFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = process.env.GITHUB_REPOSITORY || "srelens/srelens";

// The one Windows build of each the release makes. Adding an arm64 Windows
// target means adding it here and in the installer templates together.
const PACKAGES = [
  { name: "srectl", asset: (v) => `srectl-${v}-x86_64-pc-windows-msvc.zip` },
  { name: "srelens", asset: (v) => `srelens_${v}_x64-setup.exe` },
];

const PLACEHOLDER_SHA = "0".repeat(64);

// Throws rather than calling process.exit(). This renderer runs on a Windows
// runner, and exiting while an undici socket is still open trips a libuv
// assertion there -- which buries the message the caller actually needs under
// a stack trace about async.c, and reports exit 127 instead of 1.
class RenderError extends Error {}

function die(message) {
  throw new RenderError(message);
}

async function main() {
  const version = process.argv[2];
  // A flag in the version slot means the version was omitted, not that someone
  // named a release `--out`. Saying so beats "'--out' is not a stable X.Y.Z
  // version", which reads as though the flag itself were the problem.
  if (!version || version.startsWith("-")) {
    die("usage: render.mjs <version> [--out <dir>] [--release <path>]");
  }
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    // winget is a public catalogue of stable software. A dev pre-release
    // (`0.8.1-152`) is not what someone typing `winget install` is asking for,
    // and moderation would rightly reject a stream of them.
    die(`'${version}' is not a stable X.Y.Z version; winget is stable-only`);
  }

  const flag = (name) => {
    const at = process.argv.indexOf(name);
    return at === -1 ? null : process.argv[at + 1];
  };
  const outRoot = flag("--out") || "winget-out";
  const releaseFile = flag("--release");

  const tag = `srelens-v${version}`;
  const base = `https://github.com/${REPO}/releases/download/${tag}`;
  const releaseUrl = `https://api.github.com/repos/${REPO}/releases/tags/${tag}`;

  let release;
  if (releaseFile) {
    release = JSON.parse(readFileSync(releaseFile, "utf8"));
  } else {
    // A token only lifts the rate limit -- the release is public. Shared CI
    // runners exhaust the anonymous 60 an hour between them.
    const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
    const response = await fetch(releaseUrl, {
      headers: {
        accept: "application/vnd.github+json",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
    });
    if (!response.ok) {
      die(`${response.status} fetching ${releaseUrl} -- is ${tag} published?`);
    }
    release = await response.json();
  }

  /** The sha256 GitHub recorded when the asset was uploaded. */
  function checksumFor(file) {
    const asset = (release.assets || []).find((a) => a.name === file);
    if (!asset) die(`${tag} has no ${file}`);
    const hex = /^sha256:([0-9a-f]{64})$/i.exec(asset.digest || "")?.[1];
    if (!hex) die(`GitHub records no sha256 for ${file}: ${asset.digest}`);
    // Uppercase is what wingetcreate writes and what every manifest in
    // winget-pkgs carries. The schema accepts either; matching the corpus
    // keeps our diffs from looking like a hand edit.
    return hex.toUpperCase();
  }

  // Normalised to LF on read: a checkout on Windows gives the templates CRLF,
  // and every marker below is written with LF. The manifests are written with
  // LF too, which is what winget-pkgs carries.
  const readTemplate = (name) =>
    readFileSync(join(HERE, name), "utf8").split(String.fromCharCode(13)).join("");

  // The committed files say they are templates; the rendered ones say where
  // they came from, since a reviewer in winget-pkgs sees only the result.
  const GENERATED =
    `# GENERATED for ${tag} -- do not edit by hand.\n` +
    `# Rendered from packaging/winget/ in ${REPO} by packaging/winget/render.mjs,\n` +
    `# using the SHA-256 GitHub records for that release's assets.\n`;

  function render(name, asset) {
    let text = readTemplate(name);

    // Strip the template banner (every leading comment line) and replace it.
    text = text.replace(/^(#[^\n]*\n)+/, GENERATED);

    const before = text;
    text = text.split("0.0.0").join(version);
    if (text === before) die(`${name} contains no version placeholder`);

    if (name.endsWith(".installer.yaml")) {
      if (!text.includes(PLACEHOLDER_SHA)) {
        die(`${name} no longer contains a checksum placeholder`);
      }
      text = text.replace(PLACEHOLDER_SHA, checksumFor(asset));
      if (!text.includes(`${base}/${asset}`)) {
        die(`${name} does not point at ${asset}`);
      }
    }

    if (text.includes("0.0.0") || text.includes(PLACEHOLDER_SHA)) {
      die(`a placeholder survived the render of ${name} — refusing to publish it`);
    }
    return text;
  }

  // Every file rendered before any is written: half a render on disk is half
  // a submission waiting to happen.
  const files = [];
  for (const { name, asset } of PACKAGES) {
    const id = `srelens.${name}`;
    // s/srelens/<name>/<version> -- winget-pkgs partitions by the publisher's
    // first letter, then identifier, then version.
    const dir = join(outRoot, "manifests", "s", "srelens", name, version);
    for (const file of [`${id}.yaml`, `${id}.locale.en-US.yaml`, `${id}.installer.yaml`]) {
      files.push([dir, file, render(file, asset(version))]);
    }
  }

  for (const [dir, file, text] of files) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, file), text);
    console.error(`wrote ${join(dir, file)}`);
  }

  console.error(`\nsrelens.srectl and srelens.srelens ${version} are ready to submit`);
}

try {
  await main();
} catch (error) {
  if (!(error instanceof RenderError)) throw error;
  console.error(`render: ${error.message}`);
  // Not process.exit(): see RenderError above. Node leaves when the loop
  // drains, carrying this code with it.
  process.exitCode = 1;
}
