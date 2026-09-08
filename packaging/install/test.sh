#!/bin/sh
# Tests for install.sh.
#
#   sh packaging/install/test.sh
#
# Runs on any Linux with curl. The refusal cases put a fake `curl` and a fake
# `uname` ahead of the real ones on PATH, so they need no network beyond the
# one fixture download; the happy-path case really does install the latest
# release into a temp directory and run it.
#
# This exists because an install script is the easiest thing in a repository
# to leave broken: nothing builds it, nothing imports it, and the only person
# who finds out is a stranger following the README.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/install.sh"
[ -f "$script" ] || { echo "install.sh not found next to $0" >&2; exit 1; }

# The fixtures have to match the machine running the suite: the happy path
# executes the binary it downloads, and an x86-64 one will not run on an
# aarch64 developer host without binfmt emulation. CI runs x86-64, so only
# a developer would ever have seen it.
case "$(uname -m)" in
    x86_64 | amd64) host_arch="x86_64" ;;
    aarch64 | arm64) host_arch="aarch64" ;;
    *) echo "these tests have no fixture for $(uname -m)" >&2; exit 1 ;;
esac
host_target="$host_arch-unknown-linux-musl"

work="$(mktemp -d)"

# Some cases need a second account and a shared group to be meaningful.
# Anything this run creates, this run removes: a test suite that leaves a
# login account behind on the host has done more than test.
made_user=""
made_users=""
mounted=""
orig_mode=""
orig_owner=""
orig_group=""
made_group=""
cleanup() {
    # The destination first: an interrupt during the ownership cases would
    # otherwise leave /usr/local/bin world-writable, foreign-owned, or a
    # symlink to a directory this is about to delete.
    reset_dest 2>/dev/null || true
    # Before the rm. A live mount inside $work makes `rm -rf` fail, and
    # under `set -e` the trap would exit there -- leaving the accounts
    # behind AND the tmpfs mounted on a developer's machine.
    [ -z "$mounted" ] || umount "$mounted" >/dev/null 2>&1 || true
    rm -rf "$work"
    [ -z "$made_user" ] || userdel -r "$made_user" >/dev/null 2>&1 || true
    for u in $made_users; do
        userdel -r "$u" >/dev/null 2>&1 || true
    done
    [ -z "$made_group" ] || groupdel "$made_group" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

pass=0
fail=0

check() { # check <name> <expected substring> <output> <exit> <want exit>
    if printf '%s' "$3" | grep -qF "$2" && [ "$4" = "$5" ]; then
        echo "  ok    $1"
        pass=$((pass + 1))
    else
        echo "  FAIL  $1 (exit $4, wanted $5; looked for: $2)"
        printf '%s\n' "$3" | sed 's/^/          /'
        fail=$((fail + 1))
    fi
}

ok() {
    echo "  ok    $1"
    pass=$((pass + 1))
}

no() {
    echo "  FAIL  $1"
    fail=$((fail + 1))
}

# Where an install lands here, now that it cannot be told.
#
# /usr/local/bin when writable -- which for root is always, since
# permission bits do not apply to uid 0 -- and $HOME/.local/bin otherwise.
# So as root the cases below shape /usr/local/bin itself, which is the
# directory a real `curl | sudo sh` installs into.
if [ -w /usr/local/bin ]; then
    default_dest="/usr/local/bin"
else
    default_dest="$HOME/.local/bin"
fi

# Running as root means installing into the real /usr/local/bin and, for
# the destination cases, bending it: world-writable, foreign-owned, ACL'd,
# replaced by a symlink. An interrupt between shaping and restoring would
# leave it that way, and any srelens-tui already there gets overwritten.
#
# Fine in a container, not fine on somebody's machine. Refuse rather than
# do it quietly -- CI runs this inside a container, which is why the root
# pass exists at all.
if [ "$(id -u)" = "0" ] && [ "$default_dest" = "/usr/local/bin" ]; then
    if [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ] &&
        [ -z "${SRELENS_TEST_ALLOW_SYSTEM:-}" ]; then
        echo "Refusing to run as root outside a container." >&2
        echo "" >&2
        echo "These cases install into /usr/local/bin and reshape it -- they make it" >&2
        echo "world-writable, foreign-owned, ACL-bearing, even a symlink -- and an" >&2
        echo "interrupt would leave it that way. Any srelens-tui already there would" >&2
        echo "be overwritten too." >&2
        echo "" >&2
        echo "Run them in a container:" >&2
        echo "  docker run --rm -v \"\$PWD/packaging/install:/i:ro\" debian:bookworm-slim \\" >&2
        echo "    sh -c \"apt-get -qq update && apt-get -qq install -y curl ca-certificates acl \\" >&2
        echo "           libdigest-sha-perl && cp -r /i /tmp/i && sh /tmp/i/test.sh\"" >&2
        echo "" >&2
        echo "or, if this machine is disposable, SRELENS_TEST_ALLOW_SYSTEM=1." >&2
        exit 1
    fi

    # What the directory looked like before any of this, so it can be put
    # back as it was rather than reset to a guess about what it should be.
    orig_mode="$(stat -c %a /usr/local/bin 2>/dev/null)" || orig_mode=""
    orig_owner="$(stat -c %u /usr/local/bin 2>/dev/null)" || orig_owner=""
    orig_group="$(stat -c %g /usr/local/bin 2>/dev/null)" || orig_group=""
fi

# Put the destination back as it was found. Every case that bends it calls
# this afterwards, and so does the exit trap -- an interrupt in the middle
# of the ownership cases would otherwise leave a system directory
# world-writable or belonging to somebody else.
reset_dest() {
    if [ "$default_dest" != "/usr/local/bin" ]; then
        # The home branch: nothing to chown, but the mode and any leftover
        # binary still carry into the next case.
        chmod 0755 "$default_dest" 2>/dev/null || true
        rm -rf "$default_dest/srelens-tui"
        return 0
    fi
    # A case may have moved the directory aside to put a symlink there.
    if [ -L /usr/local/bin ] && [ -d /usr/local/bin.real ]; then
        rm -f /usr/local/bin
        mv /usr/local/bin.real /usr/local/bin
    fi
    setfacl -b /usr/local/bin 2>/dev/null || true
    # The values this run found, not a guess at what a distribution ships.
    [ -z "$orig_owner" ] || chown "$orig_owner" /usr/local/bin 2>/dev/null || true
    [ -z "$orig_group" ] || chgrp "$orig_group" /usr/local/bin 2>/dev/null || true
    [ -z "$orig_mode" ] || chmod "$orig_mode" /usr/local/bin 2>/dev/null || true
    [ -z "$orig_owner" ] || chown "$orig_owner" /usr/local 2>/dev/null || true
    rm -rf /usr/local/bin/srelens-tui
}

# A case that bends the real destination only means something as root; an
# unprivileged run would be installing into its own home instead.
can_shape_dest() {
    [ "$default_dest" = "/usr/local/bin" ]
}

# Somebody who is neither root nor us, for the ownership cases.
other_user="nobody"
id -u nobody >/dev/null 2>&1 || other_user=""

echo "arguments"

out="$(sh "$script" --help 2>&1)" && rc=0 || rc=$?
check "--help explains itself and points macOS at Homebrew" \
    "brew install srelens/tap/srelens-tui" "$out" "$rc" 0

out="$(sh "$script" --nope 2>&1)" && rc=0 || rc=$?
check "an unknown flag is refused, not ignored" "unknown option" "$out" "$rc" 1

out="$(sh "$script" --version 2>&1)" && rc=0 || rc=$?
check "--version without a value is refused" "needs a value" "$out" "$rc" 1

# `--version="$UNSET"` reaches the script as `--version=`. Accepting that
# as "no version given" would silently install the latest release instead
# of the pin the caller asked for.
out="$(sh "$script" --version= 2>&1)" && rc=0 || rc=$?
check "an empty --version= is refused too" "needs a value" "$out" "$rc" 1

# The flag is gone. Saying so beats "unknown option" for anyone following
# an older README.
out="$(sh "$script" --install-dir /tmp/x 2>&1)" && rc=0 || rc=$?
check "--install-dir is refused, and says where it installs instead" "no longer accepted" "$out" "$rc" 1
out="$(sh "$script" --install-dir=/tmp/x 2>&1)" && rc=0 || rc=$?
check "and its equals form too" "no longer accepted" "$out" "$rc" 1

echo "platform"

mkdir -p "$work/fake"
cat > "$work/fake/uname" <<EOF
#!/bin/sh
case "\${1:-}" in
  -s) echo "\${FAKE_OS:-Linux}" ;;
  -m) echo "\${FAKE_ARCH:-$host_arch}" ;;
  *)  echo Linux ;;
