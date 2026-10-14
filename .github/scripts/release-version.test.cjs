const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { execFileSync } = require('node:child_process');

// Read as LF: a Windows checkout may hold the workflow as CRLF.
const workflow = readFileSync(join(__dirname, '../workflows/release.yml'), 'utf8').replace(/\r\n/g, '\n');
const job = (id) => workflow.split(`\n  ${id}:\n`)[1].split(/\n  [\w-]+:\n/)[0];

test('the installer size report never holds a release back', () => {
  // v0.17.0 built and signed everything, then stayed a draft because its
  // installers had grown past a 15% budget. The sizes are still reported, but
  // nothing in the job can fail on them.
  const sizes = job('size-baseline');
  assert.doesNotMatch(sizes, /--max-growth|exit "\$status"/);
  // Every call into the script is guarded, so its exit status never ends the step.
  const calls = sizes.split('\n').filter((line) => line.includes('size-baseline.mjs'));
  assert.ok(calls.length >= 2, sizes);
  for (const call of calls) assert.match(call, /\|\| (true|echo)/, call);
});

test('the size report keeps the growth against the previous release, and says when it has nothing', () => {
  const sizes = job('size-baseline');
  // Only --check-regression compares with the previous stable release; the
  // plain table compares with the reference client.
  assert.match(sizes, /size-baseline\.mjs --check-regression --tag/);
  assert.match(sizes, /could not be produced/);
});

test('a release is published with this run\'s notes, not the ones its draft was made with', () => {
  // A retry finds the failed run's draft, and tauri-action does not rewrite an
  // existing draft's body.
  const publish = job('publish-release');
  assert.match(publish, /NOTES: \$\{\{ needs\.version\.outputs\.notes \}\}/);
  assert.match(publish, /gh release edit [^\n]*\\\n[^\n]*--notes-file [^\n]* --draft=false|--notes-file[\s\S]*--draft=false/);
});

// The version step's own script, as the workflow runs it on `main`.
const computeVersion = (() => {
  const step = job('version').split('- name: Compute + commit next version\n')[1];
  const lines = step.split('run: |\n')[1].split('\n');
  const indent = lines[0].match(/^ */)[0].length;
  const body = [];
  for (const line of lines) {
    if (line.trim() && line.match(/^ */)[0].length < indent) break;
    body.push(line.slice(indent));
  }
  return body.join('\n').replace(/\$\{\{ github\.ref_name \}\}/g, 'main').replace(/\$\{\{ inputs\.force \}\}/g, '');
})();

/** A repository whose `main` holds `commits` ([subject, version, tag?]), with a bare origin. */
function repository(commits) {
  const dir = mkdtempSync(join(tmpdir(), 'srelens-version-'));
  const git = (...args) => execFileSync('git', args, { cwd: join(dir, 'work'), encoding: 'utf8' }).trim();
  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', join(dir, 'origin.git')]);
  mkdirSync(join(dir, 'work', 'apps', 'desktop', 'src-tauri'), { recursive: true });
  git('init', '-q', '-b', 'main');
  git('config', 'user.name', 'test');
  git('config', 'user.email', 'test@example.com');
  git('remote', 'add', 'origin', join(dir, 'origin.git'));
  for (const [subject, version, tag] of commits) {
    writeFileSync(join(dir, 'work', 'apps', 'desktop', 'src-tauri', 'tauri.conf.json'), `{\n  "version": "${version}"\n}\n`);
    writeFileSync(join(dir, 'work', 'Cargo.toml'), `[workspace.package]\nversion = "${version}"\n`);
    writeFileSync(join(dir, 'work', 'Cargo.lock'), `[[package]]\nname = "srelens-core"\nversion = "${version}"\n`);
    git('add', '-A');
    git('commit', '-q', '--allow-empty', '-m', subject);
    if (tag) git('tag', tag);
  }
  git('push', '-q', 'origin', 'main');
  const run = () => {
    const output = join(dir, 'output');
    writeFileSync(output, '');
    execFileSync('bash', ['-c', computeVersion], {
      cwd: join(dir, 'work'),
      env: { ...process.env, GITHUB_OUTPUT: output },
      stdio: ['ignore', 'ignore', 'inherit'],
    });
    const values = readFileSync(output, 'utf8');
    return { version: values.match(/^version=(.*)$/m)?.[1], release: values.match(/^release=(.*)$/m)?.[1] };
  };
  return { dir, git, run, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

test('a release that failed after its bump is retried at the same version, not bumped past', () => {
  // main at 0.17.0 from a bump commit no release ever tagged: the last stable
  // run built it and stopped short of publishing.
  const repo = repository([
    ['feat: the last release', '0.15.0', 'srelens-v0.15.0'],
    ['feat(tui)!: rename the terminal product', '0.15.0'],
    ['chore(release): srelens-v0.17.0 [skip ci]', '0.17.0'],
    ['fix(release): the fix that lets it publish', '0.17.0'],
  ]);
  try {
    const head = repo.git('rev-parse', 'HEAD');
    assert.deepEqual(repo.run(), { version: '0.17.0', release: 'true' });
    assert.equal(repo.git('rev-parse', 'HEAD'), head, 'no second bump commit');
    assert.equal(repo.git('ls-remote', 'origin', 'main').split('\t')[0], head, 'nothing pushed');
  } finally {
    repo.cleanup();
  }
});

test('a published release is still followed by a bump', () => {
  const repo = repository([
    ['feat: the last release', '0.17.0', 'srelens-v0.17.0'],
    ['fix(ui): a later fix', '0.17.0'],
  ]);
  try {
    assert.deepEqual(repo.run(), { version: '0.17.1', release: 'true' });
    assert.equal(repo.git('log', '-1', '--format=%s'), 'chore(release): srelens-v0.17.1 [skip ci]');
    assert.equal(repo.git('ls-remote', 'origin', 'main').split('\t')[0], repo.git('rev-parse', 'HEAD'));
  } finally {
    repo.cleanup();
  }
});

test('a pending version too small for what landed since is bumped, not retried', () => {
  // A patch release left main at an untagged 1.2.1; a breaking change landed
  // after it. Retrying 1.2.1 would ship that breaking change as a patch.
  const repo = repository([
    ['feat: the last release', '1.2.0', 'srelens-v1.2.0'],
    ['fix(ui): a fix', '1.2.0'],
    ['chore(release): srelens-v1.2.1 [skip ci]', '1.2.1'],
    ['feat(api)!: a breaking change', '1.2.1'],
  ]);
  try {
    assert.deepEqual(repo.run(), { version: '2.0.0', release: 'true' });
    assert.equal(repo.git('log', '-1', '--format=%s'), 'chore(release): srelens-v2.0.0 [skip ci]');
  } finally {
    repo.cleanup();
  }
});
