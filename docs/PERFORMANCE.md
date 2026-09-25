# Performance baselines

A stated reason for building srelens on Tauri rather than Electron is that it
should be smaller and lighter. This page publishes what that is actually worth,
and exactly how it was measured, so the claim can be checked rather than taken
on trust.

Comparisons are against [Freelens](https://github.com/freelensapp/freelens), the
maintained open-source fork of Lens and the closest equivalent Electron client.
srelens is independently developed and not affiliated with it.

## Download size

Every number below is the byte size of a published release asset, read from the
GitHub Releases API — nothing is built locally, estimated, or rounded up from a
directory listing.

Reproduce **this exact table** by pinning both sides; without the tags each
side resolves to whatever is latest, so the numbers move as either project
ships:

```bash
node scripts/perf/size-baseline.mjs --tag srelens-v0.5.0 --comparison-tag v1.10.3
```

For the current comparison instead, drop both flags:

```bash
node scripts/perf/size-baseline.mjs
```

Measured on 2026-08-16, srelens `v0.5.0` against Freelens `v1.10.3`:

| Installer | srelens | Freelens | Difference |
| --- | ---: | ---: | ---: |
| macOS arm64 `.dmg` | 17.1 MiB | 190.2 MiB | 11.2× smaller |
| macOS x64 `.dmg` | 18.3 MiB | 202.3 MiB | 11.1× smaller |
| Windows x64 `.exe` | 11.2 MiB | 154.6 MiB | 13.8× smaller |
| Windows x64 `.msi` | 16.8 MiB | 170.1 MiB | 10.1× smaller |
| Linux x64 `.deb` | 19.8 MiB | 146.8 MiB | 7.4× smaller |
| Linux x64 `.rpm` | 19.8 MiB | 121.8 MiB | 6.2× smaller |
| Linux x64 `.AppImage` | 91.7 MiB | 199.2 MiB | 2.2× smaller |

### Reading these honestly

- **The AppImage gap is much smaller than the rest, and that is expected.** A
  `.deb` or `.rpm` resolves the system WebView (`libwebkit2gtk`) as a package
  dependency; an AppImage has to carry it. That 91.7 MiB is the fair number for
  a self-contained Linux download, and it is the one to quote when comparing
  against Electron's self-contained bundles.
- **Only buckets both projects publish are compared.** srelens ships no arm64
  Linux or Windows installers today, so those rows are absent rather than
  filled with a cross-architecture comparison.
- **Signatures, checksums, SBOMs, and `portable` builds are excluded**, since
  they are not what a user downloads to install the app. Anything the script
  cannot classify is listed in its output rather than silently dropped.

### Regression budget

The release workflow runs this comparison against the **draft** release before
it is published, and blocks publication if any installer grew more than **15%**
over the previous stable release — a failed check leaves the release as an
invisible draft to inspect rather than a public one to retract:

```bash
node scripts/perf/size-baseline.mjs --check-regression --max-growth 15
```

For reference, `v0.4.1 → v0.5.0` grew at most 5.9% (the `.deb`/`.rpm`).

## Memory

`scripts/perf/memory-baseline.sh` measures resident memory across an app's
**whole process tree**. That total is the only fair basis for this comparison:
an Electron app spreads its footprint over a main process plus renderer, GPU,
and utility helpers, while a Tauri app spreads it over its own binary plus the
OS WebView's processes. Measuring a single process would flatter whichever
architecture keeps more of itself in children.

```bash
# start the app, set up the scenario, then:
scripts/perf/memory-baseline.sh srelens --include com.apple.WebKit --settle 30
scripts/perf/memory-baseline.sh Freelens --settle 30
```

The script reports the median of several readings after a settle delay, and
prints every process it counted so contamination is visible rather than
averaged in.

### The macOS WebView caveat

On macOS a Tauri app's WebView runs as `com.apple.WebKit.WebContent`, `.GPU`,
and `.Networking` — XPC services adopted by **launchd**, so their parent is
PID 1, not the app, and their argv names no client. They are neither
descendants of the app nor name matches, so neither of the obvious collection
strategies finds them.

This is not a rounding detail. Spot-checking a dev build while writing this
script, the WebKit services held roughly as much resident memory as the app
process itself — collecting only the app's own processes reported well under
half the real total. (Those were exploratory readings on a debug build, not
baselines; they are quoted only to show the size of the gap.) Since an Electron
app's helpers *are* branded children and get counted, a script that misses
these would systematically flatter the Tauri side.

So the script **refuses to print a macOS total** while unattributed WebKit
processes are running and `--include` was not passed. A valid macOS
measurement therefore requires quitting every other WebKit client (Safari
included) so the remaining WebKit processes can only belong to the app under
test. On Linux the equivalent helpers (`WebKitWebProcess`) are genuine child
processes and need no special handling.

**Numbers are not published here yet.** Producing figures worth citing requires
a run this repository has not yet done: release builds of both apps (a debug
build is not representative), on an otherwise-idle machine, with no stray
`srelens --mcp-stdio` servers running, in two scenarios — idle, and three
clusters connected with watches live. Tracked in
[issue #31](https://github.com/srelens/srelens/issues/31); the size numbers
above are complete and independent of it.

## Extension platform budgets

The extension platform's roadmap targets — manifest loading under 50 ms, the
navigation apps contribute under 10 ms, a typical host call's own overhead under
5 ms — are measured by two test suites described in
[extensions/testing.md](extensions/testing.md#performance-budgets). The
`extension budgets (release)` CI job runs them on every CI run and uploads
the results as the `extension-budgets` artifact, one JSON file per
measurement, so they can be compared across runs.

Measured on 2026-09-25 on an Apple M5 Max, over 50 installed apps (45 copies of
the Argo CD example and 5 of Flux), under two conditions. The host figures are
`cargo test --release` on one test thread. The client figures — the sidebar's
Apps group and the app list — are Vitest in Node 26 with jsdom and no coverage:
the same TypeScript, not the production bundle in the WebView, so they compare
across runs of the suite rather than with what a person waits for. Each figure is
the range of medians over several runs; CI's runners are slower, which is what the
artifact is for.

| Budget | Measured in | Target | Measured |
| --- | --- | ---: | ---: |
| Load 50 apps (`extensions.list`) | host, release | 50 ms | 5.4–9.0 ms |
| Sidebar Apps group from 50 apps | client, Vitest | 10 ms | 0.06–0.11 ms |
| App list: host answer to ready, 50 apps | client, Vitest | 50 ms | 1.4–1.9 ms |
| A host call's own overhead, 1 app installed | host, release | 5 ms | 0.16–0.45 ms |
| A host call's own overhead, 50 apps installed | host, release | 5 ms | **4.9–8.5 ms** |
| Close a view of 6 streams, host side | host, release | 5 ms | 0.01 ms |
| Resolve 1,000 rows over 3 joins, warm | host, release | — | 3.8–10.4 ms |

### Reading these honestly

- **A host call's overhead grows with the inventory.** Every broker call —
  `extensions.read`, `extensions.resource`, `resolveColumns`, `resolveCards`,
  `resolvePanels`, `resolveLinks`, each stream tick — authorizes itself by
  reading and re-validating the whole inventory (`read` in
  `crates/registry/src/extensions.rs`), so its own cost is roughly that of
  loading every app. With one app it is well under a millisecond; with fifty it
  sits at or over the 5 ms target, and every app added re-validates on every
  call. On this machine it is the one target not met at 50 apps.
- **The inventory's size limit comes before its load time.** These 50 apps
  take 709 KB of the inventory's 1 MiB limit (VULN-2 in the
  [threat model](extensions/threat-model.md)), before any kept previous
  versions. Loading is an order of magnitude inside its target; a larger set of
  Flux-sized manifests would reach the limit first.
- **Counts do not vary by machine.** Resolving 1,000 rows lists each joined
  reader once — two lists (four 500-object pages) for three joins on two
  readers, across four concurrent resolves and a refresh. Its CRD check is made
  once per join per call, not once per reader, so two joins on one reader look
  it up twice. A table with live updates resolves twice when it opens: once on
  its own, and once when its watch first lists, because `synced` drops the
  reader's snapshot so nothing that changed between the two is missed.
- **Timings under coverage are not these numbers.** `backend` and `frontend`
  run the same suites as debug builds under coverage instrumentation and hold
  each time only to a ceiling far above its target; their timings are not
  uploaded.

## Methodology notes

- Sizes come from release metadata, so they do not depend on the machine
  running the script. They are reproducible by anyone at any time **when both
  releases are pinned** — an unpinned run reports today's latest releases,
  which is the right default for a fresh comparison but will not reproduce a
  dated table once either project publishes again.
- Set `GITHUB_TOKEN` when running the size script repeatedly; the
  unauthenticated Releases API rate limit is easy to hit.
- Memory readings depend on machine, OS version, and cluster size, so they are
  only meaningful when both apps are measured on the same machine in the same
  session against the same clusters.