esac
EOF
chmod +x "$work/fake/uname"

out="$(FAKE_OS=Darwin PATH="$work/fake:$PATH" sh "$script" 2>&1)" && rc=0 || rc=$?
check "macOS is sent to Homebrew rather than served a Linux binary" \
    "brew install srelens/tap/srelens-tui" "$out" "$rc" 1

out="$(FAKE_ARCH=riscv64 PATH="$work/fake:$PATH" sh "$script" 2>&1)" && rc=0 || rc=$?
check "an architecture with no published build is named" \
    "unsupported architecture: riscv64" "$out" "$rc" 1

echo "checksum"

# A corrupted archive with the release's real checksum file: the one case the
# verification exists for, and the one that must never install anything.
# Authenticated when a token is around (CI), because the unauthenticated
# GitHub API limit is per IP and CI runners share them. The script under test
# deliberately stays unauthenticated — that is how a real user calls it.
if [ -n "${GITHUB_TOKEN:-}" ]; then
    set -- -H "Authorization: Bearer $GITHUB_TOKEN"
else
    set --
fi
version="$(
    curl -fsSL "$@" https://api.github.com/repos/srelens/srelens/releases/latest |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
)"
version="${version#srelens-v}"
[ -n "$version" ] || { echo "could not resolve the latest version" >&2; exit 1; }

