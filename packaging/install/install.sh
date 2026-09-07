#!/bin/sh
# Install srelens-tui on Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh | sh
#
# Everything is inside main(), called on the very last line. A script read
# from a pipe is executed as it arrives, so a connection that dies halfway
# through would otherwise run whatever fragment made it — with the function
# wrapper, a truncated download is a syntax error that does nothing.
#
# POSIX sh, not bash: this has to work under dash on Debian and under
# BusyBox ash on Alpine, both of which are `/bin/sh` on machines people
# actually run.
set -eu

REPO="srelens/srelens"
BIN="srelens-tui"

main() {
    version=""
    install_dir=""

    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                version="${2:-}"
                [ -n "$version" ] || die "--version needs a value, e.g. --version 0.9.0"
                shift 2
                ;;
            --version=*)
                version="${1#--version=}"
                # An unset variable expanded into --version="$V" arrives
                # here as an empty value. Treating that as "no --version"
                # would silently install the latest release instead of the
                # pin the caller asked for.
                [ -n "$version" ] || die "--version needs a value, e.g. --version=0.9.0"
                shift
                ;;
            --install-dir)
                install_dir="${2:-}"
                [ -n "$install_dir" ] || die "--install-dir needs a value"
                shift 2
                ;;
            --install-dir=*)
                install_dir="${1#--install-dir=}"
                [ -n "$install_dir" ] || die "--install-dir needs a value"
                shift
                ;;
            -h | --help)
                usage
                return 0
                ;;
            *)
                die "unknown option: $1 (try --help)"
                ;;
        esac
    done

    check_platform
    need curl
    need tar
    require_sha_tool

    target="$(detect_target)"
    [ -n "$version" ] || version="$(latest_version)"
    # Tolerate a tag or a leading v: people paste both.
    version="${version#srelens-v}"
    version="${version#v}"

    install_dir="$(resolve_install_dir "$install_dir")"
    # Sets INSTALL_DIR to the canonical path, which is what everything below
    # uses. Before the download, so an unwritable destination costs nothing.
    prepare_install_dir "$install_dir"
    install_dir="$INSTALL_DIR"
    assert_safe_dir "$install_dir"

    say "Installing $BIN $version ($target) into $install_dir"

    tmp="$(mktemp -d)"
    # Covers the error paths too, since `set -e` exits through the trap.
    trap 'rm -rf "$tmp"' EXIT INT TERM

    archive="$BIN-$version-$target.tar.gz"
    base="https://github.com/$REPO/releases/download/srelens-v$version"

    download "$base/$archive" "$tmp/$archive"
    download "$base/$BIN-$version-SHA256SUMS.txt" "$tmp/SHA256SUMS.txt"
    verify_checksum "$tmp" "$archive"

    # Into a subdirectory, never into $tmp itself. The archive contains a
    # `./` member, and GNU tar restores directory ownership and permissions
    # from the archive when it runs as root -- which under `sudo` would
    # rewrite the 0700 root-owned directory mktemp -d just made into
    # whatever the release runner had, typically 0755 and a numeric uid.
    # An account matching that uid could then swap the binary between the
    # checksum below and the moment it is run. Extracting one level down
    # leaves $tmp itself untouched at 0700, so nothing can be reached
    # through it whatever the archive claims about its own directory.
    mkdir "$tmp/unpack" || die "cannot prepare a private directory to unpack into"
    tar -xzf "$tmp/$archive" -C "$tmp/unpack"
    [ -f "$tmp/unpack/$BIN" ] || die "the archive did not contain $BIN"
    chmod 0755 "$tmp/unpack/$BIN"

    # Run it before it is installed, not after. A binary for the wrong
    # architecture or a corrupt one that still hashed correctly fails here,
    # while the only thing that has happened is a write to a temp dir.
    "$tmp/unpack/$BIN" --version >/dev/null 2>&1 ||
        die "the downloaded binary does not run on this machine"

    install_binary "$tmp/unpack/$BIN" "$install_dir/$BIN"

    say ""
    say "Installed: $install_dir/$BIN"
    say "  $("$install_dir/$BIN" --version)"
    warn_if_not_on_path "$install_dir"
    say ""
    say "Next: $BIN            # browse the cluster in your current context"
    say "      $BIN toolbox    # what it found on your PATH (kubectl, helm)"
    say "      $BIN update     # move to a newer release later"
}

usage() {
    cat <<EOF
Install $BIN, the srelens terminal UI, on Linux.

Usage:
  install.sh [--version <x.y.z>] [--install-dir <path>]

Options:
  --version <x.y.z>     Install this version instead of the latest stable.
  --install-dir <path>  Install here instead of $(printf '%s' '/usr/local/bin, or ~/.local/bin when that is not writable').
  -h, --help            Show this message.

The binary is the statically linked musl build, so it does not care which
libc or which distribution is on the machine. Its SHA-256 is checked against
the release's published SHA256SUMS before anything is installed.

On macOS use Homebrew instead:  brew install srelens/tap/$BIN
EOF
}

