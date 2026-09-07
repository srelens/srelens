# Homebrew tap

`brew install srelens/tap/srelens-tui` installs the terminal UI from the
archive the release already publishes.

This directory holds the formula and the script that renders it. The published
copy lives in a separate repository, because that is how Homebrew finds it:
`srelens/tap` resolves to `github.com/srelens/homebrew-tap`, and a tap must be
a repository of its own.

| File | What it is |
| --- | --- |
| `srelens-tui.rb` | The formula, as a template. Version `0.0.0` and zeroed checksums. |
| `render.mjs` | Fills those in from a published release's `SHA256SUMS`. |

Linux installs take the **static musl** archives, not the glibc ones. The
glibc builds are produced on ubuntu-22.04 and ubuntu-24.04-arm, so they
carry a glibc floor of 2.35 and 2.39; Homebrew on Linux supports far older
distributions than that, and a dynamically linked binary would fail before
`main` with a `GLIBC_2.3x` symbol error that tells the user nothing.

A formula, not a cask: this is a command-line binary. The desktop `.dmg` wants
`brew install --cask` and is tracked separately in #225.

## How a release updates it

`.github/workflows/homebrew-publish.yml` runs after a **stable** release is
published, renders the formula for that version, and pushes it to the tap. It
is also `workflow_dispatch`-able for any already-published version, which is
the point of it being a separate workflow — welded to the release pipeline, a
failed push could only be retried by cutting another release. That is how the
AUR package once sat four versions behind.

Stable only. A tap carries one version per formula, and `0.8.1-152` is not
what someone typing `brew install` is asking for.

## Setting it up

Two things this repository cannot do for itself:

1. **The tap repository.** Someone with organisation rights creates a public
   `srelens/homebrew-tap`. The name matters: Homebrew maps `srelens/tap` to
   exactly that, and `brew install srelens/tap/srelens-tui` will not resolve
   without it. A `Formula/` directory is all it needs; the first publish
   creates the file.
2. **A token.** Set `HOMEBREW_TAP_TOKEN` as a repository secret — a
   fine-grained token with **Contents: read and write** on the tap repository
   only. `GITHUB_TOKEN` cannot be used: it is scoped to this repository and
   cannot push to another one.

Until both exist the publish **skips** rather than failing, so releases keep
working. The run log says which one is missing.

Set `HOMEBREW_TAP_REPO` as a repository variable to publish somewhere other
than `srelens/homebrew-tap` — useful for trying the whole path against a
personal tap before pointing it at the real one.

## Rendering by hand

```bash
node packaging/homebrew/render.mjs 1.2.3            # prints the formula
node packaging/homebrew/render.mjs 1.2.3 --out Formula/srelens-tui.rb
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

## The formula and self-update

`srelens-tui update` refuses to overwrite a Homebrew-managed copy and points
back at `brew upgrade` — writing over a file Homebrew tracks would leave its
database describing a version that is no longer there. The formula's caveats
say the same thing from the other side.