archive="srelens-tui-$version-$host_target.tar.gz"
base="https://github.com/srelens/srelens/releases/download/srelens-v$version"
mkdir -p "$work/fixtures"
curl -fsSL -o "$work/fixtures/$archive" "$base/$archive"
curl -fsSL -o "$work/fixtures/SHA256SUMS.txt" "$base/srelens-tui-$version-SHA256SUMS.txt"
cp "$work/fixtures/$archive" "$work/fixtures/good.tar.gz"
printf 'X' | dd of="$work/fixtures/$archive" bs=1 seek=5000 conv=notrunc status=none

cat > "$work/fake/curl" <<EOF
#!/bin/sh
out=""; url=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    -o) out="\$2"; shift 2 ;;
    http*) url="\$1"; shift ;;
    *) shift ;;
  esac
done
case "\$url" in
  *api.github.com*) echo '{"tag_name": "srelens-v$version"}' ;;
  *SHA256SUMS*)     cp "$work/fixtures/SHA256SUMS.txt" "\$out" ;;
  *.tar.gz)
    if [ -n "\${SERVE_GOOD:-}" ]; then
      cp "$work/fixtures/good.tar.gz" "\$out"
    else
      cp "$work/fixtures/\$(basename "\$url")" "\$out"
    fi ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$work/fake/curl"

dest="$default_dest"
out="$(PATH="$work/fake:$PATH" sh "$script" 2>&1)" && rc=0 || rc=$?
check "a corrupted archive is refused" "checksum mismatch" "$out" "$rc" 1
if [ -e "$dest/srelens-tui" ]; then
    no "nothing is installed when the checksum fails"
else
    ok "nothing is installed when the checksum fails"
fi

echo "latest-version resolution"

# Offline, through the fake curl that serves the API's tag_name. Proves the
# script parses `latest` correctly without spending an unauthenticated API
# call per run on a shared runner IP.
dest="$default_dest"
out="$(SERVE_GOOD=1 PATH="$work/fake:$PATH" sh "$script" 2>&1)" && rc=0 || rc=$?
check "the newest release is resolved from the API" "srelens-tui $version" "$out" "$rc" 0
check "and installed" "Installed: $dest/srelens-tui" "$out" "$rc" 0

