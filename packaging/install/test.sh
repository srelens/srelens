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
trap cleanup EXIT
trap 'cleanup; exit 129' HUP
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

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

# An isolated HOME, before anything reads it.
#
# The unprivileged branch of the destination choice is $HOME/.local/bin, and
# these cases install for real: they replace whatever srelens-tui is there,
# and reset_dest deletes it afterwards. Pointed at a developer's own home,
# running the tests would uninstall their copy. Pointed here, the same cases
# run against a directory that goes away with $work.
HOME="$work/home"
export HOME
mkdir -p "$HOME"

# The installer refuses to install anywhere it cannot check for extended
# ACLs, which needs either getfacl or a GNU ls. On BusyBox without the acl
# package neither exists, so every case here would fail for that one reason.
# Say so once instead.
# shellcheck disable=SC2012  # asking ls what it is, not listing anything
if ! command -v getfacl >/dev/null 2>&1 &&
    ! ls --version 2>/dev/null | head -n 1 | grep -q coreutils; then
    echo "This host cannot inspect extended ACLs: no getfacl, and an ls that" >&2
    echo "does not identify itself as GNU coreutils. The installer refuses to" >&2
    echo "install anywhere under those conditions, so every case below would" >&2
    echo "fail for that reason alone." >&2
    echo "" >&2
    echo "Install the acl package first (on Alpine: apk add acl)." >&2
    exit 1
fi

# Same for getfattr, when running as root. A privileged install refuses to
# REPLACE a binary whose extended attributes it cannot read -- a file
# capability is the one that matters, and only root can put one there -- and
# this suite installs over itself repeatedly, so every update case would fail
# for that one reason.
if [ "$(id -u)" = "0" ] && ! command -v getfattr >/dev/null 2>&1; then
    echo "Running as root, but getfattr is not installed." >&2
    echo "" >&2
    echo "A privileged install refuses to replace a binary whose extended" >&2
    echo "attributes it cannot read, and these cases install over themselves," >&2
    echo "so every update below would fail for that reason alone." >&2
    echo "" >&2
    echo "Install the attr package first (Debian: apt install attr; Alpine: apk add attr)." >&2
    exit 1
fi

# Where an install lands here, now that it cannot be told.
#
# /usr/local/bin when writable -- which for root is always, since
# permission bits do not apply to uid 0 -- and $HOME/.local/bin otherwise.
# So as root the cases below shape /usr/local/bin itself, which is the
# directory a real `curl | sudo sh` installs into.
# Asked of the installer rather than worked out again here. A version that
# does not exist is enough: the destination is named before anything is
# downloaded, so this costs one 404 rather than an archive.
#
# `writable` is not the whole rule: a world-writable /usr/local/bin, which is
# what a GitHub runner has, is writable and refused, and the install falls
# back to the home directory. Working that out a second time here is how the
# two drift apart.
default_dest="$(sh "$script" --version 0.0.1 2>&1 |
    sed -n 's/^Installing .* into \(.*\)$/\1/p' | head -n 1)"
[ -n "$default_dest" ] || {
    echo "could not work out where the installer would install" >&2
    exit 1
}

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
        echo "" >&2
        echo "  docker run --rm -v \"\$PWD/packaging/install:/i:ro\" \\" >&2
        echo "    debian:bookworm-slim sh -c '" >&2
        echo "      apt-get -qq update" >&2
        echo "      apt-get -qq install -y curl ca-certificates acl attr libcap2-bin libdigest-sha-perl" >&2
        echo "      cp -r /i /tmp/i && sh /tmp/i/test.sh" >&2
        echo "    '" >&2
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
    [ "$default_dest" = "/usr/local/bin" ] && [ "$(id -u)" = "0" ]
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

echo "how the docs say to run it"

# The form the docs show: mktemp for the name, chained so a failure carries.
# A fixed name in a shared directory can be pre-created as a symlink for
# `curl -o` to truncate; unchained, a failed download leaves the previous file
# to be run and a successful rm ends the snippet at status 0.
chain_rc=0
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT && curl -fsSL "https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/no-such-file.sh" -o "$f" && sh "$f" >/dev/null 2>&1 ) || chain_rc=$?
if [ "$chain_rc" != "0" ]; then
    ok "the chained form fails when the download fails, exit $chain_rc"
else
    no "the chained form reported success on a failed download"
fi

# The documented form is download-then-run, not `curl | sh`. A pipeline
# reports its LAST command, so a download that fails leaves sh reading an
# empty script, doing nothing, and exiting 0 -- the line succeeds having
# installed nothing, and anything automated around it carries on.
missing="https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/no-such-file.sh"
piped_rc=0
curl -fsSL "$missing" 2>/dev/null | sh >/dev/null 2>&1 || piped_rc=$?
if [ "$piped_rc" = "0" ]; then
    ok "the piped form hides a failed download, which is why the docs do not use it"
