# Homebrew tap

The tap exposes both ways into srelens on macOS:

```bash
brew install --cask srelens/tap/srelens  # desktop app
brew install srelens/tap/srectl          # terminal UI
```

This directory holds the formula, cask, and scripts that render them. The
published copies live in a separate repository, because that is how Homebrew finds them:
`srelens/tap` resolves to `github.com/srelens/homebrew-tap`, and a tap must be
a repository of its own.

| File | What it is |
| --- | --- |
| `srectl.rb` | The formula, as a template. Version `0.0.0` and zeroed checksums. |
| `render.mjs` | Fills those in from a published release's `SHA256SUMS`. |
| `srelens.rb` | The desktop cask template, with one URL mapping for both macOS architectures. |
| `render-cask.mjs` | Fills the cask from locally computed hashes of the two release DMGs and emits its badge endpoint. |
| `render-cask.test.mjs` | Offline renderer regressions for versions, checksums, placeholders, and architecture mapping. |

Linux installs take the **static musl** archives, not the glibc ones. The
glibc builds are produced on ubuntu-22.04 and ubuntu-24.04-arm, so they
carry a glibc floor of 2.35 and 2.39; Homebrew on Linux supports far older
distributions than that, and a dynamically linked binary would fail before
`main` with a `GLIBC_2.3x` symbol error that tells the user nothing.

The terminal binary is a formula; the desktop `.dmg` is a cask. Both install
the prebuilt release artifacts rather than rebuilding the workspace.

## How a release updates it

`.github/workflows/homebrew-publish.yml` runs after a **stable** release is
public, renders the formula, downloads and hashes the exact Apple Silicon and
Intel DMGs, renders the cask, and pushes all of them to the tap in one commit.
It also publishes `badges/srelens.json` in that commit; the README's Shields
endpoint therefore reports the cask version rather than the unrelated version
from Homebrew's central cask repository.

The workflow is also `workflow_dispatch`-able for any already-published stable
version, which is the point of it being separate — welded to the release
pipeline, a failed push could only be retried by cutting another release. That
is how the AUR package once sat four versions behind.

Stable only. A tap carries one version per formula, and `0.8.1-152` is not
what someone typing `brew install` is asking for.

## Setting it up

Two things this repository cannot do for itself:

1. **The tap repository** — ~~to create~~ **done**: [srelens/homebrew-tap](https://github.com/srelens/homebrew-tap)
   exists, public, default branch `main`. The name matters: Homebrew maps
   `srelens/tap` to exactly that, and `brew install srelens/tap/srectl`
   will not resolve without it. It can stay empty — the first publish creates
   `Formula/srectl.rb` and the branch with it.
2. **A token.** Set `HOMEBREW_TAP_TOKEN` as a repository secret — a
   fine-grained token with **Contents: read and write** on the tap repository
   only. `GITHUB_TOKEN` cannot be used: it is scoped to this repository and
   cannot push to another one.

Until both exist the publish **skips** rather than failing, so releases keep
working, and the run log says which one is missing. Both are checked, not just
the token: setting a tap up is two steps by two different means, so a token
without a repository is the normal middle of doing it rather than a mistake —
and cloning a repository that is not there would fail the release for it. Once
the token is set the skip is a warning rather than silence, since by then
somebody is expecting a publish.

Set `HOMEBREW_TAP_REPO` as a repository variable to publish somewhere other
than `srelens/homebrew-tap` — useful for trying the whole path against a
personal tap before pointing it at the real one.

## Rendering by hand

```bash
node packaging/homebrew/render.mjs 1.2.3            # prints the formula
node packaging/homebrew/render.mjs 1.2.3 --out Formula/srectl.rb

node packaging/homebrew/render-cask.mjs 1.2.3 \
  --sums /path/to/desktop-SHA256SUMS \
  --out Casks/srelens.rb \
  --badge-out badges/srelens.json
```

It reads the checksums the release published rather than downloading the
archives and hashing them: re-hashing here would only prove that whatever this
machine received hashes to itself. It refuses a version that is not `X.Y.Z`, a
release that carries no TUI archives, a checksum file that does not list one,
and any placeholder that survives the substitution — each of which would
otherwise surface as an install failure on someone else's machine with nothing
pointing back here.

`--sums <file>` reads the checksums from a local file instead, for an offline
render or to try one before the release is public.

The cask renderer is deliberately offline-only. Its checksum file must contain
exactly one record for `srelens_<version>_aarch64.dmg` and one for
`srelens_<version>_x64.dmg`; missing, duplicate, malformed, or unexpected
records fail closed. The workflow creates that file only after downloading the
two exact public release assets and hashing them itself. It never uses
Homebrew's `:no_check` escape hatch.

## The formula and self-update

`srectl update` refuses to overwrite a Homebrew-managed copy and points
back at `brew upgrade` — writing over a file Homebrew tracks would leave its
database describing a version that is no longer there. The formula's caveats
say the same thing from the other side.