say() { printf '%s\n' "$*"; }

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

# Refusing to install without a hashing tool is deliberate: installing an
# unverified binary quietly would defeat the point of publishing checksums.
require_sha_tool() {
    if command -v sha256sum >/dev/null 2>&1; then return 0; fi
    if command -v shasum >/dev/null 2>&1; then return 0; fi
    die "neither sha256sum nor shasum found; cannot verify the download"
}

# The SHA-256 of one file.
#
# The algorithm travels WITH the command, never as a bare tool name: plain
# `shasum` is SHA-1, so selecting it by name and calling it without -a 256
# yields a 40-character digest that can never match a 64-character one. Every
# archive would be rejected as corrupt, on exactly the machines that have
# shasum and no sha256sum -- which is why neither Debian nor Alpine, where
# this was first tested, could show it.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d" " -f1
    else
        shasum -a 256 "$1" | cut -d" " -f1
    fi
}

check_platform() {
    os="$(uname -s)"
    case "$os" in
        Linux) ;;
        Darwin)
            die "this script is for Linux. On macOS: brew install srelens/tap/$BIN"
            ;;
        *)
            die "unsupported operating system: $os"
            ;;
    esac
}

# Always the static musl build, never the glibc one.
#
# The glibc archives are built on ubuntu-22.04 and ubuntu-24.04-arm, so they
# carry a floor of glibc 2.35 and 2.39 — newer than Debian 11, RHEL 9 or any
# LTS a fair number of clusters are administered from. A dynamically linked
# binary fails there before main() with a GLIBC_2.3x symbol error that tells
# the reader nothing. The static build has no floor at all, and musl's
# slower allocator and narrower resolver do not matter to a client that
# spends its life waiting on an API server.
detect_target() {
    arch="$(uname -m)"
    case "$arch" in
        x86_64 | amd64) printf 'x86_64-unknown-linux-musl' ;;
        aarch64 | arm64) printf 'aarch64-unknown-linux-musl' ;;
        *) die "unsupported architecture: $arch (x86_64 and aarch64 are published)" ;;
    esac
}

latest_version() {
    # The tags are `srelens-v1.2.3`, so the version is what follows the v.
    tag="$(
        curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
            sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
            head -n 1
    )" || die "could not reach the GitHub API to find the latest release"
    [ -n "$tag" ] || die "could not determine the latest release"
    printf '%s' "${tag#srelens-v}"
}

# /usr/local/bin when it is writable, ~/.local/bin otherwise.
#
# No sudo. A script fetched over the network re-invoking itself as root is
# exactly the pattern people are right to be nervous about, and the fallback
# needs no privileges at all. Anyone who wants it system-wide can say so:
#   curl ... | sudo sh
resolve_install_dir() {
    if [ -n "$1" ]; then
        printf '%s' "$1"
        return
    fi
    if [ -w /usr/local/bin ] 2>/dev/null; then
        printf '/usr/local/bin'
    else
        printf '%s/.local/bin' "$HOME"
    fi
}

download() {
    curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1" ||
        die "download failed: $1"
}

verify_checksum() {
    dir="$1"
    file="$2"

    expected="$(
        grep "  $file\$" "$dir/SHA256SUMS.txt" 2>/dev/null |
            head -n 1 | cut -d' ' -f1
    )" || true
    [ -n "$expected" ] ||
        die "$file is not listed in the release's SHA256SUMS"

    actual="$(cd "$dir" && sha256_of "$file")"

    if [ "$expected" != "$actual" ]; then
        printf 'error: checksum mismatch for %s\n' "$file" >&2
        printf '  expected %s\n' "$expected" >&2
        printf '  actual   %s\n' "$actual" >&2
        die "refusing to install"
    fi
    say "Checksum verified: $actual"
}

# Create the destination, confirm it can be written to, and settle on the
# canonical path for it.
#
# Canonical matters as much as the checks that follow. Inspecting a resolved
# path but then staging, renaming and finally RUNNING through the path as
# given leaves the whole window open: a symlink the caller passed can be
# repointed the moment after it is approved, and the install lands wherever
# it now says. Everything downstream uses what this sets.
prepare_install_dir() {
    dir="$1"
    mkdir -p "$dir" 2>/dev/null ||
        die "cannot create $dir"
    [ -w "$dir" ] ||
        die "$dir is not writable. Re-run with --install-dir <somewhere you own>, or with sudo."
    resolved="$(cd "$dir" 2>/dev/null && pwd -P)" || resolved=""
    [ -n "$resolved" ] && dir="$resolved"
    INSTALL_DIR="$dir"
}