else
    no "expected the piped form to report success on a failed download"
fi

chain_rc=0
curl -fsSL "$missing" -o "$work/should-not-exist.sh" 2>/dev/null && sh "$work/should-not-exist.sh" >/dev/null 2>&1 || chain_rc=$?
if [ "$chain_rc" != "0" ]; then
    ok "the documented form reports it, exit $chain_rc"
else
    no "the documented form swallowed a failed download too"
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

# These run as an unprivileged account with a HOME under $work.
#
# /usr/local/bin is not writable for that account, so the home branch of the
# destination choice is the one taken -- which means every rule about the
# destination can be exercised against a directory this suite owns, instead of
# by bending a system one. Root cannot do it: permission bits do not apply to
# uid 0, so root always writes /usr/local/bin whatever its mode.
if [ "$made_user" != "tester" ] && [ "$(id -u)" = "0" ] &&
    command -v useradd >/dev/null 2>&1; then
    if useradd -m tester >/dev/null 2>&1; then
        made_user="tester"
    fi
fi

# Create a HOME for one case, owned by the account that will install into it.
new_home() {
    rm -rf "$1"
    mkdir -p "$1/.local/bin"
    chown -R tester "$1" 2>/dev/null || true
    chgrp -R tester "$1" 2>/dev/null || true
}

# Run the installer as that account, with that HOME.
install_into() {
    ihome="$1"
    shift
    chmod 0711 "$work" 2>/dev/null || true
    chmod 0644 "$script" 2>/dev/null || true
    # Anything left is environment for the run, e.g. a PATH carrying a fake
    # command ahead of the real one.
    su tester -c "$* HOME='$ihome' sh '$script' --version '$version'" 2>&1
}

