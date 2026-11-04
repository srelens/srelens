const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');

const workflow = readFileSync(join(__dirname, '../workflows/homebrew-publish.yml'), 'utf8');
const ci = readFileSync(join(__dirname, '../workflows/ci.yml'), 'utf8');
const cask = readFileSync(join(__dirname, '../../packaging/homebrew/srelens.rb'), 'utf8');
const readme = readFileSync(join(__dirname, '../../README.md'), 'utf8');
const tauri = JSON.parse(readFileSync(join(__dirname, '../../apps/desktop/src-tauri/tauri.conf.json'), 'utf8'));

test('the publish hashes the two exact public desktop DMGs', () => {
  assert.match(workflow, /select\(\.draft == false and \.prerelease == false\) \| \.tag_name/);
  assert.ok(
    workflow.indexOf('select(.draft == false and .prerelease == false)') < workflow.indexOf('gh release download'),
    'the public stable release check must happen before downloading assets',
  );
  assert.match(workflow, /gh release download "srelens-v\$VERSION"/);
  assert.match(workflow, /--pattern "srelens_\$\{VERSION\}_aarch64\.dmg"/);
  assert.match(workflow, /--pattern "srelens_\$\{VERSION\}_x64\.dmg"/);
  assert.match(
    workflow,
    /sha256sum "srelens_\$\{VERSION\}_aarch64\.dmg" "srelens_\$\{VERSION\}_x64\.dmg" > SHA256SUMS/,
  );
  assert.doesNotMatch(cask, /:no_check/);
});

test('the cask installs the desktop product name declared by Tauri', () => {
  assert.match(cask, new RegExp(`app "${tauri.productName}\\.app"`));
});

test('one tap commit publishes the formula, cask, and badge endpoint', () => {
  assert.match(workflow, /node packaging\/homebrew\/render\.mjs "\$VERSION" --out _tap\/Formula\/srectl\.rb/);
  assert.match(
    workflow,
    /node packaging\/homebrew\/render-cask\.mjs "\$VERSION"[\s\S]*--out _tap\/Casks\/srelens\.rb[\s\S]*--badge-out _tap\/badges\/srelens\.json/,
  );
  for (const path of ['Formula/srectl.rb', 'Casks/srelens.rb', 'badges/srelens.json']) {
    assert.match(workflow, new RegExp(`git add ${path.replace('.', '\\.')}($|\\n)`));
  }
  assert.equal(workflow.match(/git commit -m/g)?.length, 1);
  assert.equal(workflow.match(/git push(?:\s|$)/g)?.length, 1);
});

test('the README badge reads the tap-published cask version endpoint', () => {
  assert.match(
    readme,
    /img\.shields\.io\/endpoint\?url=https%3A%2F%2Fraw\.githubusercontent\.com%2Fsrelens%2Fhomebrew-tap%2Fmain%2Fbadges%2Fsrelens\.json/,
  );
});

test('CI runs the offline Homebrew renderer regression suite', () => {
  assert.match(ci, /node --test packaging\/homebrew\/\*\.test\.mjs/);
});
