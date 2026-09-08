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

`--version` overrides the version. Through a pipe it needs `-s --`, because
everything after `sh` belongs to the shell rather than to the script:

```bash
curl -fsSL .../install.sh | sh -s -- --version 0.9.0
```

There is no `--install-dir`. It existed and was removed: an arbitrary
caller-chosen destination was most of this script's attack surface, and
making one safe from a POSIX shell means winning filesystem races a shell has
no primitives for — it cannot hold a descriptor across a check and a use, so
every rule below is a claim about a path that could change underneath it. The
two destinations here are root's or yours by construction. Anyone who wants
the binary elsewhere can unpack the tarball, which is a plain `tar -xzf`.

The checks below still run, because both destinations remain reachable from
the environment: `$HOME` is whatever the caller says it is.

## Decisions worth not re-litigating

**Always musl, never glibc.** The glibc archives are built on ubuntu-22.04 and
ubuntu-24.04-arm, so they carry a floor of glibc 2.35 and 2.39 — newer than
Debian 11 or RHEL 9, which is exactly the sort of host a cluster gets
administered from. The failure is a `GLIBC_2.3x not found` before `main()`,
which tells the reader nothing actionable. The static build has no floor.

**An unsafe `/usr/local/bin` falls back, it does not stop the install.**
Writable is not the same as safe, and the difference decides *where* the
binary goes rather than whether it arrives. A GitHub runner ships
`/usr/local/bin` world-writable: the rules refuse that directory and the
install lands in `~/.local/bin` instead. Whichever is chosen is then checked
for real, so an unsafe home still stops it.

**Every component of the destination is checked, not just the leaf.** A
directory can be impeccable itself and still sit under one somebody else
owns, who can rename it and put their own directory at the same path after
the check. The walk is the one `sudo` and `ssh` do over their own paths, and
each component must be:

- owned by root or by you;
- inspectable at all. A component that cannot be read fails closed: an
  inspection that does not answer is not an answer;
- free of an extended ACL, which can grant write where the mode bits show
  none. `getfacl` answers this properly; GNU `ls` answers it with a trailing
  `+`; BusyBox `ls` does not answer it at all, and there the install refuses
  rather than proceeding blind. **On Alpine that means `apk add acl` first** —
  the alternative was letting an ACL through on precisely the systems that
  cannot see it;
- sticky, if either write bit is set **and it is a parent**. Sticky settles
  both at once there: only an entry's owner may unlink it, and `/tmp` is
  `drwxrwxrwt`, so treating either write bit as disqualifying would refuse
  every path running through it. The **destination** gets no such exemption —
  sticky stops another user removing our files, not creating `srelens-tui`
  there first and owning it, after which its mode is copied onto the rollback
  (a planted 4755 becoming a root-owned setuid file) and it can be swapped for
  a symlink to a directory so the `mv` lands underneath it;
- not group-writable at all, unless sticky. Who is really in a group cannot
  be established from a shell: an SSSD or LDAP source resolves accounts one
  at a time while declining to enumerate, so `getent` gives a lower bound
  rather than a fact. This used to try -- member list, then the passwd table
  for primary-group members -- on the grounds that Fedora leaves
  `~/.local/bin` group-writable under a 002 umask. It does not: Fedora 41
  sets `UMASK 022`, and a fresh account gets `drwxr-xr-x`. Thirty-five lines
  defending a case that was not real, and unable to answer the question
  anyway. `alice:alice` is the per-user-group
  convention Fedora leaves on `~/.local/bin` under a 002 umask, but a
  convention is not a guarantee, so the membership is looked up. Accounts
  whose PRIMARY group is that one stay invisible to it — a group-writable
  destination is the weakest check here, and one that is not group-writable
  does not depend on any of it.

The path is resolved with `cd` + `pwd -P` first, and the resolved path is
what staging, the rename and the final version check all use — approving one
path and installing through another leaves a symlink repointable in between.

**A `noexec` working directory is expected, not fatal.** A hardened host
mounts `/tmp` noexec and `mktemp` puts the working tree there, so running the
binary to check it would report every good download as broken. The script
probes whether it can execute anything there at all, and moves the check to
after the install when it cannot -- where a binary that does not run is
removed again -- and the copy it replaced is put back, with the permissions
it had, since that check is the first time the new one could be run at all.

**The working directory gets the same walk.** `mktemp -d` makes the directory
itself 0700 and yours, but places it under `TMPDIR` when that is set -- and
`sudo` can carry the invoking user's `TMPDIR` straight into a root install. A
parent someone else owns can rename the tree after the checksum passes and
put their own binary where the verified one was.

**A directory or a symlink where the binary goes is refused.** `mv file dir`
moves the file *into* a directory rather than over it, and a symlink cannot
be replaced faithfully — the rollback copy is taken by reading the
destination, which follows the link, so a restore would put a regular file
where a link had been while reporting the previous copy was put back.

**Unpredictable staging, and a private unpack.** The staging file is created
with `mktemp` rather than at `.srelens-tui.install.<pid>`, which could be
pre-created as a symlink for `cp` to write through. The archive is unpacked
one level below the private temp directory, never into it: it carries a `./`
member, and GNU tar restores directory ownership and permissions from the
archive when it runs as root, which would rewrite `mktemp -d`'s 0700 into
whatever the release runner had.

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

As root it refuses to run outside a container, and says so. The destination
cases install into the real `/usr/local/bin` and bend it -- world-writable,
foreign-owned, ACL-bearing, briefly a symlink -- and an interrupt would leave
it that way. In a container that is free; on your machine it is not. Run them
the way CI does:

```bash
docker run --rm -v "$PWD/packaging/install:/i:ro" debian:bookworm-slim sh -c '
  apt-get -qq update
  apt-get -qq install -y curl ca-certificates acl libdigest-sha-perl
  cp -r /i /tmp/i && sh /tmp/i/test.sh
'
```

The directory is snapshotted before anything touches it and put back from that
snapshot -- by each case, and by the exit trap, so an interrupt restores it
too. Add `--privileged` to also run the noexec cases, which need to mount a
tmpfs; without it they skip and say so.

Unprivileged, the suite installs into an isolated `HOME` under its own temp
directory rather than your real `~/.local/bin` -- the cases replace whatever
binary is at the destination and delete it afterwards, so running the tests
would otherwise uninstall your own copy.

Sixty-three cases: argument handling, the macOS and unknown-architecture refusals,
a corrupted archive (which must install nothing), latest-version resolution, a
real install, installing over an existing copy, a run with a PATH that
lacks `sha256sum` so the `shasum` branch is actually taken, the piped
`sh -s --` form the docs tell people to use, a symlink planted in the
install directory, and the destination refusals below. The refusal cases
put a fake `curl` and `uname` ahead of the real ones on `PATH`.

Only one call reaches the real GitHub API, and the test makes it with a token
when one is present. `install.sh` itself is deliberately unauthenticated --
that is how a stranger runs it -- so leaving the live cases to resolve
`latest` themselves would spend the shared per-IP quota that hosted runners
draw on, and exhausting it would fail this job on unrelated pull requests.

CI runs this and `shellcheck -s sh` on every pull request, under `dash` rather
than bash — the script promises POSIX `sh`, and bash would quietly accept
bashisms that Debian and Alpine users would not.

It runs the suite twice: once on the runner, and once as root in a container.
Six cases skip without root, and they are the six that matter most — every
rule about who owns the destination needs a second account and a second group
to mean anything. On an unprivileged runner alone, the ownership walk, the ACL
refusal and the group-membership lookup all report `skip`, and the job goes
green having proven none of them.