if [ "$made_user" = "tester" ]; then
    # The plain case first: an ordinary home, an ordinary install.
    home="$work/homes/plain"
    new_home "$home"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "an ordinary home installs" "Installed: $home/.local/bin/srelens-tui" "$out" "$rc" 0

    # An unpredictable staging name does not survive a directory other users
    # can unlink from: they can take the staged file away and leave a symlink,
    # or replace the finished binary before it is run. Only the directory's
    # own permissions close that, so an unsafe one is refused outright.
    home="$work/homes/world"
    new_home "$home"
    chmod 0777 "$home/.local/bin"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a world-writable destination is refused" "writable by other users" "$out" "$rc" 1
    if [ -e "$home/.local/bin/srelens-tui" ]; then
        no "it installed into the world-writable directory anyway"
    else
        ok "nothing was installed there"
    fi

    # Sticky does NOT rescue the destination. It stops another user removing
    # our files; it does not stop them creating srelens-tui there first and
    # owning it -- after which its mode is read and copied onto the rollback
    # (a planted 4755 becoming a root-owned setuid file), and it can be
    # swapped for a symlink to a directory so the mv lands underneath it.
    home="$work/homes/sticky"
    new_home "$home"
    chmod 1777 "$home/.local/bin"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a world-writable destination is refused even with sticky" "writable by other users" "$out" "$rc" 1
    if [ -e "$home/.local/bin/srelens-tui" ]; then
        no "it installed into the sticky world-writable directory anyway"
    else
        ok "nothing was installed there"
    fi

    # A sticky ANCESTOR is still fine, which is what keeps /tmp usable --
    # and every path in this suite runs through it.
    sticky_parent="$work/sticky-parent"
    rm -rf "$sticky_parent"
    mkdir -p "$sticky_parent"
    chmod 1777 "$sticky_parent"
    home="$sticky_parent/home"
    new_home "$home"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a sticky world-writable ANCESTOR is still fine" "Installed:" "$out" "$rc" 0

    # A directory belonging to somebody else: they can arrange the swap at
    # leisure and get a file written by the installing account out of it.
    if [ -n "$other_user" ]; then
        home="$work/homes/theirs"
        new_home "$home"
        # Theirs, but still writable by us: the point is that ownership is
        # refused, not that the directory happened to be unwritable.
        chown "$other_user" "$home/.local/bin" 2>/dev/null || true
        chgrp tester "$home/.local/bin" 2>/dev/null || true
        chmod 0770 "$home/.local/bin"
        out="$(install_into "$home")" && rc=0 || rc=$?
        check "a directory owned by another user is refused" "belongs to $other_user" "$out" "$rc" 1
        if [ -e "$home/.local/bin/srelens-tui" ]; then
            no "it installed into the other user's directory anyway"
        else
            ok "nothing was installed there either"
        fi

        # Sticky does not save a directory whose OWNER is somebody else: the
        # owner may remove anything inside it regardless.
        home="$work/homes/theirs-sticky"
        new_home "$home"
        chown "$other_user" "$home/.local/bin" 2>/dev/null || true
        chmod 1777 "$home/.local/bin"
        out="$(install_into "$home")" && rc=0 || rc=$?
        check "a sticky directory owned by someone else is refused" "safely" "$out" "$rc" 1

        # A directory can be impeccable itself and still sit under one somebody
        # else owns, who can rename it and put their own in its place after the
        # check. Every component is walked, so the ancestor is what fails.
        home="$work/homes/foreign-parent"
        new_home "$home"
        chown "$other_user" "$home/.local" 2>/dev/null || true
        out="$(install_into "$home")" && rc=0 || rc=$?
        check "a directory under a foreign ancestor is refused" "belongs to $other_user" "$out" "$rc" 1
        if [ -e "$home/.local/bin/srelens-tui" ]; then
            no "it installed under the replaceable ancestor anyway"
        else
            ok "nothing was installed under the replaceable ancestor"
        fi
    else
        echo "  skip  no second account: cannot test foreign ownership"
    fi

    # A symlink is not the directory it points at. `ls -ld` on one reports
    # `lrwxrwxrwx` owned by whoever made the link, so inspecting the path as
    # given would describe the link while the install lands somewhere else --
    # and ~/.local/bin being a link is an ordinary thing for it to be.
    target="$work/unsafe-target"
    rm -rf "$target"
    mkdir -p "$target"
    chmod 0777 "$target"
    home="$work/homes/link-unsafe"
    new_home "$home"
    rmdir "$home/.local/bin"
    ln -sfn "$target" "$home/.local/bin"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a symlink to an unsafe directory is refused" "writable by other users" "$out" "$rc" 1
    if [ -e "$target/srelens-tui" ]; then
        no "it installed through the symlink anyway"
    else
        ok "nothing was installed through the symlink"
    fi

    # A safe symlink must still install -- through the directory it points at,
    # named canonically. Approving the resolved path but staging and running
    # through the path as given would leave the link repointable afterwards.
    target="$work/real-bin"
    rm -rf "$target"
    mkdir -p "$target"
    chown tester "$target" 2>/dev/null || true
    chgrp tester "$target" 2>/dev/null || true
    home="$work/homes/link-safe"
    new_home "$home"
    rmdir "$home/.local/bin"
    ln -sfn "$target" "$home/.local/bin"
    chown -h tester "$home/.local/bin" 2>/dev/null || true
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a safe symlink installs into its target" "Installed: $target/srelens-tui" "$out" "$rc" 0
    if [ -x "$target/srelens-tui" ]; then
        ok "the binary landed in the resolved directory"
    else
        no "nothing landed in the resolved directory"
    fi

    # A FIFO where the binary goes. Reading one waits for a writer that never
    # comes, so this has to be refused rather than copied -- an installer
    # that hangs forever is worse than one that says no.
    if command -v mkfifo >/dev/null 2>&1; then
        home="$work/homes/fifo-dest"
        new_home "$home"
        mkfifo "$home/.local/bin/srelens-tui" 2>/dev/null || true
        if [ -p "$home/.local/bin/srelens-tui" ]; then
            out="$(install_into "$home")" && rc=0 || rc=$?
            check "a FIFO where the binary goes is refused" "not a regular file" "$out" "$rc" 1
        else
            echo "  skip  could not create a FIFO here"
        fi
    else
        echo "  skip  no mkfifo: cannot test a special file at the destination"
    fi

    # A symlink where the binary goes is refused: the rollback copy is taken
    # by reading $dest, which follows the link, so a restore would put a
    # regular file where a link had been.
    home="$work/homes/link-dest"
    new_home "$home"
    ln -sfn /bin/true "$home/.local/bin/srelens-tui"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a symlink where the binary goes is refused" "is a symlink" "$out" "$rc" 1
    if [ -L "$home/.local/bin/srelens-tui" ]; then
        ok "the symlink is left as it was"
    else
        no "the symlink was replaced"
    fi
    # And a second hard link to the binary. The backup is a copy into a
    # fresh inode, so a rollback could not give the other name back the
    # file it shares now -- refuse rather than promise a restore that
    # splits them.
    home="$work/homes/hardlinked"
    new_home "$home"
    printf '#!/bin/sh\necho OLD COPY\n' > "$home/.local/bin/srelens-tui"
    chmod 0755 "$home/.local/bin/srelens-tui"
    ln "$home/.local/bin/srelens-tui" "$home/.local/bin/srelens-tui.other"
    chown tester "$home/.local/bin/srelens-tui" 2>/dev/null || true
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a hard-linked binary is refused" "hard links" "$out" "$rc" 1
    if grep -q "OLD COPY" "$home/.local/bin/srelens-tui" 2>/dev/null &&
        [ "$(stat -c %h "$home/.local/bin/srelens-tui" 2>/dev/null)" = "2" ]; then
        ok "and both names still share the old inode"
    else
        no "the hard-linked binary was touched"
    fi

    # `mv file dir` moves the file INTO the directory. A destination that is
    # already a directory would swallow the staging file and leave nothing at
    # the path asked for -- and the version line used to hide that inside a
    # command substitution, so the install printed Installed and exited 0.
    home="$work/homes/dir-dest"
    new_home "$home"
    mkdir -p "$home/.local/bin/srelens-tui"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a directory where the binary goes is refused" "is a directory" "$out" "$rc" 1
    case "$out" in
        *Installed:*) no "it claimed to have installed something" ;;
        *) ok "it did not claim to have installed anything" ;;
    esac
