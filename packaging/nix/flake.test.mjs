// The flake is the Nix install path. These checks hold the contract a reader
// of docs/INSTALL.md is promised, and the two facts that silently rot:
// a git crate in Cargo.lock with no hash for importCargoLock, and a pnpm
// major the flake does not build with.
//
//   node --test packaging/nix/flake.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const read = (path) => readFileSync(join(root, path), "utf8");

const flake = read("flake.nix");
const install = read("docs/INSTALL.md");

test("the flake publishes the three installable packages", () => {
  for (const name of ["srectl", "srelens", "srelens-server"]) {
    assert.match(flake, new RegExp(`\\b${name}\\b`));
    assert.match(install, new RegExp(`github:srelens/srelens#${name}`));
  }
  assert.match(install, /nix profile upgrade/);
});

test("the package version is the workspace version", () => {
  const cargo = read("Cargo.toml");
  const version = cargo.match(/\[workspace\.package\][\s\S]*?^version = "([^"]+)"/m);
  assert.ok(version, "Cargo.toml has no workspace.package version");
  assert.match(flake, /builtins\.fromTOML \(builtins\.readFile \.\/Cargo\.toml\)/);
  assert.doesNotMatch(flake, new RegExp(`version = "${version[1]}"`));
});

test("the flake packages the exact pnpm version pinned by package.json", () => {
  assert.match(flake, /packageManager = \(builtins\.fromJSON \(builtins\.readFile \.\/package\.json\)\)\.packageManager/);
  assert.match(flake, /pnpmVersion = builtins\.elemAt \(builtins\.match "pnpm@\(\[\^\+\]\+\)/);
  assert.match(flake, /builtins\.getAttr "pnpm_\$\{pnpmMajor\}" pkgs\)\.override/);
  assert.match(flake, /version = pnpmVersion/);
});

test("every git crate in Cargo.lock has an importCargoLock hash", () => {
  const lock = read("Cargo.lock");
  const packages = lock.split("\n[[package]]\n").slice(1);
  const git = [];
  for (const block of packages) {
    const source = block.match(/^source = "git\+[^"]+"/m);
    if (!source) continue;
    const name = block.match(/^name = "([^"]+)"/m)[1];
    const version = block.match(/^version = "([^"]+)"/m)[1];
    git.push(`${name}-${version}`);
  }
  assert.ok(git.length > 0, "Cargo.lock has no git dependency; this check is then asserting nothing");
  for (const id of git) {
    assert.match(flake, new RegExp(`"${id}"`), `${id} is a git crate and flake.nix does not hash it`);
  }
});