echo "install"

dest="$default_dest"
out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "the release installs" "Installed: $dest/srelens-tui" "$out" "$rc" 0
check "the checksum is reported, not assumed" "Checksum verified:" "$out" "$rc" 0

if [ -x "$dest/srelens-tui" ] && "$dest/srelens-tui" --version >/dev/null 2>&1; then
    ok "the installed binary runs: $("$dest/srelens-tui" --version)"
else
    no "the installed binary does not run"
fi

# Installing over an existing copy is the update path, and must not fail on
# ETXTBSY or leave the staging file behind.
out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "installing over an existing copy succeeds" "Installed:" "$out" "$rc" 0
if [ -z "$(find "$dest" -name '.srelens-tui.install.*' -o -name '.srelens-tui.backup.*' 2>/dev/null)" ]; then
    ok "no staging or backup file is left behind"
else
    no "a staging or backup file was left in $dest"
fi

echo "through a pipe"

# How the documented one-liner actually runs. Options cannot follow a bare
# `sh` -- it reads them as its own -- so the docs say `sh -s --`, and this
# proves that form reaches the script's parser.
dest="$default_dest"
# The `cat` is the point: this reproduces the documented one-liner, where
# the script arrives on stdin rather than as a path.
# shellcheck disable=SC2002
out="$(cat "$script" | sh -s -- --version "$version" 2>&1)" && rc=0 || rc=$?
check "options survive sh -s --" "Installed: $dest/srelens-tui" "$out" "$rc" 0

echo "staging file"

# The staging name must not be derivable from the pid: installed as root
# into a directory another user can write to, a predictable name can be
# pre-created as a symlink, and cp writes through it. mktemp names cannot
# be aimed at, and a symlink sitting in the directory is left alone.
dest="$default_dest"
echo "do not touch me" > "$work/canary"
ln -sf "$work/canary" "$dest/.srelens-tui.install.99999"
out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "installs alongside a planted symlink" "Installed:" "$out" "$rc" 0
if [ "$(cat "$work/canary")" = "do not touch me" ]; then
    ok "a planted symlink is not written through"
else
    no "the canary was overwritten"
fi
# The pattern is the literal source line, so single quotes are the point.
# shellcheck disable=SC2016
if grep -q 'mktemp "$dir/.$BIN.install.XXXXXX"' "$script"; then
    ok "the staging file is created by mktemp, not from the pid"
else
    no "the staging file is no longer created with mktemp"
fi

echo "unsafe destinations"