else
    echo "  skip  no unprivileged account this run created: cannot shape a destination"
fi

# Writable is not the same as safe, and the difference decides where the
# install goes rather than whether it happens. A GitHub runner ships
# /usr/local/bin world-writable: the rules refuse THAT directory, and the
# install falls back to the home one rather than giving up.
#
# The only case here that touches a system directory, hence the container
# gate at the top of this file.
if [ "$made_user" = "tester" ] && can_shape_dest; then
    chmod 0777 /usr/local/bin
    home="$work/homes/fallback"
    new_home "$home"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "an unsafe /usr/local/bin falls back rather than refusing" "Installed: $home/.local/bin/srelens-tui" "$out" "$rc" 0
    reset_dest
else
    echo "  skip  not root in a container: cannot make /usr/local/bin unsafe"
fi

echo "shared groups and ancestors"

if [ "$made_user" = "tester" ]; then
    # Group-writable is refused whatever the group is. Who is really in a
    # group cannot be established from a shell -- an SSSD or LDAP source can
    # resolve accounts one at a time while declining to enumerate, so any
    # answer is a lower bound rather than a fact.
    home="$work/homes/own-group"
    new_home "$home"
    chmod 0775 "$home/.local/bin"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a group-writable destination is refused" "group-writable" "$out" "$rc" 1
    if [ -e "$home/.local/bin/srelens-tui" ]; then
        no "it installed into the group-writable directory anyway"
    else
        ok "nothing was installed there"
    fi
    check "and says how to fix it" "chmod g-w" "$out" "$rc" 1

    # The owner's own group is refused too: it is the case the old rule tried
    # to allow, on the strength of a convention that Fedora does not actually
    # follow -- UMASK is 022 there and a fresh ~/.local/bin is drwxr-xr-x.
    if command -v groupadd >/dev/null 2>&1; then
        if ! getent group shared >/dev/null 2>&1; then
            if groupadd shared >/dev/null 2>&1; then
                made_group="shared"
            fi
        fi
        home="$work/homes/shared-group"
        new_home "$home"
        if chgrp shared "$home/.local/bin" 2>/dev/null; then
            chmod 0775 "$home/.local/bin"
            out="$(install_into "$home")" && rc=0 || rc=$?
            check "a shared group is refused by the same rule" "group-writable" "$out" "$rc" 1
        else
            echo "  skip  could not set a shared group"
        fi
    else
        echo "  skip  no groupadd: cannot test a shared group"
    fi

    # Group-writable plus sticky is refused for the same reason: sticky
    # protects entries that exist, not the right to create one.
    home="$work/homes/group-sticky"
    new_home "$home"
    chmod 3775 "$home/.local/bin"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "group-writable plus sticky is refused too" "group-writable" "$out" "$rc" 1

    # An extended ACL can grant write to any account while the mode bits
    # look impeccable. getfacl answers this properly; GNU ls answers it with
    # a trailing + on the mode string; BusyBox ls does not answer it at all.
    home="$work/homes/acl"
    new_home "$home"
    if command -v setfacl >/dev/null 2>&1 && [ -n "$other_user" ] &&
        setfacl -m "u:$other_user:rwx" "$home/.local/bin" 2>/dev/null; then
        out="$(install_into "$home")" && rc=0 || rc=$?
        check "a directory with an extended ACL is refused" "extended ACL" "$out" "$rc" 1
    else
        echo "  skip  no setfacl, or the filesystem will not take an ACL"
    fi

    # A getfacl that FAILS is not a directory without an ACL. Empty output
    # from a failed command reads exactly like a file carrying only its base
    # entries.
    mkdir -p "$work/fake"
    cat > "$work/fake/getfacl" <<EOF
