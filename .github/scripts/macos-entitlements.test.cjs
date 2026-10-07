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
  // Only once the profile is written: the signing config embeds that file,
  // so setting it without the secret fails the build on a missing file.
  assert.match(
    step,
    /if \[ -n "\$APPLE_PROVISIONING_PROFILE" \]; then\n\s*printf [^\n]*> "[^"]*\/embedded\.provisionprofile"\n\s*echo "MAC_SIGNED_CONFIG=--config \S*macos-signing\.conf\.json" >> "\$GITHUB_ENV"\n\s*else\n/,
  );
  assert.equal(workflow.match(/MAC_SIGNED_CONFIG=/g)?.length, 1, 'MAC_SIGNED_CONFIG may only be set by the signing step');
  assert.match(workflow, /args: \$\{\{ matrix\.args \}\} \$\{\{ env\.MAC_SIGNED_CONFIG \}\}/);
});

// Once a release ships the entitlement, a stable build without it (unsigned,
// or signed without the profile) locks out everyone who enabled Touch ID, and
// one whose profile doesn't cover its signature never launches. Neither may
// reach the updater.
test('stable builds require signing and the profile, and every entitled app is checked', () => {
  const workflow = readFileSync(join(__dirname, '../workflows/release.yml'), 'utf8');
  assert.match(workflow, /HAS_MAC_PROFILE: \$\{\{ secrets\.APPLE_PROVISIONING_PROFILE != '' \}\}/);
  const gate = workflow.match(/- name: Require signing and the provisioning profile[\s\S]*?(?=\n      - name:)/)?.[0] ?? '';
  assert.match(
    gate,
    /if: matrix\.platform == 'macos-latest' && github\.ref_name == 'main' && \(env\.ENABLE_MAC_SIGNING != 'true' \|\| env\.HAS_MAC_PROFILE != 'true'\)\n\s*run: \|\n.*::error::.*\n\s*exit 1/,
  );
  assert.ok(workflow.indexOf(gate) < workflow.indexOf('- name: Build app & publish release'), 'the gate must run before the build');
  assert.match(workflow, /if: runner\.os == 'macOS' && env\.MAC_SIGNED_CONFIG != ''\n\s*run: sh packaging\/macos\/check-profile\.sh /);
});