# These bend /usr/local/bin itself, which is where a real `curl | sudo sh`
# lands. The destination cannot be named any more, so testing what the rules
# do means shaping the directory they will pick, and putting it back after.
if can_shape_dest; then
    # Whatever the install cases above left behind is not part of these.
    reset_dest

    # An unpredictable staging name does not survive a directory other users
    # can unlink from: they can take the staged file away and leave a symlink,
    # or replace the finished binary before it is run. Only the directory's
    # own permissions close that, so an unsafe one is refused outright.
    chmod 0777 /usr/local/bin
    out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
    check "a world-writable destination is refused" "writable by anyone" "$out" "$rc" 1
    if [ -e /usr/local/bin/srelens-tui ]; then
        no "it installed into the world-writable directory anyway"
    else
        ok "nothing was installed there"
    fi
    reset_dest

    # The sticky bit is what makes /tmp safe: only an entry's owner may unlink
    # it, so the staged file cannot be taken away. That case must still work.
    chmod 1777 /usr/local/bin
    out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
    check "world-writable WITH the sticky bit still installs" "Installed:" "$out" "$rc" 0
    reset_dest

    # A directory belonging to somebody else: they can arrange the swap at
    # leisure and get a root-written file out of it.
    if [ -n "$other_user" ] && chown "$other_user" /usr/local/bin 2>/dev/null; then
        out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a directory owned by another user is refused" "belongs to $other_user" "$out" "$rc" 1
        if [ -e /usr/local/bin/srelens-tui ]; then
            no "it installed into the other user's directory anyway"
        else
            ok "nothing was installed there either"
        fi
    else
        echo "  skip  no second account to own the destination"
    fi
    reset_dest

    # Sticky does not save a directory whose OWNER is somebody else: the owner
    # may remove anything inside it regardless.
    if [ -n "$other_user" ] && chown "$other_user" /usr/local/bin 2>/dev/null; then
        chmod 1777 /usr/local/bin
        out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a sticky directory owned by someone else is refused" "belongs to $other_user" "$out" "$rc" 1
    else
        echo "  skip  no second account to own the destination"
    fi
    reset_dest

    # A symlink is not the directory it points at. `ls -ld` on one reports
    # `lrwxrwxrwx` owned by whoever made the link, so inspecting the path as
    # given would describe the link while the install lands somewhere else.
    target="$work/unsafe-target"
    mkdir -p "$target"
    chmod 0777 "$target"
    mv /usr/local/bin /usr/local/bin.real
    ln -sfn "$target" /usr/local/bin
    out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
    check "a symlink to an unsafe directory is refused" "writable by anyone" "$out" "$rc" 1
    if [ -e "$target/srelens-tui" ]; then
        no "it installed through the symlink anyway"
    else
        ok "nothing was installed through the symlink"
    fi
    rm -f /usr/local/bin
    mv /usr/local/bin.real /usr/local/bin
    reset_dest

    # A safe symlink must still install -- through the directory it points at,
    # named canonically. Approving the resolved path but staging and running
    # through the path as given would leave the link repointable after the
    # check.
    target="$work/real-bin"
    mkdir -p "$target"
    mv /usr/local/bin /usr/local/bin.real
    ln -sfn "$target" /usr/local/bin
    out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
    check "a safe symlink installs into its target" "Installed: $target/srelens-tui" "$out" "$rc" 0
    if [ -x "$target/srelens-tui" ]; then
        ok "the binary landed in the resolved directory"
    else
        no "nothing landed in the resolved directory"
    fi
    rm -f /usr/local/bin
    mv /usr/local/bin.real /usr/local/bin
    reset_dest

    # A directory can be impeccable itself and still sit under one somebody
    # else owns, who can rename it and put their own in its place after the
    # check. Every component is walked, so the ancestor is what fails here.
    if [ -n "$other_user" ] && chown "$other_user" /usr/local 2>/dev/null; then
        out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a directory under a foreign ancestor is refused" "belongs to $other_user" "$out" "$rc" 1
        if [ -e /usr/local/bin/srelens-tui ]; then
            no "it installed under the replaceable ancestor anyway"
        else
            ok "nothing was installed under the replaceable ancestor"
        fi
    else
        echo "  skip  no second account to own an ancestor"
    fi
    reset_dest

    # `mv file dir` moves the file INTO the directory. A destination that is
    # already a directory would swallow the staging file and leave nothing at
    # the path asked for -- and the version line used to hide that inside a
    # command substitution, so the install printed Installed and exited 0.
    mkdir -p /usr/local/bin/srelens-tui
    out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
    check "a directory where the binary goes is refused" "is a directory" "$out" "$rc" 1
    case "$out" in
        *Installed:*) no "it claimed to have installed something" ;;
        *) ok "it did not claim to have installed anything" ;;
    esac
    reset_dest
else
    echo "  skip  not root: the destination is this account's own home, not a directory to bend"
fi

echo "shared groups and ancestors"

