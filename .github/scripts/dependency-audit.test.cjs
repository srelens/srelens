const { test } = require('node:test');
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');

const workflow = readFileSync(join(__dirname, '../workflows/dependency-audit.yml'), 'utf8').replace(/\r\n/g, '\n');
const ciWorkflow = readFileSync(join(__dirname, '../workflows/ci.yml'), 'utf8').replace(/\r\n/g, '\n');
const CHECKOUT_REVISION = '3d3c42e5aac5ba805825da76410c181273ba90b1';
const SETUP_GO_REVISION = 'b7ad1dad31e06c5925ef5d2fc7ad053ef454303e';
const UPLOAD_SARIF_REVISION = 'b96794f015dfd88f77b49b1c93e0fa7110f94c63';
const OSV_VERSION = 'v2.7.0';
const OSV_REPORTER_REVISION = 'bfe2f84abd2f09fe0b83ba7f6dad40d296762f40';
const OSV_SHA256 = '160c6253e2922e9e0a6a9771c5c4c40a4b3c6090b34a3cdd5e80c96cc2e4218b';
const withoutComments = (text) => text.replace(/^\s*#.*\n/gm, '');

function job(id) {
  const marker = `\n  ${id}:\n`;
  const start = workflow.indexOf(marker);
  assert.notEqual(start, -1, `missing ${id} job`);
  const bodyStart = start + marker.length;
  const next = workflow.slice(bodyStart).search(/^  [\w-]+:\n/m);
  return workflow.slice(start + 1, next === -1 ? undefined : bodyStart + next);
}

function assertExplicitTrackedInputs(body) {
  for (const input of ['Cargo.lock', 'pnpm-lock.yaml', 'go.mod']) {
    assert.ok(body.includes(`':(glob)**/${input}'`), `missing tracked input pathspec for ${input}`);
  }
  assert.match(body, /scan_args\+=\(--lockfile "\$\{parse_as\}:\$\{input\}"\)/);
  for (const parser of ['Cargo.lock', 'pnpm-lock.yaml', 'go.mod']) {
    assert.ok(body.includes(`parse_as='${parser}'`), `missing explicit parser for ${parser}`);
  }
  assert.match(body, /Dependency input path contains a forbidden control character/);
  assert.match(body, /if \(\( input_count == 0 \)\)/);
  for (const workspaceFile of ['go.work', 'go.work.sum']) {
    assert.ok(body.includes(`':(glob)**/${workspaceFile}'`), `missing Go workspace guard for ${workspaceFile}`);
  }
  assert.match(body, /Go workspace overlays are not supported by this audit/);
  for (const unsupported of [
    'package-lock.json',
    'npm-shrinkwrap.json',
    'yarn.lock',
    'bun.lock',
    'bun.lockb',
  ]) {
    assert.ok(body.includes(`':(glob)**/${unsupported}'`), `missing unsupported-input guard for ${unsupported}`);
  }
  assert.match(body, /Unsupported dependency input/);
  assert.doesNotMatch(body, /--recursive/);
}

test('dependency audit runs on every pull request and has a separate full-scan cadence', () => {
  const definition = withoutComments(workflow);
  assert.match(definition, /pull_request:\n\s+branches: \[dev, main, feat\/ui-next\]/);
  assert.doesNotMatch(definition.split('\nconcurrency:')[0], /^\s+paths(?:-ignore)?:/m);
  assert.match(definition, /schedule:\n\s+- cron: ['"][^'"]+['"]/);
  assert.match(definition, /workflow_dispatch:/);
  assert.doesNotMatch(workflow, /pull_request_target:/);
});

test('scanner is a checksum-pinned official release binary, not a mutable container', () => {
  assert.match(workflow, new RegExp(`OSV_SCANNER_VERSION: ${OSV_VERSION.replace('.', '\\.')}`));
  assert.match(workflow, new RegExp(`OSV_SCANNER_SHA256: ${OSV_SHA256}`));
  assert.match(workflow, /releases\/download\/\$\{OSV_SCANNER_VERSION\}\/osv-scanner_linux_amd64/);
  assert.match(workflow, /sha256sum --check --strict/);
  assert.doesNotMatch(workflow, /google\/osv-scanner-action/);
  assert.doesNotMatch(workflow, /docker:\/\//);
  const uses = [...workflow.matchAll(/^\s+uses: ([^\s#]+)(?:\s+#.*)?$/gm)].map((match) => match[1]);
  assert.deepEqual(uses, [
    `actions/checkout@${CHECKOUT_REVISION}`,
    `actions/setup-go@${SETUP_GO_REVISION}`,
    `actions/checkout@${CHECKOUT_REVISION}`,
    `github/codeql-action/upload-sarif@${UPLOAD_SARIF_REVISION}`,
  ]);
  for (const use of uses) {
    assert.match(use, /@[0-9a-f]{40}$/);
  }
});

test('pull requests compare raw base and head scans with the pinned upstream reporter and a read-only token', () => {
  const differential = job('pull-request-diff');
  assert.match(withoutComments(workflow), /\npermissions:\n  contents: read\n\nenv:/);
  assert.match(differential, /if: github\.event_name == 'pull_request'/);
  assert.match(differential, new RegExp(`actions/checkout@${CHECKOUT_REVISION}`));
  assert.match(differential, /permissions:\n\s+contents: read/);
  assert.doesNotMatch(differential, /security-events: write/);
  assert.match(differential, /github\.event\.pull_request\.base\.sha/);
  assert.match(differential, /base_output="\$RUNNER_TEMP\/osv-base\.json"/);
  assert.match(differential, /head_output="\$RUNNER_TEMP\/osv-head\.json"/);
  assert.doesNotMatch(differential, /(?:base|head)_output="\$GITHUB_WORKSPACE/);
  assert.match(differential, /head_sha="\$\(git rev-parse HEAD\)"/);
  assert.match(differential, /scan_tree_root="\$RUNNER_TEMP\/osv-tree"/);
  assert.match(differential, /git worktree add --detach "\$scan_tree_root" "\$BASE_SHA"/);
  assert.match(differential, /git worktree remove --force "\$scan_tree_root"/);
  assert.match(differential, /git worktree add --detach "\$scan_tree_root" "\$head_sha"/);
  assert.match(differential, /scan_tree "\$scan_tree_root" "\$base_output"/);
  assert.match(differential, /scan_tree "\$scan_tree_root" "\$head_output"/);
  assert.doesNotMatch(differential, /scan_tree "\$GITHUB_WORKSPACE"/);
  assert.equal((differential.match(/scan_tree "\$scan_tree_root"/g) || []).length, 2,
    'base and head must be scanned at the same absolute path for Reporter source identity');
  assert.equal((differential.match(/--no-ignore/g) || []).length, 1);
  assert.match(differential, /reject_dependency_symlinks "\$tree"/);
  assert.match(differential, /reject_go_workspace_overlays "\$tree"/);
  assert.match(differential, /reject_unsupported_dependency_inputs "\$tree"/);
  assert.match(differential, /git -C "\$tree" ls-files -s -z/);
  for (const input of ['Cargo.toml', 'Cargo.lock', 'package.json', 'pnpm-lock.yaml', 'go.mod', 'go.sum']) {
    assert.ok(differential.includes(`*/${input}`), `missing symlink guard for ${input}`);
  }
  assertExplicitTrackedInputs(differential);
  assert.match(differential, new RegExp(`actions/setup-go@${SETUP_GO_REVISION}`));
  assert.match(differential, /go-version: ['"]1\.27\.0['"]/);
  assert.match(differential, /cache: false/);
  assert.match(differential, /GOWORK: ['"]off['"]/);
  assert.doesNotMatch(differential, /cargo metadata|CARGO_NET_OFFLINE|CARGO_HOME|dtolnay\/rust-toolchain/);
  assert.match(workflow, new RegExp(`OSV_REPORTER_REVISION: ${OSV_REPORTER_REVISION}`));
  assert.match(differential, /go install "github\.com\/google\/osv-scanner\/v2\/cmd\/osv-reporter@\$\{OSV_REPORTER_REVISION\}"/);
  assert.doesNotMatch(differential, /node \.github\/scripts\/osv-reporter-smoke/);
  assert.match(differential, /OSV Reporter source-path smoke test passed/);
  assert.match(differential, /different-source-control/);
  assert.match(differential, /JSON\.parse\(fs\.readFileSync\(path, "utf8"\)\)/);
  assert.match(differential, /Array\.isArray\(value\.results\)/);
  assert.match(differential, /"\$reporter" --old "\$base_output" --new "\$head_output"/);
  assert.match(differential, /\(\n\s+cd "\$scan_tree_root"\n\s+"\$reporter" --old/);
  assert.match(differential, /--output-files=gh-annotations:#stderr/);
  assert.match(differential, /--fail-on-vuln=true/);
  assert.doesNotMatch(differential, /dependency-audit-diff\.cjs/);
  assert.doesNotMatch(differential, /git show .*dependency-audit/);
  assert.match(differential, /--config "\$empty_config"/);
  assert.doesNotMatch(differential, /--config(?:=|\s+)osv-scanner\.toml/);
});

test('weekly inventory uploads every raw finding without failing on vulnerability debt', () => {
  const full = job('full-inventory');
  assert.match(full, /if: github\.event_name != 'pull_request'/);
  assert.match(full, new RegExp(`actions/checkout@${CHECKOUT_REVISION}`));
  assert.match(full, new RegExp(`github/codeql-action/upload-sarif@${UPLOAD_SARIF_REVISION}`));
  assert.match(withoutComments(full), /permissions:\n\s+actions: read\n\s+contents: read\n\s+security-events: write/);
  assert.match(full, /--format sarif/);
  assert.equal((full.match(/--no-ignore/g) || []).length, 1);
  assert.match(full, /reject_dependency_symlinks "\$GITHUB_WORKSPACE"/);
  assert.match(full, /reject_go_workspace_overlays "\$GITHUB_WORKSPACE"/);
  assert.match(full, /reject_unsupported_dependency_inputs "\$GITHUB_WORKSPACE"/);
  assertExplicitTrackedInputs(full);
  assert.match(full, /--config "\$empty_config"/);
  assert.match(full, /if \[\[ \$status -ne 0 && \$status -ne 1 \]\]/);
  assert.doesNotMatch(full, /--config(?:=|\s+)osv-scanner\.toml/);
});

test('Go workspace overlay guard rejects a tracked go.work file', () => {
  const marker = '          reject_go_workspace_overlays() {';
  const start = workflow.indexOf(marker);
  const end = workflow.indexOf('\n          }\n', start);
  assert.notEqual(start, -1, 'missing Go workspace guard');
  assert.notEqual(end, -1, 'unterminated Go workspace guard');
  const guard = workflow.slice(start, end + '\n          }'.length).replace(/^ {10}/gm, '');
  const repository = mkdtempSync(join(tmpdir(), 'srelens-osv-go-work-'));
  const runGuard = () => execFileSync(
    'bash',
    ['-c', `set -euo pipefail\n${guard}\nreject_go_workspace_overlays "$1"`, 'guard', repository],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
  );

  try {
    execFileSync('git', ['init', '-q'], { cwd: repository });
    assert.doesNotThrow(runGuard);
    writeFileSync(join(repository, 'go.work'), 'go 1.27.0\n');
    execFileSync('git', ['add', 'go.work'], { cwd: repository });
    assert.throws(runGuard, (error) => {
      assert.match(error.stderr, /Go workspace overlays are not supported by this audit: go\.work/);
      return true;
    });
  } finally {
    rmSync(repository, { recursive: true, force: true });
  }
});

test('unsupported lockfile guard rejects tracked npm and Yarn inputs before filtering', () => {
  const marker = '          reject_unsupported_dependency_inputs() {';
  const start = workflow.indexOf(marker);
  const end = workflow.indexOf('\n          }\n', start);
  assert.notEqual(start, -1, 'missing unsupported dependency-input guard');
  assert.notEqual(end, -1, 'unterminated unsupported dependency-input guard');
  const guard = workflow.slice(start, end + '\n          }'.length).replace(/^ {10}/gm, '');
  const repository = mkdtempSync(join(tmpdir(), 'srelens-osv-unsupported-lock-'));
  const runGuard = () => execFileSync(
    'bash',
    ['-c', `set -euo pipefail\n${guard}\nreject_unsupported_dependency_inputs "$1"`, 'guard', repository],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
  );

  try {
    execFileSync('git', ['init', '-q'], { cwd: repository });
    assert.doesNotThrow(runGuard);
    writeFileSync(join(repository, 'yarn.lock'), '# yarn lockfile v1\n');
    execFileSync('git', ['add', 'yarn.lock'], { cwd: repository });
    assert.throws(runGuard, (error) => {
      assert.match(error.stderr, /Unsupported dependency input: yarn\.lock/);
      return true;
    });
  } finally {
    rmSync(repository, { recursive: true, force: true });
  }
});

test('required backend coverage refuses a stale Cargo.lock', () => {
  const marker = '\n  backend:\n';
  const start = ciWorkflow.indexOf(marker);
  assert.notEqual(start, -1, 'missing backend job');
  const bodyStart = start + marker.length;
  const next = ciWorkflow.slice(bodyStart).search(/^  [\w-]+:\n/m);
  const backend = ciWorkflow.slice(start + 1, next === -1 ? undefined : bodyStart + next);
  const freshnessCommands = [
    'cargo metadata --locked --format-version 1 >/dev/null',
    'cargo metadata --locked --manifest-path spikes/sidecar-sandbox/Cargo.toml --format-version 1 >/dev/null',
  ];
  for (const command of freshnessCommands) {
    assert.ok(backend.includes(command), `missing lock freshness check: ${command}`);
    const freshnessCheck = backend.indexOf(command);
    assert.ok(backend.indexOf('dtolnay/rust-toolchain@') < freshnessCheck);
    for (const untrustedCommand of ['pnpm install --frozen-lockfile', 'pnpm build', 'cargo llvm-cov']) {
      assert.ok(freshnessCheck < backend.indexOf(untrustedCommand), `${command} must run before ${untrustedCommand}`);
    }
  }
  const coverageRuns = backend.match(/^\s+cargo llvm-cov .*--no-report.*$/gm) || [];
  assert.equal(coverageRuns.length, 3);
  for (const command of coverageRuns) {
    assert.match(command, /cargo llvm-cov --locked --no-report/);
  }
});

test('the repository keeps one reasoned, expiry-bound local disposition ledger', () => {
  const config = readFileSync(join(__dirname, '../../osv-scanner.toml'), 'utf8');
  const entries = config.split('[[IgnoredVulns]]').slice(1);
  assert.ok(entries.length > 0, 'expected at least one reviewed exception');
  for (const entry of entries) {
    assert.match(entry, /^\s*id = "[^"]+"/m);
    assert.match(entry, /^reason = "[^"]+"/m);
    assert.match(entry, /^ignoreUntil = \d{4}-\d{2}-\d{2}/m);
  }
});
