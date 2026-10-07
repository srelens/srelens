const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync, existsSync } = require('node:fs');
const { join } = require('node:path');

const tauriDir = join(__dirname, '../../apps/desktop/src-tauri');
const read = (file) => readFileSync(join(tauriDir, file), 'utf8');
const plistString = (plist, key) =>
  plist.match(new RegExp(`<key>${key.replaceAll('.', '\\.')}</key>\\s*<string>([^<]*)</string>`))?.[1];

// #819: Touch ID needs the App ID entitlement, and macOS refuses to launch an
// app whose entitlement names a bundle other than the one its profile covers.
test('the App ID entitlement names this app\'s bundle identifier and team', () => {
  const { identifier } = JSON.parse(read('tauri.conf.json'));
  const plist = read('Entitlements.plist');
  const team = plistString(plist, 'com.apple.developer.team-identifier');
  assert.match(team ?? '', /^[A-Z0-9]{10}$/);
  assert.equal(plistString(plist, 'com.apple.application-identifier'), `${team}.${identifier}`);
});

// An unsigned or dev build that claims the entitlement has no profile to back
// it and is killed at launch, so only the signed release path may apply it.
test('only signed macOS release builds get the entitlements', () => {
  assert.equal(JSON.parse(read('tauri.conf.json')).bundle.macOS?.entitlements, undefined);
  assert.ok(!existsSync(join(tauriDir, 'tauri.macos.conf.json')), 'tauri.macos.conf.json would apply to every macOS build');

  const signing = JSON.parse(read('macos-signing.conf.json')).bundle.macOS;
  assert.equal(signing.entitlements, './Entitlements.plist');
  assert.equal(signing.files['embedded.provisionprofile'], './embedded.provisionprofile');

  const workflow = readFileSync(join(__dirname, '../workflows/release.yml'), 'utf8');
  const step = workflow.match(/- name: Enable macOS signing[\s\S]*?(?=\n      - name:)/)?.[0] ?? '';
  assert.match(step, /if: matrix\.platform == 'macos-latest' && env\.ENABLE_MAC_SIGNING == 'true'/);
  assert.match(step, /MAC_SIGNED_CONFIG=--config \S*macos-signing\.conf\.json/);
  assert.equal(workflow.match(/MAC_SIGNED_CONFIG=/g)?.length, 1, 'MAC_SIGNED_CONFIG may only be set by the signing step');
  assert.match(workflow, /args: \$\{\{ matrix\.args \}\} \$\{\{ env\.MAC_SIGNED_CONFIG \}\}/);
});
