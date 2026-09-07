# The winget package

`winget install srelens.srelens-tui` on Windows.

The three manifests here are templates. `render.mjs` fills in the version and
the checksum from a published release, and `winget-publish.yml` opens the pull
request into [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).

## This channel is not ours to complete

Every other distribution channel in this repository finishes when our workflow
finishes. This one does not. A submission is a pull request into somebody
else's repository: automated validation runs, then a human moderates it. A
green `winget` job means **asked**, not **published**, and publication lags
the release by hours or more.

That is also why it is a separate workflow. Welded into the release pipeline,
a rejected or stale submission could only be retried by cutting another
release — which is how AUR once sat four versions behind.

## Prerequisites

**`WINGET_TOKEN`** — a classic personal access token with `public_repo`, on an
account that will appear as the submitter. `wingetcreate` uses it to fork
`microsoft/winget-pkgs` into that account, push a branch, and open the pull
request. Nothing is written to this repository, so `GITHUB_TOKEN` cannot do
it. Without the secret the job skips with a note rather than failing a
release.

**The first submission is by hand.** Everything after it is automatic, but the
one that creates the package is worth doing where you can watch it:

```bash
node packaging/winget/render.mjs 0.9.0 --out winget-out
```

then, on a Windows machine with wingetcreate:

```powershell
wingetcreate submit --token <pat> winget-out/manifests/s/srelens/srelens-tui/0.9.0
```

Expect review comments on a new package that a version bump never gets: the
identifier, the description, the tags. Once `srelens.srelens-tui` exists in
the catalogue, the release workflow keeps it current on its own.

## What the manifests say

`zip` + `portable`, not an installer. The release publishes a portable archive
with a bare `srelens-tui.exe` inside; winget unpacks it, registers the command,
and puts it on `PATH` through its links directory. Nothing appears in
Add/Remove Programs and `winget uninstall` removes it again.

x64 only, matching the single Windows archive the release builds. An arm64
Windows target means adding it to the installer template and to `TARGET` in
`render.mjs` together.

The checksum comes from the release's own `SHA256SUMS.txt` rather than from
re-downloading and hashing here — re-hashing would only prove that whatever
this machine received hashes to itself.

## Tests

```bash
node --test packaging/winget/render.test.mjs
```

Eight cases, all offline: they pass `--sums` with a checksum file written by
the test, so nothing depends on a release existing or on GitHub being
reachable. They cover a full render, the uppercase hash and ASCII-only
output, and the refusals -- a pre-release version, a checksum file that does
not list the archive, a hash that is not a hash, an empty file, and a missing
version argument. CI runs them on every pull request.

## Checking a render before submitting

The rendered manifests are ordinary winget manifests, and the client validates
them:

```powershell
winget validate --manifest winget-out\manifests\s\srelens\srelens-tui\0.9.0
```

Installing from them locally needs `winget settings --enable LocalManifestFiles`,
which requires an administrator.

## Not the desktop app

This is the terminal binary. `winget install srelens` for the desktop app is
[#226](https://github.com/srelens/srelens/issues/226) — a different package
identifier, a different artifact, and an installer rather than a portable
archive.
