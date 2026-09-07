# The Linux install script

`install.sh` is what this serves:

```bash
curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh | sh
```

It is served straight from `main` on raw.githubusercontent.com, so **a change
merged to main is live immediately** — there is no release step between this
file and the URL people paste into a root shell. Treat edits accordingly.

## What it does

Detects the architecture, downloads the static musl archive for the latest
stable release, verifies its SHA-256 against the release's own
`SHA256SUMS.txt`, and installs the binary to `/usr/local/bin` or
`~/.local/bin`.

`--version` and `--install-dir` override those two choices. Through a pipe
they need `-s --`, because everything after `sh` belongs to the shell rather
than to the script:

```bash
curl -fsSL .../install.sh | sh -s -- --version 0.9.0 --install-dir ~/bin
```

## Decisions worth not re-litigating

**Always musl, never glibc.** The glibc archives are built on ubuntu-22.04 and
ubuntu-24.04-arm, so they carry a floor of glibc 2.35 and 2.39 — newer than
Debian 11 or RHEL 9, which is exactly the sort of host a cluster gets
administered from. The failure is a `GLIBC_2.3x not found` before `main()`,
which tells the reader nothing actionable. The static build has no floor.

**Unpredictable staging.** The file is created with `mktemp` inside the
destination directory rather than at `.srelens-tui.install.<pid>`. Installed
as root into a directory someone else can write to, a name derived from the
pid can be pre-created as a symlink, and `cp` writes through a destination
symlink — as root, into a file of that user's choosing.

**No sudo.** A script fetched over the network that re-invokes itself as root
is the pattern people are right to be nervous about, and the `~/.local/bin`
fallback needs no privileges. Anyone who wants it system-wide can run the
whole pipeline under `sudo`.

**Everything inside `main()`, called on the last line.** A script read from a
pipe executes as it arrives, so a connection dropped halfway would otherwise
run whatever fragment arrived. With the wrapper, a truncated download is a
syntax error that does nothing.

**Linux only.** macOS gets `brew install srelens/tap/srelens-tui`, which is
already the documented path there; the script says so rather than competing
with it.

**Stable only.** The binary's own `srelens-tui update` carries the dev and
stable channels for anyone who wants a pre-release. Bootstrapping is not the
place for that choice.

## Tests

```bash
sh packaging/install/test.sh
```

Twenty-two cases: argument handling, the macOS and unknown-architecture refusals,
a corrupted archive (which must install nothing), latest-version resolution, a
real install, installing over an existing copy, a run with a PATH that
lacks `sha256sum` so the `shasum` branch is actually taken, the piped
`sh -s --` form the docs tell people to use, and a symlink planted in the
install directory to prove the staging file is not written through it. The refusal cases
put a fake `curl` and `uname` ahead of the real ones on `PATH`.

Only one call reaches the real GitHub API, and the test makes it with a token
when one is present. `install.sh` itself is deliberately unauthenticated --
that is how a stranger runs it -- so leaving the live cases to resolve
`latest` themselves would spend the shared per-IP quota that hosted runners
draw on, and exhausting it would fail this job on unrelated pull requests.

CI runs this and `shellcheck -s sh` on every pull request, under `dash` rather
than bash — the script promises POSIX `sh`, and bash would quietly accept
bashisms that Debian and Alpine users would not.
