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
directory listing. Reproduce it with:

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

The release workflow re-runs this comparison after publishing and fails if any
installer grew more than **15%** over the previous stable release:

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
scripts/perf/memory-baseline.sh srelens --settle 30 --samples 5
scripts/perf/memory-baseline.sh Freelens --settle 30 --samples 5
```

The script reports the median of several readings after a settle delay, and
prints every process it counted so contamination is visible rather than
averaged in.

**Numbers are not published here yet.** Producing figures worth citing requires
a run this repository has not yet done: release builds of both apps (a debug
build is not representative), on an otherwise-idle machine, with no stray
`srelens --mcp-stdio` servers running, in two scenarios — idle, and three
clusters connected with watches live. Tracked in
[issue #31](https://github.com/srelens/srelens/issues/31); the size numbers
above are complete and independent of it.

## Methodology notes

- Sizes come from release metadata, so they are reproducible by anyone at any
  time and do not depend on the machine running the script.
- Set `GITHUB_TOKEN` when running the size script repeatedly; the
  unauthenticated Releases API rate limit is easy to hit.
- Memory readings depend on machine, OS version, and cluster size, so they are
  only meaningful when both apps are measured on the same machine in the same
  session against the same clusters.
