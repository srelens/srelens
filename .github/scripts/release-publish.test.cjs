const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { runInNewContext } = require('node:vm');

const workflow = readFileSync(join(__dirname, '../workflows/release.yml'), 'utf8');
// Only top-level job headers and their single-line needs/if fields. Shell
// bodies are indented further, so they cannot become jobs in this projection.
const jobs = Object.fromEntries([...workflow.matchAll(/^  ([\w-]+):\n([\s\S]*?)(?=^  [\w-]+:|$(?![\s\S]))/gm)]
  .map(([, id, body]) => [id, {
    needs: (body.match(/^    needs: (.+)$/m)?.[1] ?? '').split(/[\s,\[\]]+/).filter(Boolean),
    condition: body.match(/^    if: (.+)$/m)?.[1] ?? '',
  }]));

function ancestors(id, found = new Set()) {
  for (const parent of jobs[id].needs) {
    if (found.has(parent)) continue;
    found.add(parent);
    ancestors(parent, found);
  }
  return found;
}

// Evaluate the workflow's own guards for the string/boolean expressions it
// uses, including GitHub's implicit success() when no status function occurs.
// This is a guard regression, not a full Actions runner simulation.
function enabled(id, { cancelled = false, branch = 'main', release = 'true', publisher = 'success' } = {}) {
  const results = Object.fromEntries(Object.keys(jobs).map(job => [job, 'success']));
  results['release-notes'] = 'skipped'; // normal for a stable main release
  results['publish-release'] = publisher;
  const success = () => [...ancestors(id)].every(job => results[job] === 'success');
  const condition = jobs[id].condition.replace(/^\$\{\{\s*|\s*\}\}$/g, '');
  if (!/\b(always|cancelled|failure|success)\s*\(/.test(condition) && !success()) return false;
  const needs = Object.fromEntries(jobs[id].needs.map(job => [job, { result: results[job], outputs: { release } }]));
  const expression = condition.replace(/needs\.([\w-]+)/g, (_, job) => `needs[${JSON.stringify(job)}]`);
  return runInNewContext(expression, { needs, github: { ref_name: branch }, success, always: () => true, cancelled: () => cancelled });
}

for (const id of ['homebrew', 'aur']) {
  test(`${id} publishes a completed stable release despite skipped release notes`, () => {
    assert.ok(ancestors(id).has('release-notes'), 'exercise the transitive skipped dependency');
    assert.equal(enabled(id), true);
  });
  test(`${id} does not publish cancelled runs, dev builds, or non-releases`, () => {
    assert.equal(enabled(id, { cancelled: true }), false);
    assert.equal(enabled(id, { branch: 'dev' }), false);
    assert.equal(enabled(id, { release: 'false' }), false);
  });
  test(`${id} requires the release to have published successfully`, () => {
    for (const publisher of ['failure', 'skipped', 'cancelled']) assert.equal(enabled(id, { publisher }), false, publisher);
  });
}

test('every job downstream of artifact publication explicitly handles skipped ancestors', () => {
  const downstream = Object.keys(jobs).filter(id => {
    const parents = ancestors(id);
    return parents.has('publish-release') || parents.has('tui-publish');
  });
  assert.ok(downstream.includes('homebrew') && downstream.includes('aur'));
  for (const id of downstream) {
    assert.match(jobs[id].condition, /\b(always|cancelled)\s*\(/, `${id} must not inherit implicit success()`);
  }
});