if can_shape_dest && command -v groupadd >/dev/null 2>&1; then
    # A group-writable directory is only safe when the group is the owner's
    # own -- the per-user-group convention. A shared group is a set of people
    # who can each replace the binary between staging and running it.
    if ! getent group shared >/dev/null 2>&1; then
        if groupadd shared >/dev/null 2>&1; then
            made_group="shared"
        fi
    fi
    if chgrp shared /usr/local/bin 2>/dev/null; then
        chmod 0775 /usr/local/bin
        out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a directory writable by a shared group is refused" "group shared" "$out" "$rc" 1
    else
        echo "  skip  could not set a shared group"
    fi
    reset_dest

    # A group named after its owner is the per-user-group CONVENTION, not a
    # guarantee. If the group really has other members, any of them can
    # replace the binary, so membership is looked up rather than assumed.
    #
    # Only ever on an account this run created: adding a pre-existing account
    # to group root and removing it again would strip a membership the host
    # meant to have.
    if ! id -u tester >/dev/null 2>&1 && command -v useradd >/dev/null 2>&1; then
        if useradd -m tester >/dev/null 2>&1; then
            made_user="tester"
        fi
    fi
    if [ "$made_user" = "tester" ] && command -v usermod >/dev/null 2>&1 &&
        usermod -aG root tester >/dev/null 2>&1; then
        chmod 0775 /usr/local/bin
        out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "an owner-named group with real members is refused" "besides root" "$out" "$rc" 1
        gpasswd -d tester root >/dev/null 2>&1 || true
    else
        echo "  skip  no account this run created: not touching an existing one's groups"
    fi
    reset_dest

    # Supplementary members are only half of a group: an account whose PRIMARY
    # group it is never appears in the member list, while it can write there
    # perfectly well.
    if [ "$made_user" = "tester" ] && command -v useradd >/dev/null 2>&1; then
        if useradd -M -g root primaryroot >/dev/null 2>&1; then
            # Recorded BEFORE it is used: an interrupt between the useradd and
            # the userdel below would otherwise leave the account behind.
            made_users="$made_users primaryroot"
            chmod 0775 /usr/local/bin
            out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
            check "a group that is someone else's primary group is refused" "primaryroot" "$out" "$rc" 1
            userdel primaryroot >/dev/null 2>&1 || true
            made_users="$(printf %s "$made_users" | sed 's/ primaryroot//')"
        else
            echo "  skip  could not create an account with a primary GID of 0"
        fi
    else
        echo "  skip  no account this run created: cannot test primary-group membership"
    fi
    reset_dest

    # An extended ACL can grant write to any account while the mode bits look
    # impeccable. ls marks one with a trailing +, and reading an ACL portably
    # is not something a POSIX shell can do, so the marker alone is a refusal.
    # shellcheck disable=SC2012  # reading the mode string is the whole point
    # The guard reads the trailing + that `ls -l` puts on a directory with an
    # extended ACL. BusyBox ls does not print it, so there the ACL is invisible
    # to the check and this case has nothing to assert.
    if command -v setfacl >/dev/null 2>&1 && [ -n "$other_user" ] &&
        setfacl -m "u:$other_user:rwx" /usr/local/bin 2>/dev/null &&
        [ "$(ls -ld /usr/local/bin | cut -c11)" = "+" ]; then
        out="$(sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a directory with an extended ACL is refused" "extended ACL" "$out" "$rc" 1
    else
        echo "  skip  no setfacl, or this ls does not mark ACLs (BusyBox)"
    fi
    reset_dest
else
    echo "  skip  not root, or no groupadd: cannot shape the destination's group"
fi

# The per-user-group case must keep working, or every Fedora install with a
# 002 umask breaks: there ~/.local/bin is `alice:alice` mode 0775.
#
# As the user, into their own home -- which is the actual shape, and the one
# branch of the destination choice that root never takes.
if [ "$made_user" = "tester" ]; then
    tester_home="$(getent passwd tester | cut -d: -f6)"
    if [ -n "$tester_home" ] && [ -d "$tester_home" ]; then
        mkdir -p "$tester_home/.local/bin"
        # Group as well as owner: made by root, the tree carries root group,
        # which is not the per-user group this case is about.
        chown -R tester:tester "$tester_home/.local" 2>/dev/null ||
            chown -R tester "$tester_home/.local" 2>/dev/null || true
        chmod 0775 "$tester_home/.local/bin"
        chmod 0644 "$script" 2>/dev/null || true
        chmod 0711 "$work" 2>/dev/null || true
        out="$(su tester -c "sh '$script' --version '$version'" 2>&1)" && rc=0 || rc=$?
        check "group-writable by the owner's own group still installs" "Installed: $tester_home/.local/bin/srelens-tui" "$out" "$rc" 0
    else
        echo "  skip  the created account has no home directory"
    fi