#!/bin/sh
exit 1
EOF
    chmod +x "$work/fake/getfacl"
    home="$work/homes/acl-broken"
    new_home "$home"
    out="$(install_into "$home" "PATH=$work/fake:$PATH")" && rc=0 || rc=$?
    check "a getfacl that fails is not read as no ACL" "cannot read the ACL" "$out" "$rc" 1
    rm -f "$work/fake/getfacl"

    # And where neither tool can answer -- BusyBox without the acl package --
    # the install refuses rather than proceeding blind. That case used to be
    # documented as a known gap and allowed, which meant an ACL sailed
    # through on precisely the systems that could not see it.
    #
    # Simulated with a PATH carrying no getfacl at all and an `ls` that
    # declines to identify itself, which is what BusyBox looks like from in
    # there. A restricted PATH rather than a prefix, or the real getfacl in
    # /usr/bin answers and the case proves nothing.
    blind="$work/blind"
    rm -rf "$blind"
    mkdir -p "$blind"
    blind_missing=""
    for tool in sh uname curl sed head cut grep tar gzip chmod mktemp dirname \
        mkdir cp mv rm ln cat awk id getent stat sha256sum; do
        tpath="$(command -v "$tool" 2>/dev/null)" || { blind_missing="$blind_missing $tool"; continue; }
        ln -sf "$tpath" "$blind/$tool"
    done
    real_ls="$(command -v ls)"
    cat > "$blind/ls" <<EOF
#!/bin/sh
# BusyBox ls: no --version to identify itself, and no ACL marker.
case "\$1" in
  --version) echo "ls: unrecognized option: version" >&2; exit 1 ;;
esac
exec $real_ls "\$@"
EOF
    chmod +x "$blind/ls"
    if [ -n "$blind_missing" ]; then
        echo "  skip  no ACL-blind run:$blind_missing not found"
    else
        home="$work/homes/acl-blind"
        new_home "$home"
        chmod -R a+rx "$blind"
        out="$(install_into "$home" "PATH=$blind")" && rc=0 || rc=$?
        check "an ls that cannot report ACLs refuses rather than guessing" "cannot tell whether" "$out" "$rc" 1
        if [ -e "$home/.local/bin/srelens-tui" ]; then
            no "it installed without being able to see the ACLs"
        else
            ok "nothing was installed while ACLs were unreadable"
        fi
    fi
else
    echo "  skip  no unprivileged account this run created: cannot shape a group"
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
check "an untrusted TMPDIR is refused" "writable by other users" "$out" "$rc" 1
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
        # leave the caller with nothing where a working copy stood -- nor
        # hand back a copy with permissions it never had.
        # 4700, not 0700: the rollback must put back the permissions and
        # drop the set-ID bit, never reproduce it on a file this script
        # did not write.
        chmod 4700 "$dest/srelens-tui" 2>/dev/null || true
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
        restored_mode="$(stat -c %a "$dest/srelens-tui" 2>/dev/null)" || restored_mode="?"
        if [ "$restored_mode" = "700" ]; then
            ok "and came back 700: permissions kept, set-ID bit dropped"
        else
            no "the restored binary is mode $restored_mode, wanted 700 from 4700"
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
check "a TMPDIR symlink is resolved before it is judged" "writable by other users" "$out" "$rc" 1


echo "wrong version"

# A release that published a stale binary under the right asset name and
# checksum: it runs, so `--version` succeeding proves nothing. What was
# asked for has to be what arrived.
if [ "$made_user" = "tester" ]; then
    wrong="$work/wrong"
    rm -rf "$wrong"
    mkdir -p "$wrong"
    printf '#!/bin/sh\necho "srelens-tui 9.9.9"\n' > "$wrong/srelens-tui"
    chmod 0755 "$wrong/srelens-tui"
    printf 'nothing\n' > "$wrong/LICENSE"
    wrongarchive="srelens-tui-$version-$host_target.tar.gz"
    (cd "$wrong" && tar -czf "$work/fixtures/$wrongarchive.wrong" .)
    wrongsum="$(sha256sum "$work/fixtures/$wrongarchive.wrong" | cut -d' ' -f1)"
    printf '%s  %s\n' "$wrongsum" "$wrongarchive" > "$work/fixtures/WRONGSUMS.txt"
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
  *SHA256SUMS*)     cp "$work/fixtures/WRONGSUMS.txt" "\$out" ;;
  *.tar.gz)         cp "$work/fixtures/$wrongarchive.wrong" "\$out" ;;
  *) exit 1 ;;
