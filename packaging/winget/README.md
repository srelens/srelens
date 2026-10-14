# The winget packages

Two packages on Windows:

| Package | Installs | From |
| --- | --- | --- |
| `srelens.srectl` | the CLI, `srectl` | `srectl-<version>-x86_64-pc-windows-msvc.zip` |
| `srelens.srelens` | the desktop app | `srelens_<version>_x64-setup.exe` |

The six manifests here are templates. `render.mjs` fills in the version and
the checksums from a published release, and `winget-publish.yml` opens one
pull request per package into
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).

The web server has no package: it ships as the `ghcr.io/srelens/srelens`
container image, and winget has nothing to install it from.

## This channel is not ours to complete

Every other distribution channel in this repository finishes when our workflow
finishes. This one does not. A submission is a pull request into somebody
else's repository: automated validation runs, then a human moderates it. A
green `winget` job means **asked**, not **published**, and publication lags
the release by hours or more.

That is also why it is a separate workflow. Welded into the release pipeline,
a rejected or stale submission could only be retried by cutting another
release — which is how AUR once sat four versions behind. A manual run takes
a `package` input, so a submission that failed can be retried without opening
a second pull request for the one that went through. The workflow renders
only what it submits (`render.mjs --package`), so a release missing one
package's asset can still submit the other.

## Prerequisites

**`WINGET_TOKEN`** — a classic personal access token with `public_repo`, on an
account that will appear as the submitter. `wingetcreate` uses it to fork
`microsoft/winget-pkgs` into that account, push a branch, and open the pull
request. Nothing is written to this repository, so `GITHUB_TOKEN` cannot do
it. Without the secret the job skips with a note rather than failing a
release.

**The first submission of each package is by hand.** Everything after it is
automatic, but the one that creates a package is worth doing where you can
watch it:

```bash
node packaging/winget/render.mjs 0.16.0 --out winget-out
```

then, on a Windows machine with wingetcreate:

```powershell
wingetcreate submit --token <pat> winget-out/manifests/s/srelens/srectl/0.16.0
wingetcreate submit --token <pat> winget-out/manifests/s/srelens/srelens/0.16.0
```

Expect review comments on a new package that a version bump never gets: the
identifier, the description, the tags. Once both exist in the catalogue, the
release workflow keeps them current on its own.

## What the manifests say

**`srelens.srectl`** is `zip` + `portable`, not an installer. The release
publishes a portable archive with a bare `srectl.exe` inside; winget unpacks
it, registers the command, and puts it on `PATH` through its links directory.
There is no installer to run and nothing lands in Program Files, but a
portable package is still registered so winget can track and remove it -- it
appears in Apps & Features, and `winget uninstall srelens.srectl` removes it.
The identifier is required: `winget uninstall` on its own has nothing to act
on.

**`srelens.srelens`** is `nullsoft`: the NSIS installer the release already
publishes, and the same file the in-app updater installs. Tauri builds it
per-user, so the manifest says `Scope: user` and it installs without an
administrator. The `.msi` the release also carries is not offered; two
installers for one architecture and scope only hand winget a choice nobody
asked for.

Both are x64 only, matching the Windows builds the release makes. An arm64
Windows target means adding it to the installer templates and to `PACKAGES`
in `render.mjs` together.

The checksums are the SHA-256 GitHub records for each release asset, read
from the release API, rather than re-downloading and hashing here —
re-hashing would only prove that whatever this machine received hashes to
itself. The release's `SHA256SUMS.txt` lists the srectl archives only, so it
cannot vouch for the desktop installer; for the srectl archive the two agree.

## Tests

```bash
node --test packaging/winget/render.test.mjs
```

Eleven cases, all offline: they pass `--release` with a release written by the
test, so nothing depends on a release existing or on GitHub being reachable.
They cover a full render of each package, ASCII-only output, and the
refusals -- a pre-release version, a release missing either asset (and
writing nothing when it does), a digest that is not a sha256 or is not there,
and a missing version argument -- and `--package` rendering one package
without needing the other's asset. CI runs them on every pull request.

## Checking a render before submitting

The rendered manifests are ordinary winget manifests, and the client validates
them:

```powershell
winget validate --manifest winget-out\manifests\s\srelens\srectl\0.16.0
winget validate --manifest winget-out\manifests\s\srelens\srelens\0.16.0
```

Installing from them locally needs `winget settings --enable LocalManifestFiles`,
which requires an administrator.
