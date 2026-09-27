// Tests for what trust.mjs refuses to sign (#559). Run with:
//   node --test scripts/extensions/trust.test.mjs
//
// A document the script signs but every host refuses is worse than an error: a root
// signed below its threshold is only found out once a build pins it, and a catalog signed
// with its entries missing publishes an empty catalog. The keys are drawn fresh for each
// run, so nothing here can sign what anything trusts.

import { test } from "node:test";
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const script = new URL("./trust.mjs", import.meta.url).pathname;
const freshKey = () => `seed:${randomBytes(32).toString("hex")}`;

function trust(...args) {
  const run = spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
  return { status: run.status, stdout: run.stdout, stderr: run.stderr };
}

test("a root is signed only at its root threshold", () => {
  const [a, b, c, catalog] = [freshKey(), freshKey(), freshKey(), freshKey()];
  const root = (...sign) =>
    trust(
      "root", "--version", "1",
      "--root", a, "--root", b, "--root", c, "--root-threshold", "2",
      "--catalog", catalog, "--catalog-threshold", "1",
      ...sign.flatMap((key) => ["--sign", key]),
    );

  const one = root(a);
  assert.equal(one.status, 1);
  assert.match(one.stderr, /needs signatures from 2 of its --root keys; --sign names 1/);
  assert.equal(one.stdout, "");
  // The same key twice, or a key that is not a root key, does not make up the count.
  assert.equal(root(a, a).status, 1);
  assert.equal(root(a, catalog).status, 1);

  const two = root(a, c);
  assert.equal(two.status, 0, two.stderr);
  const signed = JSON.parse(two.stdout);
  assert.equal(signed.signatures.length, 2);
  assert.equal(JSON.parse(Buffer.from(signed.payload, "base64")).roles.root.threshold, 2);
});

test("a threshold above a role's distinct keys is refused", () => {
  const [a, catalog] = [freshKey(), freshKey()];
  const refused = trust(
    "root", "--version", "1",
    "--root", a, "--root", a, "--root-threshold", "2",
    "--catalog", catalog, "--catalog-threshold", "1",
    "--sign", a,
  );
  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /--root-threshold 2 needs that many distinct --root keys; 1 given/);
});

test("a catalog is signed only with the entries it was given", () => {
  const dir = mkdtempSync(join(tmpdir(), "trust-test-"));
  try {
    const key = freshKey();
    const sign = (source) => {
      const path = join(dir, "catalog.json");
      writeFileSync(path, JSON.stringify(source));
      return trust("catalog", "--in", path, "--version", "1", "--expires", "2100-01-01T00:00:00Z", "--sign", key);
    };
    for (const source of [{ schemaVersion: 1 }, { schemaVersion: 1, extensions: null }, { extensions: {} }, null]) {
      const refused = sign(source);
      assert.equal(refused.status, 1, JSON.stringify(source));
      assert.match(refused.stderr, /--in must be a catalog with an "extensions" array/);
      assert.equal(refused.stdout, "");
    }
    // An empty list is a catalog that lists nothing, said so on purpose.
    for (const extensions of [[], [{ id: "org.example.app" }]]) {
      const signed = sign({ schemaVersion: 1, extensions });
      assert.equal(signed.status, 0, signed.stderr);
      const catalog = JSON.parse(Buffer.from(JSON.parse(signed.stdout).payload, "base64"));
      assert.deepEqual(catalog.extensions, extensions);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
