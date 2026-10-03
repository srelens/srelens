#!/usr/bin/env node
// Render the winget manifests for a published release.
//
//   node packaging/winget/render.mjs 1.2.3 [--out <dir>] [--sums <path>]
//
// Writes the three files winget wants — version, defaultLocale, installer —
// into `<out>/manifests/s/srelens/srelens-tui/<version>/`, which is the path
// they must occupy inside a microsoft/winget-pkgs checkout. Defaults to
// `./winget-out`.
//
// `--sums` reads the checksums from a file instead of the release, for an
// offline render or to try one before the release is public.
//
// The checksum comes from the release's own SHA256SUMS rather than from
// re-downloading and hashing here, for the same reason the Homebrew renderer
// does it that way: re-hashing a download would only prove that whatever this
// machine received hashes to itself.
//
// Fails rather than guesses. A missing archive, a checksum file that does not
// list it, or a surviving placeholder all stop the render — a manifest with a
// wrong hash fails on a stranger's machine at install time, after moderation
// has already waved it through.

import { writeFileSync, readFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = process.env.GITHUB_REPOSITORY || "srelens/srelens";
const ID = "srelens.srelens-tui";

// The one Windows archive the release builds. Adding an arm64 Windows target
// means adding it here and in the installer template together.
const TARGET = "x86_64-pc-windows-msvc";

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
    die("usage: render.mjs <version> [--out <dir>] [--sums <path>]");
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
  const sumsFile = flag("--sums");

  const base = `https://github.com/${REPO}/releases/download/srelens-v${version}`;
  const archive = `srelens-tui-${version}-${TARGET}.zip`;
  const sumsUrl = `${base}/srelens-tui-${version}-SHA256SUMS.txt`;

  let sums;
  if (sumsFile) {
    sums = readFileSync(sumsFile, "utf8");
  } else {
    const response = await fetch(sumsUrl);
    if (!response.ok) {
      die(
        `${response.status} fetching ${sumsUrl} — is srelens-v${version} published, and does it carry the TUI archives?`
      );
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
      // Uppercase is what wingetcreate writes and what every manifest in
      // winget-pkgs carries. The schema accepts either; matching the corpus
      // keeps our diffs from looking like a hand edit.
      return hash.toUpperCase();
    }
    die(`${sumsUrl} does not list ${file}`);
  }

  // Normalised to LF on read: a checkout on Windows gives the templates CRLF,
  // and every marker below is written with LF. The manifests are written with
  // LF too, which is what winget-pkgs carries.
  const readTemplate = (name) =>
    readFileSync(join(HERE, name), "utf8").split(String.fromCharCode(13)).join("");

  // The committed files say they are templates; the rendered ones say where
  // they came from, since a reviewer in winget-pkgs sees only the result.
  const GENERATED =
    `# GENERATED for srelens-v${version} -- do not edit by hand.\n` +
    `# Rendered from packaging/winget/ in ${REPO} by packaging/winget/render.mjs,\n` +
    `# using that release's published SHA256SUMS.\n`;

  function render(name) {
    let text = readTemplate(name);

    // Strip the template banner (every leading comment line) and replace it.
    text = text.replace(/^(#[^\n]*\n)+/, GENERATED);

    const before = text;
    text = text.split("0.0.0").join(version);
    if (text === before) die(`${name} contains no version placeholder`);

    if (name.includes("installer")) {
      if (!text.includes(PLACEHOLDER_SHA)) {
        die("the installer template no longer contains a checksum placeholder");
      }
      text = text.replace(PLACEHOLDER_SHA, checksumFor(archive));
      if (!text.includes(`${base}/${archive}`)) {
        die(`the installer template does not point at ${archive}`);
      }
    }

    if (text.includes("0.0.0") || text.includes(PLACEHOLDER_SHA)) {
      die(`a placeholder survived the render of ${name} — refusing to publish it`);
    }
    return text;
  }

  // s/srelens/srelens-tui/<version> — winget-pkgs partitions by the publisher's
  // first letter, then identifier, then version.
  const outDir = join(outRoot, "manifests", "s", "srelens", "srelens-tui", version);
  mkdirSync(outDir, { recursive: true });

  for (const name of [
    `${ID}.yaml`,
    `${ID}.locale.en-US.yaml`,
    `${ID}.installer.yaml`,
  ]) {
    const path = join(outDir, name);
    writeFileSync(path, render(name));
    console.error(`wrote ${path}`);
  }

  console.error(`\n${outDir} is ready to submit for ${version}`);
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
