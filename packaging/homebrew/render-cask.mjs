#!/usr/bin/env node
// Render the desktop Homebrew Cask from checksums of the two public release
// DMGs. The caller downloads and hashes the exact assets; this script stays
// network-free so the same inputs are reproducible in tests and by maintainers.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const VERSION_PLACEHOLDER = "0.0.0";
const ARM_SHA_PLACEHOLDER = "0".repeat(64);
const INTEL_SHA_PLACEHOLDER = "1".repeat(64);
const ARCH_MAPPING = 'arch arm: "aarch64", intel: "x64"';
const URL_MAPPING =
  'url "https://github.com/srelens/srelens/releases/download/srelens-v#{version}/srelens_#{version}_#{arch}.dmg"';

function stableVersion(version) {
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`'${version ?? ""}' is not a stable X.Y.Z version; the tap is stable-only`);
  }
}

function replaceExactlyOnce(source, before, after, label) {
  const first = source.indexOf(before);
  if (first === -1 || source.indexOf(before, first + before.length) !== -1) {
    throw new Error(`the cask template must contain exactly one ${label}`);
  }
  return source.slice(0, first) + after + source.slice(first + before.length);
}

function checksumsFor(version, sums) {
  const expected = new Set([
    `srelens_${version}_aarch64.dmg`,
    `srelens_${version}_x64.dmg`,
  ]);
  const found = new Map();

  for (const [index, raw] of String(sums).split("\n").entries()) {
    const line = raw.trim();
    if (!line) continue;

    const record = line.match(/^(\S+)\s+(\*?.+)$/);
    if (!record) throw new Error(`malformed checksum line ${index + 1}: ${line}`);
    const [, hash, rawName] = record;
    const name = rawName.replace(/^\*/, "");

    if (!/^[0-9a-f]{64}$/i.test(hash)) {
      throw new Error(`the checksum listed for ${name} is not a sha256: ${hash}`);
    }
    if (!expected.has(name)) {
      throw new Error(`unexpected asset ${name}; expected only the two DMGs for ${version}`);
    }
    if (found.has(name)) {
      throw new Error(`the checksum manifest lists ${name} more than once`);
    }
    found.set(name, hash.toLowerCase());
  }

  for (const name of expected) {
    if (!found.has(name)) throw new Error(`the checksum manifest does not list ${name}`);
  }
  return found;
}

export function renderCask({ version, sums, template }) {
  stableVersion(version);
  const checksums = checksumsFor(version, sums);
  let cask = String(template).replaceAll("\r", "");

  if (!cask.includes(ARCH_MAPPING) || !cask.includes(URL_MAPPING)) {
    throw new Error("the cask template architecture mapping no longer matches the release asset names");
  }

  cask = replaceExactlyOnce(
    cask,
    `version "${VERSION_PLACEHOLDER}"`,
    `version "${version}"`,
    "version placeholder",
  );
  cask = replaceExactlyOnce(
    cask,
    `arm:   "${ARM_SHA_PLACEHOLDER}"`,
    `arm:   "${checksums.get(`srelens_${version}_aarch64.dmg`)}"`,
    "Apple Silicon checksum placeholder",
  );
  cask = replaceExactlyOnce(
    cask,
    `intel: "${INTEL_SHA_PLACEHOLDER}"`,
    `intel: "${checksums.get(`srelens_${version}_x64.dmg`)}"`,
    "Intel checksum placeholder",
  );

  cask = cask.replace(
    /^# The version and checksums below are placeholders\.[\s\S]*?# and publishes the result to srelens\/homebrew-tap\. See README\.md\.\n/m,
    `# GENERATED for srelens-v${version} — do not edit by hand.\n` +
      "# Rendered from packaging/homebrew/srelens.rb after hashing both public DMGs.\n",
  );

  if (
    cask.includes(VERSION_PLACEHOLDER) ||
    cask.includes(ARM_SHA_PLACEHOLDER) ||
    cask.includes(INTEL_SHA_PLACEHOLDER)
  ) {
    throw new Error("a placeholder survived the cask render — refusing to publish it");
  }
  return cask;
}

export function renderBadge(version) {
  stableVersion(version);
  return `${JSON.stringify(
    {
      schemaVersion: 1,
      label: "homebrew cask",
      message: version,
      color: "14b8a6",
      namedLogo: "homebrew",
    },
    null,
    2,
  )}\n`;
}

function parseArgs(argv) {
  const version = argv[0];
  const values = new Map();
  for (let index = 1; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!new Set(["--sums", "--out", "--badge-out"]).has(name)) {
      throw new Error(`unknown option ${name ?? "<missing>"}`);
    }
    if (!value || value.startsWith("--")) throw new Error(`${name} needs a path`);
    if (values.has(name)) throw new Error(`${name} was provided more than once`);
    values.set(name, value);
  }
  if (!values.has("--sums")) {
    throw new Error("usage: render-cask.mjs <version> --sums <file> [--out <path>] [--badge-out <path>]");
  }
  return { version, values };
}

async function main() {
  const { version, values } = parseArgs(process.argv.slice(2));
  const cask = renderCask({
    version,
    sums: readFileSync(values.get("--sums"), "utf8"),
    template: readFileSync(join(HERE, "srelens.rb"), "utf8"),
  });

  if (values.has("--out")) {
    writeFileSync(values.get("--out"), cask);
    console.error(`wrote ${values.get("--out")} for ${version}`);
  } else {
    process.stdout.write(cask);
  }
  if (values.has("--badge-out")) {
    writeFileSync(values.get("--badge-out"), renderBadge(version));
    console.error(`wrote ${values.get("--badge-out")} for ${version}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`render-cask: ${error.message}`);
    process.exitCode = 1;
  });
}
