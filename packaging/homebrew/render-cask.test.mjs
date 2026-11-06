import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { renderBadge, renderCask } from "./render-cask.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const template = readFileSync(join(HERE, "srelens.rb"), "utf8");
const armHash = "a".repeat(64);
const intelHash = "b".repeat(64);

function sums(version = "1.2.3") {
  return [
    `${armHash}  srelens_${version}_aarch64.dmg`,
    `${intelHash}  srelens_${version}_x64.dmg`,
    "",
  ].join("\n");
}

test("renders the two release DMGs onto the matching Homebrew architectures", () => {
  const cask = renderCask({ version: "1.2.3", sums: sums(), template });

  assert.match(cask, /arch arm: "aarch64", intel: "x64"/);
  assert.match(cask, /version "1\.2\.3"/);
  assert.match(cask, new RegExp(`sha256 arm:\\s+"${armHash}",`));
  assert.match(cask, new RegExp(`intel:\\s+"${intelHash}"`));
  assert.match(
    cask,
    /url "https:\/\/github\.com\/srelens\/srelens\/releases\/download\/srelens-v#\{version\}\/srelens_#\{version\}_#\{arch\}\.dmg"/,
  );
  assert.doesNotMatch(cask, /0\.0\.0/);
  assert.doesNotMatch(cask, new RegExp("0".repeat(64)));
  assert.doesNotMatch(cask, new RegExp("1".repeat(64)));
});

test("renders a Shields endpoint whose version is the cask version", () => {
  assert.deepEqual(JSON.parse(renderBadge("1.2.3")), {
    schemaVersion: 1,
    label: "homebrew cask",
    message: "1.2.3",
    color: "14b8a6",
    namedLogo: "homebrew",
  });
});

test("accepts stable semantic versions only", () => {
  for (const version of ["", "v1.2.3", "1.2", "1.2.3-4", "1.2.3+build", "latest"]) {
    assert.throws(
      () => renderCask({ version, sums: sums(), template }),
      /stable X\.Y\.Z version/,
      version || "empty version",
    );
  }
});

test("requires one checksum for each exact release asset", () => {
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: `${armHash}  srelens_1.2.3_aarch64.dmg\n`, template }),
    /does not list srelens_1\.2\.3_x64\.dmg/,
  );
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: `${intelHash}  srelens_1.2.3_x64.dmg\n`, template }),
    /does not list srelens_1\.2\.3_aarch64\.dmg/,
  );
});

test("rejects duplicate checksum records instead of choosing one", () => {
  const duplicate = `${sums()}${armHash}  srelens_1.2.3_aarch64.dmg\n`;
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: duplicate, template }),
    /lists srelens_1\.2\.3_aarch64\.dmg more than once/,
  );
});

test("rejects malformed and unexpected checksum records", () => {
  const malformedHash = sums().replace(armHash, "not-a-sha256");
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: malformedHash, template }),
    /not a sha256/,
  );

  const malformedLine = `${sums()}this-is-not-a-checksum-record\n`;
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: malformedLine, template }),
    /malformed checksum line/,
  );

  const unexpected = `${sums()}${"c".repeat(64)}  srelens_1.2.3_universal.dmg\n`;
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: unexpected, template }),
    /unexpected asset srelens_1\.2\.3_universal\.dmg/,
  );
});

test("rejects a checksum manifest for a different release", () => {
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: sums("1.2.4"), template }),
    /unexpected asset srelens_1\.2\.4_aarch64\.dmg/,
  );
});

test("refuses to publish when a template placeholder survives", () => {
  const extraPlaceholder = template.replace(
    'app "srelens.app"',
    `app "srelens.app"\n  caveats "${"0".repeat(64)}"`,
  );
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: sums(), template: extraPlaceholder }),
    /placeholder survived/,
  );
});

test("refuses a template whose required architecture mapping drifted", () => {
  const swapped = template.replace(
    'arch arm: "aarch64", intel: "x64"',
    'arch arm: "x64", intel: "aarch64"',
  );
  assert.throws(
    () => renderCask({ version: "1.2.3", sums: sums(), template: swapped }),
    /architecture mapping/,
  );
});