# Refuse a destination that another user could tamper with mid-install.
#
# EVERY component is checked, not just the leaf. A directory can pass on its
# own permissions and still sit under one somebody else owns -- and that
# owner can rename it and put their own directory at the same path, after the
# check and before the install. The staging file, the rename and the version
# line would then all run inside theirs. This is the walk `sudo` and `ssh` do
# over their own paths, for the same reason.
#
# The rules per component:
#
#   * owned by root or by us. Anyone else can replace what is inside it.
#   * if world-writable, the sticky bit must be set, so only an entry's owner
#     may unlink it. That is what makes /tmp usable rather than disqualifying.
#   * if group-writable, the group must be the owner's own -- the per-user
#     group convention, where `alice:alice` has one member. A shared group is
#     a set of people who can all replace the binary.
#
# The same rule srelens-tui's own `update` applies to the binary it replaces.
assert_safe_dir() {
    dir="$1"

    # Resolve first. `ls -ld` on a symlink describes the LINK -- mode
    # `lrwxrwxrwx`, owned by whoever made it -- which says nothing about where
    # the install lands. `cd` + `pwd -P` is portable; `readlink -f` is GNU.
    resolved="$(cd "$dir" 2>/dev/null && pwd -P)" || resolved=""
    [ -n "$resolved" ] || return 0

    SAFE_ME="$(id -un 2>/dev/null)" || SAFE_ME=""

    assert_component "/"
    rest="${resolved#/}"
    prefix=""
    while [ -n "$rest" ]; do
        name="${rest%%/*}"
        case "$rest" in
            */*) rest="${rest#*/}" ;;
            *) rest="" ;;
        esac
        prefix="$prefix/$name"
        assert_component "$prefix"
    done
}

# One component of the path, against the rules above.
assert_component() {
    path="$1"

    # `ls -ld` rather than stat: stat's flags differ between GNU and BSD, and
    # this has to run under BusyBox too.
    listing="$(ls -ld "$path" 2>/dev/null)" || return 0
    [ -n "$listing" ] || return 0
    perms="$(printf %s "$listing" | cut -c1-10)"
    owner="$(printf %s "$listing" | awk '{print $3}')"
    group="$(printf %s "$listing" | awk '{print $4}')"

    # Anything that is not a directory mode is not something to guess from.
    case "$perms" in
        d?????????) ;;
        *) return 0 ;;
    esac

    if [ -n "$owner" ] && [ "$owner" != "root" ] && [ "$owner" != "$SAFE_ME" ]; then
        die "$path belongs to $owner, who could replace what is inside it while this installs. Choose a path you control: --install-dir \$HOME/.local/bin"
    fi

    if [ "$(printf %s "$perms" | cut -c9)" = "w" ]; then
        case "$(printf %s "$perms" | cut -c10)" in
            t | T) ;;
            *)
                die "$path is writable by anyone and has no sticky bit, so another user could replace the binary between staging and running it. Choose a path you control: --install-dir \$HOME/.local/bin"
                ;;
        esac
    fi

    # Group-writable is only safe when the group is the owner's own, which is
    # the per-user-group convention (`alice:alice`, one member). Distributions
    # using it leave ~/.local/bin group-writable under a 002 umask, so
    # refusing that outright would break an ordinary Fedora install; a group
    # with a different name is a set of people who can all replace the binary.
    if [ "$(printf %s "$perms" | cut -c6)" = "w" ] &&
        [ -n "$group" ] && [ "$group" != "$owner" ]; then
        die "$path is writable by the group $group, whose members could replace the binary between staging and running it. Choose a path you control: --install-dir \$HOME/.local/bin"
    fi
}
# Install by rename where possible: a running binary being overwritten in
# place gets ETXTBSY on Linux, while replacing the directory entry does not
# disturb a process already holding the old inode.
install_binary() {
    src="$1"
    dest="$2"
    # Already created, checked and canonicalised by prepare_install_dir and
    # assert_safe_dir. Deliberately not re-derived from $dest here.
    dir="$INSTALL_DIR"

    # mktemp, not a name built from the pid. Installing as root into a directory
    # someone else can write to, the old `.srelens-tui.install.<pid>` was
    # predictable enough to pre-create as a symlink -- and `cp` follows a
    # destination symlink, so the copy would have written through it as root,
    # to a file of the attacker's choosing. mktemp creates the file itself,
    # exclusively and 0600, under a name nobody can aim at.
    staged="$(mktemp "$dir/.$BIN.install.XXXXXX")" ||
        die "cannot create a staging file in $dir"
    cp "$src" "$staged" || {
        rm -f "$staged"
        die "cannot write to $dir"
    }
    chmod 0755 "$staged"
    mv -f "$staged" "$dest" || {
        rm -f "$staged"
        die "cannot replace $dest"
    }
}

warn_if_not_on_path() {
    case ":${PATH}:" in
        *":$1:"*) return 0 ;;
    esac
    say ""
    say "Note: $1 is not on your PATH. Add it:"
    say "      export PATH=\"$1:\$PATH\""
}

main "$@"