else
    echo "  skip  no account this run created: cannot test a per-user group"
fi

echo "temporary directory"

# mktemp -d honours TMPDIR, and sudo can carry the invoking user's straight
# into a root install. The private tree is only private if its parents are,
# so the destination walk is applied to it as well.
dest="$default_dest"
bad="$work/untrusted-tmp"
mkdir -p "$bad"
chmod 0777 "$bad"
rm -f "$dest/srelens-tui"
out="$(TMPDIR="$bad" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "an untrusted TMPDIR is refused" "writable by anyone" "$out" "$rc" 1
if [ -e "$dest/srelens-tui" ]; then
    no "it installed with the working tree in an untrusted place"
else
    ok "nothing was installed from an untrusted working tree"
fi

# And a TMPDIR that is fine must still work.
good="$work/trusted-tmp"
mkdir -p "$good"
out="$(TMPDIR="$good" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "a trusted TMPDIR still installs" "Installed:" "$out" "$rc" 0

# A hardened host mounts /tmp noexec, and mktemp puts the working tree there.
# Running the binary to check it must not be the thing that fails, so the
# check moves to after the install when the working filesystem forbids exec.
#
# Needs CAP_SYS_ADMIN to mount, so this skips on an ordinary CI runner and
# runs under docker --privileged or a local root shell.
if [ "$(id -u)" = "0" ] && command -v mount >/dev/null 2>&1; then
    noexec="$work/noexec"
    mkdir -p "$noexec"
    if mount -t tmpfs -o rw,noexec,nosuid,size=200m tmpfs "$noexec" 2>/dev/null; then
        # Recorded before use, so an interrupt anywhere below still unmounts.
        mounted="$noexec"
        dest="$default_dest"
        out="$(TMPDIR="$noexec" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a noexec working directory still installs" "Installed:" "$out" "$rc" 0
        check "and says why the check moved" "mounted noexec" "$out" "$rc" 0

        # With the pre-install check skipped, an incompatible binary reaches
        # the destination before anything has run it. Failing then must not
        # leave the caller with nothing where a working copy stood.
        bad="$work/bad"
        mkdir -p "$bad"
        printf 'this is not a binary\n' > "$bad/srelens-tui"
        printf 'nothing here either\n' > "$bad/LICENSE"
        badarchive="srelens-tui-$version-$host_target.tar.gz"
        (cd "$bad" && tar -czf "$work/fixtures/$badarchive.bad" .)
        badsum="$(sha256sum "$work/fixtures/$badarchive.bad" | cut -d" " -f1)"
        printf '%s  %s\n' "$badsum" "$badarchive" > "$work/fixtures/BADSUMS.txt"
        cat > "$work/fake/curl" <<EOF
#!/bin/sh
out=""; url=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    -o) out="\$2"; shift 2 ;;
    http*) url="\$1"; shift ;;
    *) shift ;;
  esac
done
case "\$url" in
  *api.github.com*) echo '{"tag_name": "srelens-v$version"}' ;;
  *SHA256SUMS*)     cp "$work/fixtures/BADSUMS.txt" "\$out" ;;
  *.tar.gz)         cp "$work/fixtures/$badarchive.bad" "\$out" ;;
  *) exit 1 ;;
esac
EOF
        chmod +x "$work/fake/curl"

        out="$(TMPDIR="$noexec" PATH="$work/fake:$PATH" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "a binary that will not run is rejected after install" "does not run" "$out" "$rc" 1
        check "and the previous copy is put back" "put back" "$out" "$rc" 1
        if [ -x "$dest/srelens-tui" ] && "$dest/srelens-tui" --version >/dev/null 2>&1; then
            ok "the working copy survived a failed update"
        else
            no "the working copy was lost"
        fi
        rm -f "$work/fake/curl"

        if umount "$noexec" 2>/dev/null; then
            mounted=""
        fi
    else
        echo "  skip  cannot mount a noexec filesystem here (needs CAP_SYS_ADMIN)"
    fi
