const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { execFileSync } = require('node:child_process');

test('dev manifests wait for published release notes and read the release body', () => {
  const workflow = readFileSync(join(__dirname, '../workflows/release.yml'), 'utf8');
  const job = workflow.split('  updater-manifest:')[1].split('\n  size-baseline:')[0];
  assert.match(job, /needs: \[version, build, release-notes\]/);
  assert.match(job, /NOTES="\$\(gh release view "\$REL_TAG" --repo "\$REPO" --json body --jq '\.body'\)"/);
  assert.ok(job.indexOf('--json body') < job.indexOf('node .github/scripts/compose-latest.cjs'));
});

test('the manifest retains multiline release notes verbatim', () => {
  const dir = mkdtempSync(join(tmpdir(), 'srelens-notes-'));
  const notes = '### Recent changes\n- Fix settings\n- Keep `machine text` intact';
  try {
    mkdirSync(join(dir, 'sigs'));
    writeFileSync(join(dir, 'sigs/srelens_aarch64.app.tar.gz.sig'), 'signature');
    execFileSync(process.execPath, [join(__dirname, 'compose-latest.cjs')], { cwd: dir, env: { ...process.env, REPO: 'srelens/srelens', REL_TAG: 'srelens-v0.10.1-159', VERSION: '0.10.1-159', NOTES: notes } });
    assert.equal(JSON.parse(readFileSync(join(dir, 'latest.json'), 'utf8')).notes, notes);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