esac
EOF
    chmod +x "$work/fake/curl"
    home="$work/homes/wrong-version"
    new_home "$home"
    out="$(install_into "$home" "PATH=$work/fake:$PATH")" && rc=0 || rc=$?
    check "a binary reporting another version is refused" "not the $version that was asked for" "$out" "$rc" 1
    if [ -e "$home/.local/bin/srelens-tui" ]; then
        no "the wrong-version binary was left installed"
    else
        ok "and it is not left behind"
    fi

    # A version that CONTAINS the requested one. `1.2.30` contains `1.2.3`,
    # so a substring match accepts precisely the stale build this is for.
    # %s so the version really expands: inside single quotes it would not,
    # and the case would pass for the wrong reason.
    printf '#!/bin/sh\necho "srelens-tui %s0"\n' "$version" > "$wrong/srelens-tui"
    (cd "$wrong" && tar -czf "$work/fixtures/$wrongarchive.wrong" .)
    wrongsum="$(sha256sum "$work/fixtures/$wrongarchive.wrong" | cut -d' ' -f1)"
    printf '%s  %s\n' "$wrongsum" "$wrongarchive" > "$work/fixtures/WRONGSUMS.txt"
    home="$work/homes/version-prefix"
    new_home "$home"
    out="$(install_into "$home" "PATH=$work/fake:$PATH")" && rc=0 || rc=$?
    check "a version that merely contains the requested one is refused" "not the $version that was asked for" "$out" "$rc" 1

    # A rollback that cannot complete must keep the only copy of what was
    # there and say where it is, rather than delete it and report recovery.
    # A `mv` that refuses to move the backup is what a read-only mount or a
    # full disk looks like from here.
    cat > "$work/fake/mv" <<EOF
#!/bin/sh
for a in \$@; do
  case "\$a" in *.srelens-tui.backup.*) exit 1 ;; esac
done
exec /bin/mv "\$@"
EOF
    chmod +x "$work/fake/mv"
    home="$work/homes/rollback-fails"
    new_home "$home"
    printf '#!/bin/sh\necho OLD COPY\n' > "$home/.local/bin/srelens-tui"
    chmod 0755 "$home/.local/bin/srelens-tui"
    chown tester "$home/.local/bin/srelens-tui" 2>/dev/null || true
    out="$(install_into "$home" "PATH=$work/fake:$PATH")" && rc=0 || rc=$?
    check "a rollback that fails says so" "could NOT be put back" "$out" "$rc" 1
    check "and names where the copy still is" ".srelens-tui.backup." "$out" "$rc" 1
    if [ -n "$(find "$home/.local/bin" -name '.srelens-tui.backup.*' 2>/dev/null)" ]; then
        ok "the only copy of the previous binary is kept"
    else
        no "the previous binary was destroyed by a failed rollback"
    fi
    rm -f "$work/fake/mv"

    # The same honesty on the other branch. A FIRST install that is rejected
    # has no backup to restore, so the rollback removes what it put there --
    # and if that removal fails, saying it was removed would leave a rejected
    # binary on PATH under a name the caller now trusts.
    #
    # An `rm` that refuses to delete the installed path is what a read-only
    # mount or an immutable flag looks like from in there.
    cat > "$work/fake/rm" <<EOF
#!/bin/sh
for a in \$@; do
  case "\$a" in */srelens-tui) exit 1 ;; esac