else
    echo "  skip  not root, or no mount: cannot test a noexec working directory"
fi

# TMPDIR can be a symlink, and every download, extraction and copy reopens
# $tmp by name. Approving the resolved path but working through the link
# would leave it repointable the moment after it passed, so the resolved
# path is what gets kept -- which means resolution has to see through it.
real_tmp="$work/tmp-target"
mkdir -p "$real_tmp"
chmod 0777 "$real_tmp"
link_tmp="$work/tmp-link"
ln -sfn "$real_tmp" "$link_tmp"
dest="$default_dest"
out="$(TMPDIR="$link_tmp" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "a TMPDIR symlink is resolved before it is judged" "writable by anyone" "$out" "$rc" 1

# A lookup that fails is not a group with nobody in it. These only run for a
# group-writable destination, so shape it that way first -- whichever of the
# two destinations this account gets.
chmod 0775 "$default_dest" 2>/dev/null || true
mkdir -p "$work/fake"
cat > "$work/fake/getent" <<EOF
#!/bin/sh
exit 2
EOF
chmod +x "$work/fake/getent"
dest="$work/getent-down"
mkdir -p "$dest"
chmod 0775 "$dest"
out="$(PATH="$work/fake:$PATH" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "a group lookup that fails is not treated as empty" "cannot look up the group" "$out" "$rc" 1

# And neither is a passwd lookup that fails. In `getent passwd | awk` the
# status belongs to awk, which succeeds on no input, so a partial outage
# would read as "nobody else is in this group".
cat > "$work/fake/getent" <<'EOF'
#!/bin/sh
case "${1:-}" in
  group)  echo "root:x:0:" ;;
  passwd) exit 2 ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$work/fake/getent"
dest="$work/passwd-down"
mkdir -p "$dest"
chmod 0775 "$dest"
out="$(PATH="$work/fake:$PATH" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
check "a passwd lookup that fails is not treated as empty" "cannot enumerate accounts" "$out" "$rc" 1
rm -f "$work/fake/getent"
reset_dest

echo "unpacking"

# The archive carries a `./` member, and GNU tar restores directory
# ownership and permissions from the archive when it runs as root. Into the
# private temp directory itself that would rewrite mktemp -d's 0700 into
# whatever the release runner had; one level down leaves it untouched.
# shellcheck disable=SC2016
if grep -q 'tar -xzf "$tmp/$archive" -C "$tmp/unpack"' "$script"; then
    ok "the archive is unpacked below the private directory, not into it"
else
    no "the archive is unpacked straight into the private directory"
fi

echo "hashing tool"

# Every image this was first tested on has sha256sum, which is why the
# shasum branch could ship computing SHA-1 and pass. Running with a PATH
# that deliberately lacks sha256sum is the only way to take that branch.
if command -v shasum >/dev/null 2>&1; then
    limited="$work/limited"
    mkdir -p "$limited"
    missing=''
    for tool in sh uname curl sed head cut grep tar gzip chmod mktemp dirname mkdir cp mv rm ln cat ls awk id getent shasum; do
        path="$(command -v "$tool" 2>/dev/null)" || { missing="$missing $tool"; continue; }
        ln -sf "$path" "$limited/$tool"
    done
    if [ -n "$missing" ]; then
        echo "  skip  no shasum-only run:$missing not found"
    else
        dest="$default_dest"
        out="$(PATH="$limited" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
        check "shasum computes SHA-256, not SHA-1" "Checksum verified:" "$out" "$rc" 0
        if [ -x "$dest/srelens-tui" ]; then
            ok "the binary installs with only shasum available"
        else
            no "nothing was installed with only shasum available"
        fi
    fi
else
    echo "  skip  shasum is not installed here"
fi

echo
echo "passed $pass, failed $fail"
[ "$fail" -eq 0 ]
