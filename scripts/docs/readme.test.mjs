// The README and its translations (#865).
//
// A translation lives one directory deeper than the README it copies, so
// every relative link in it needs a `../../` the English text does not have.
// Nothing builds these files, and a broken image only shows on GitHub, so
// this checks what a reader would trip over: a missing language, a dead
// link, and a translation that dropped (or kept a stale copy of) a screenshot.
//
//   node --test scripts/docs/readme.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const LANGS = ["zh-CN", "ja", "ko", "es", "pt-BR", "de", "fr"];
const READMES = ["README.md", ...LANGS.map((l) => `docs/i18n/README.${l}.md`)];

/** Every link and image target in a Markdown file, Markdown and HTML alike. */
function targets(text) {
  const md = [...text.matchAll(/\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)].map((m) => m[1]);
  const html = [...text.matchAll(/\b(?:href|src|srcset)="([^"]+)"/g)].map((m) => m[1]);
  return [...md, ...html];
}

const isLocal = (t) => !/^(?:[a-z]+:|#|\/\/)/i.test(t);

/** A local target resolved against the file it appears in, anchor dropped. */
function resolveFrom(file, target) {
  return resolve(ROOT, dirname(file), decodeURI(target.split("#")[0]));
}

function read(file) {
  const path = join(ROOT, file);
  assert.ok(existsSync(path), `${file} is missing`);
  return readFileSync(path, "utf8");
}

test("every README links to every other language", () => {
  for (const file of READMES) {
    const linked = new Set(targets(read(file)).filter(isLocal).map((t) => resolveFrom(file, t)));
    for (const other of READMES) {
      if (other === file) continue;
      assert.ok(linked.has(join(ROOT, other)), `${file} has no link to ${other}`);
    }
  }
});

test("every local link and image in every README resolves", () => {
  for (const file of READMES) {
    for (const t of targets(read(file)).filter(isLocal)) {
      assert.ok(existsSync(resolveFrom(file, t)), `${file} links to ${t}, which does not exist`);
    }
  }
});

test("every translation shows the same screenshots as the English README", () => {
  const shots = (file) =>
    targets(read(file))
      .filter((t) => isLocal(t) && /\.(?:png|webp|gif|jpe?g)$/i.test(t))
      .map((t) => resolveFrom(file, t))
      .sort();
  const english = shots("README.md");
  assert.ok(english.length > 0, "README.md shows no screenshots");
  for (const file of READMES.slice(1)) {
    assert.deepEqual(shots(file), english, `${file} shows different screenshots`);
  }
});