done
exec /bin/rm "\$@"
EOF
    chmod +x "$work/fake/rm"
    home="$work/homes/removal-fails"
    new_home "$home"
    out="$(install_into "$home" "PATH=$work/fake:$PATH")" && rc=0 || rc=$?
    check "a rejected first install that cannot be removed says so" "could NOT be removed" "$out" "$rc" 1
    check "and names where it is still installed" "$home/.local/bin/srelens-tui" "$out" "$rc" 1
    rm -f "$work/fake/rm"

    # Metadata the rollback cannot carry. The copy takes bytes, mode, owner
    # and times; a file capability is not among them, and losing one silently
    # would take privileges from a binary that had them.
    if command -v setcap >/dev/null 2>&1; then
        home="$work/homes/capped"
        new_home "$home"
        cp /bin/true "$home/.local/bin/srelens-tui"
        chown tester "$home/.local/bin/srelens-tui" 2>/dev/null || true
        if setcap cap_net_raw+ep "$home/.local/bin/srelens-tui" 2>/dev/null; then
            out="$(install_into "$home")" && rc=0 || rc=$?
            check "a binary with a file capability is not silently replaced" "security.capability" "$out" "$rc" 1
        else
            echo "  skip  could not set a capability on this filesystem"
        fi
    else
        echo "  skip  no setcap: cannot test capability detection"
    fi

    # A first install has no previous copy to be faithful to, so it must not
    # be held to any of that -- nobody installing for the first time should be
    # sent to fetch tools.
    home="$work/homes/fresh-no-attr"
    new_home "$home"
    out="$(install_into "$home")" && rc=0 || rc=$?
    check "a first install is not asked about metadata" "Installed:" "$out" "$rc" 0

    # And where the question cannot be answered at all, an UPDATE refuses
    # rather than replacing a binary whose metadata it cannot account for.
    blind_attr="$work/blind-attr"
    rm -rf "$blind_attr"
    mkdir -p "$blind_attr"
    attr_missing=""
    for tool in sh uname curl sed head cut grep tar gzip chmod mktemp dirname \
        mkdir cp mv rm ln cat ls awk id getent getfacl stat sha256sum; do
        tpath="$(command -v "$tool" 2>/dev/null)" || { attr_missing="$attr_missing $tool"; continue; }
        ln -sf "$tpath" "$blind_attr/$tool"
    done
    if [ -n "$attr_missing" ]; then
        echo "  skip  no getfattr-blind run:$attr_missing not found"
    else
        chmod -R a+rx "$blind_attr"
        # install_into runs as an unprivileged account, and there the check is
        # deliberately not required: a file capability needs root to set, so
        # one on a user's own binary is unusual. Root is the case that refuses.
        # But root CAN set one on a user's file, and a rollback made with
        # `cp -p` would not bring it back -- so the update says, up front,
        # that the attributes were not checked and would not survive a
        # rollback.
        out="$(install_into "$home" "PATH=$blind_attr")" && rc=0 || rc=$?
        check "an unprivileged update proceeds without getfattr" "Installed:" "$out" "$rc" 0
        check "and says that extended attributes were not checked" "were not checked" "$out" "$rc" 0
        # And when such an update IS rolled back, the caveat rides on the
        # "put back" line: the previous copy is back, its attributes are not
        # claimed to be. Same blind PATH, with the wrong-version fake curl
        # from above in place of the real one.
        blind_bad="$work/blind-attr-bad"
        rm -rf "$blind_bad"
        mkdir -p "$blind_bad"
        for tpath in "$blind_attr"/*; do
            ln -sf "$tpath" "$blind_bad/${tpath##*/}"
        done
        ln -sf "$work/fake/curl" "$blind_bad/curl"
        chmod -R a+rx "$blind_bad"
        home="$work/homes/blind-attr-rollback"
        new_home "$home"
        printf '#!/bin/sh\necho OLD COPY\n' > "$home/.local/bin/srelens-tui"
        chmod 0755 "$home/.local/bin/srelens-tui"
        chown tester "$home/.local/bin/srelens-tui" 2>/dev/null || true
        out="$(install_into "$home" "PATH=$blind_bad")" && rc=0 || rc=$?
        check "a rolled-back update without getfattr does not claim the attributes came back" "put back -- without any extended attributes" "$out" "$rc" 1
        if grep -q "OLD COPY" "$home/.local/bin/srelens-tui" 2>/dev/null; then
            ok "and the previous copy itself is back"
        else
            no "the previous copy did not come back"
        fi
        if [ "$(id -u)" = "0" ]; then
            root_home="$work/homes/root-no-attr"
            rm -rf "$root_home"
            mkdir -p "$root_home/.local/bin"
            printf '#!/bin/sh
echo OLD
' > "$root_home/.local/bin/srelens-tui"
            chmod 0755 "$root_home/.local/bin/srelens-tui"
            out="$(PATH="$blind_attr" HOME="$root_home" sh "$script" --version "$version" 2>&1)" && rc=0 || rc=$?
            check "a ROOT update refuses when it cannot check for xattrs" "getfattr is not installed" "$out" "$rc" 1
        else
            echo "  skip  not root: cannot test the privileged xattr requirement"
        fi
    fi
    rm -f "$work/fake/curl"
else
    echo "  skip  no unprivileged account this run created: cannot test a wrong version"
fi

echo "interrupted mid-update"

# A signal arriving after the destination has been replaced but before the
# new binary has been run leaves an unvalidated copy live and the old one
# hidden under a random name. The traps have to undo that.
#
# Timed by watching for the line printed just before the download starts,
# then killing it: the replacement happens after that, so this covers the
# window rather than one instant inside it. Either outcome is acceptable --
# the old binary back, or the update having finished first -- but not a
# destination left empty or littered.
if [ "$made_user" = "tester" ]; then
    home="$work/homes/interrupted"
    new_home "$home"
    printf '#!/bin/sh\necho OLD COPY\n' > "$home/.local/bin/srelens-tui"
    chmod 0755 "$home/.local/bin/srelens-tui"
    chown tester "$home/.local/bin/srelens-tui" 2>/dev/null || true
    chmod 0711 "$work" 2>/dev/null || true
    chmod 0644 "$script" 2>/dev/null || true
    su tester -c "HOME='$home' sh '$script' --version '$version'" >"$work/int.log" 2>&1 &
    kill_pid=$!
    tries=0
    while [ "$tries" -lt 300 ]; do
        grep -q "Installing srelens-tui" "$work/int.log" 2>/dev/null && break
        tries=$((tries + 1))
        sleep 0.05
    done
    kill -TERM "$kill_pid" 2>/dev/null || true
    wait "$kill_pid" 2>/dev/null || true
    if grep -q "OLD COPY" "$home/.local/bin/srelens-tui" 2>/dev/null; then
        ok "an interrupted update leaves the previous binary in place"
    elif [ -x "$home/.local/bin/srelens-tui" ]; then
        ok "the update completed before the signal landed, nothing to undo"
    else
        no "an interrupted update left no binary at all"
    fi
    if [ -z "$(find "$home/.local/bin" -name '.srelens-tui.*' 2>/dev/null)" ]; then
        ok "and nothing hidden behind it"
    else
        no "an interrupted update left hidden files in the install directory"
    fi
    # HUP as well: an SSH session dropping mid-install. An untrapped HUP
    # ends dash without running the EXIT trap, so this one has to be caught
    # in its own right. Sent straight to the installer's pid, recorded by
    # the shell that execs it, rather than to `su` -- which relays TERM but
    # is not relied on to relay HUP.
    home="$work/homes/interrupted-hup"
    new_home "$home"
    printf '#!/bin/sh\necho OLD COPY\n' > "$home/.local/bin/srelens-tui"
    chmod 0755 "$home/.local/bin/srelens-tui"
    chown tester "$home/.local/bin/srelens-tui" 2>/dev/null || true
    pidfile="$work/hup.pid"
    : > "$pidfile"
    chown tester "$pidfile" 2>/dev/null || true
    su tester -c "echo \$\$ > '$pidfile'; HOME='$home' exec sh '$script' --version '$version'" >"$work/hup.log" 2>&1 &
    kill_pid=$!
    tries=0
    while [ "$tries" -lt 300 ]; do
        grep -q "Installing srelens-tui" "$work/hup.log" 2>/dev/null && break
        tries=$((tries + 1))
        sleep 0.05
    done
    hup_pid="$(cat "$pidfile" 2>/dev/null)"
    kill -HUP "${hup_pid:-$kill_pid}" 2>/dev/null || true
    wait "$kill_pid" 2>/dev/null || true
    if grep -q "OLD COPY" "$home/.local/bin/srelens-tui" 2>/dev/null; then
        ok "a hung-up update leaves the previous binary in place"
    elif [ -x "$home/.local/bin/srelens-tui" ]; then
        ok "the update completed before the hangup landed, nothing to undo"
    else
        no "a hung-up update left no binary at all"
    fi
    if [ -z "$(find "$home/.local/bin" -name '.srelens-tui.*' 2>/dev/null)" ]; then
        ok "and nothing hidden behind it"
    else
        no "a hung-up update left hidden files in the install directory"
    fi

    # And the same interruption with NOTHING there beforehand. There is no
    # copy to put back, so the rollback has to remove what it installed --
    # otherwise an unvalidated binary is left on a PATH under a name that
    # will be trusted.
    home="$work/homes/interrupted-fresh"
    new_home "$home"
    su tester -c "HOME='$home' sh '$script' --version '$version'" >"$work/int2.log" 2>&1 &
    kill_pid=$!
    tries=0
    while [ "$tries" -lt 300 ]; do
        grep -q "Installing srelens-tui" "$work/int2.log" 2>/dev/null && break
        tries=$((tries + 1))
        sleep 0.05
    done
    kill -TERM "$kill_pid" 2>/dev/null || true
    wait "$kill_pid" 2>/dev/null || true
    if [ -e "$home/.local/bin/srelens-tui" ]; then
        if "$home/.local/bin/srelens-tui" --version >/dev/null 2>&1; then
            ok "the first install completed before the signal landed"
        else
            no "an interrupted first install left an unvalidated binary behind"
        fi
    else
        ok "an interrupted first install leaves nothing behind"
    fi

else
    echo "  skip  no unprivileged account this run created: cannot interrupt an update"
fi

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
    # getfacl among them: without it, and with a BusyBox ls, the installer
    # refuses before it ever reaches the hashing this case is about.
    for tool in sh uname curl sed head cut grep tar gzip chmod mktemp dirname mkdir cp mv rm ln cat ls awk id getent getfacl getfattr stat shasum; do
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
