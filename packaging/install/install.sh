#!/bin/sh
# Install srelens-tui on Linux.
#
#   ( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
#     curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
#     sh "$f" )
#
# Download-then-run rather than piping into sh: a pipeline reports the
# status of its LAST command, so a download that 404s hands sh an empty
# script, which does nothing and exits 0 -- the line succeeds having
# installed nothing. mktemp rather than a fixed name: in a directory another
# account can write, a fixed name can be pre-created as a symlink for
# `curl -o` to truncate through. And a trap rather than a trailing `; rm`,
# which would make the cleanup's status the line's status.
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
            --install-dir | --install-dir=*)
                # Removed deliberately -- see the note above resolve_install_dir.
                die "--install-dir is no longer accepted. This installs to /usr/local/bin, or ~/.local/bin when that is not writable. To put the binary anywhere else, unpack the tarball yourself: see docs/INSTALL.md."
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

    install_dir="$(resolve_install_dir)"
    # Sets INSTALL_DIR to the canonical path, which is what everything below
    # uses. Before the download, so an unwritable destination costs nothing.
    prepare_install_dir "$install_dir"
    install_dir="$INSTALL_DIR"
    # Named before it is judged: a refusal should say which directory it is
    # about, and the answer is not always the obvious one now that an
    # unsafe /usr/local/bin falls back to the home directory.
    say "Installing $BIN $version ($target) into $install_dir"
    assert_safe_dir "$install_dir"

    # What a signal would have to undo. Set before the traps below, since
    # `set -u` makes an unset name an error inside a handler.
    INSTALL_DEST=""
    INSTALL_BACKUP=""
    INSTALL_STAGED=""
    INSTALL_COMMITTED=""
    INSTALL_ROLLBACK_KEPT=""
    INSTALL_ROLLBACK_STUCK=""
    INSTALL_ROLLBACK_RESTORED=""
    INSTALL_XATTR_UNCHECKED=""
    INSTALL_SELINUX_FROM=""

    tmp="$(mktemp -d)" || die "cannot create a private working directory"
    # Covers the error paths too, since `set -e` exits through the trap.
    #
    # INT and TERM exit explicitly. A handler that only cleans up returns to
    # where it interrupted, so the script would carry on against a working
    # directory it had just deleted -- and could reach the end and report
    # `Installed` after being asked to stop. The EXIT trap then runs a second
    # time, which `rm -rf` does not mind.
    # The destination first: a signal arriving after the replacement but
    # before the new binary has been run leaves an unvalidated copy live and
    # the old one hidden under a random name -- and on a noexec working
    # directory that window covers the only time the binary is ever checked.
    #
    # And say so when the rollback itself could not finish. A signal handler
    # that rolls back and exits in silence leaves a rejected binary live, or
    # the previous one hidden under a random name, with nobody told.
    #
    # HUP as well as INT and TERM: an SSH session that drops mid-install
    # sends HUP, and an untrapped HUP ends the shell WITHOUT running its
    # EXIT trap (dash, checked) -- the one window the transaction exists to
    # cover would be left open with nobody there to see it.
    trap 'install_rollback; report_rollback; rm -rf "$tmp"' EXIT
    trap 'install_rollback; report_rollback; rm -rf "$tmp"; exit 129' HUP
    trap 'install_rollback; report_rollback; rm -rf "$tmp"; exit 130' INT
    trap 'install_rollback; report_rollback; rm -rf "$tmp"; exit 143' TERM

    # The same walk the destination gets. `mktemp -d` makes the directory
    # itself 0700 and ours, but it puts it under $TMPDIR when that is set --
    # and `sudo` can carry the invoking user's TMPDIR straight into a root
    # install. A parent someone else owns can rename the tree after the
    # checksum passes and put their own binary where the verified one was,
    # to be run at the version check below.
    #
    # Canonicalised BEFORE the walk and kept, not resolved for inspection
    # and then discarded: every download, extraction and copy below reopens
    # $tmp by name, so approving one path and working through another would
    # leave a symlink in TMPDIR repointable the moment after it passed.
    tmp="$(cd "$tmp" 2>/dev/null && pwd -P)" ||
        die "cannot resolve the working directory"
    assert_safe_dir "$tmp"

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
    #
    # Unless this filesystem forbids running anything at all. A hardened
    # host mounts /tmp `noexec`, and mktemp puts the working tree there, so
    # a check that assumed otherwise would report every good binary as
    # broken. Probed with a script of our own rather than assumed either
    # way; when it cannot be done here, the same check runs after the
    # install instead, from the destination.
    printf '#!/bin/sh\nexit 0\n' > "$tmp/exec-probe"
    chmod 0755 "$tmp/exec-probe"
    if "$tmp/exec-probe" 2>/dev/null; then
        "$tmp/unpack/$BIN" --version >/dev/null 2>&1 ||
            die "the downloaded binary does not run on this machine"
    else
        say "Note: $tmp is mounted noexec, so the binary is checked after it is installed."
    fi

    install_binary "$tmp/unpack/$BIN" "$install_dir/$BIN"

    say ""
    # Not inside `say`. A failure in a command substitution there is
    # swallowed: the install printed `Installed` and exited 0 while the
    # binary was never in place.
    # Two questions, and a different answer for each, because a binary that
    # will not run and a binary that is the wrong version are different
    # problems for whoever is reading the output.
    installed_version="$("$install_dir/$BIN" --version 2>/dev/null)" || installed_version=""
    if [ -z "$installed_version" ]; then
        problem="does not run on this machine"
    else
        problem=""
        # Running is not enough: it has to be the version that was asked for.
        # A release that published a stale binary under the right asset name
        # and checksum would otherwise install silently, and a pinned
        # `--version` would report success having produced a different one.
        #
        # The whole token, not a substring. `1.2.30` contains `1.2.3`, so a
        # match on containment accepts precisely the stale build this is
        # meant to catch. The output is `srelens-tui <version>`; the second
        # field is compared, so the program renaming itself would not quietly
        # turn this check off either.
        reported="$(printf %s "$installed_version" | awk '{print $2}')"
        if [ "$reported" != "$version" ]; then
            problem="reports \"$installed_version\", not the $version that was asked for"
        fi
    fi

    if [ -z "$problem" ]; then
        # That is the commit: the copy that was there before is no longer
        # needed, and the traps stop trying to undo anything.
        INSTALL_COMMITTED=yes
        INSTALL_STAGED=""
        [ -z "$INSTALL_BACKUP" ] || rm -f "$INSTALL_BACKUP"
    else
        # One place decides what undoing an uncommitted install means, and it
        # is install_rollback -- doing it again here is how the restored copy
        # got deleted by the trap afterwards. This asks for it, then says what
        # happened.
        had_backup=""
        [ -z "$INSTALL_BACKUP" ] || had_backup=yes
        install_rollback
        kept="$INSTALL_ROLLBACK_KEPT"
        stuck="$INSTALL_ROLLBACK_STUCK"
        # Reported below with the reason; the EXIT trap must not repeat it.
        INSTALL_ROLLBACK_KEPT=""
        INSTALL_ROLLBACK_STUCK=""
        INSTALL_ROLLBACK_RESTORED=""
        # Undone. Nothing left for the EXIT trap to undo a second time.
        INSTALL_DEST=""
        INSTALL_BACKUP=""
        INSTALL_STAGED=""
        if [ -n "$stuck" ]; then
            die "the installed $BIN $problem, and it could NOT be removed. It is still installed at $stuck -- delete it before running $BIN from there"
        fi
        if [ -n "$kept" ]; then
            die "the installed $BIN $problem, and the copy that was there before could NOT be put back. It is still on disk: $kept"
        fi
        if [ -n "$had_backup" ]; then
            if [ -n "$INSTALL_XATTR_UNCHECKED" ]; then
                die "the installed $BIN $problem; the copy that was there before has been put back -- without any extended attributes it had, which were not checked (getfattr is not installed)"
            fi
            die "the installed $BIN $problem; the copy that was there before has been put back"
        fi
        die "the installed $BIN $problem; removed it again"
    fi
    say "Installed: $install_dir/$BIN"
    say "  $installed_version"
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
  install.sh [--version <x.y.z>]

Options:
  --version <x.y.z>  Install this version instead of the latest stable.
  -h, --help         Show this message.

It installs to /usr/local/bin, or ~/.local/bin when that is not writable.
There is no way to name a different directory: unpack the tarball yourself
if you want the binary somewhere else. See docs/INSTALL.md.

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

# /usr/local/bin when it is writable, ~/.local/bin otherwise. Those two, and
# nothing else.
#
# There was an --install-dir flag. It is gone on purpose: an arbitrary
# caller-chosen destination is most of this script's attack surface, and
# making one safe from a POSIX shell means winning filesystem races that a
# shell has no primitives for -- it cannot hold a descriptor across a check
# and a use, so every rule below is a claim about a path that could change
# underneath it. The two destinations here are root's or yours by
# construction. Anyone who wants the binary elsewhere can unpack the tarball
# themselves, which is a plain `tar -xzf` and is documented.
#
# The checks below still run, because both paths remain reachable from the
# environment: $HOME is whatever the caller says it is.
#
# No sudo. A script fetched over the network re-invoking itself as root is
# exactly the pattern people are right to be nervous about, and the fallback
# needs no privileges at all. Anyone who wants it system-wide can say so:
#   curl ... | sudo sh
resolve_install_dir() {
    # Writable is not the same as safe. A GitHub runner ships
    # /usr/local/bin world-writable, and plenty of images do something
    # similar -- there the rules below would refuse it, and refusing is the
    # right answer for THAT directory but the wrong answer for the install:
    # ~/.local/bin is sitting right there, belongs to the caller, and is
    # already the fallback for the unwritable case.
    #
    # The check runs in a subshell so its `die` ends only that, leaving this
    # a question rather than a verdict. Whichever directory is chosen is
    # then checked again for real by the caller, so an unsafe ~/.local/bin
    # still stops the install rather than being installed into quietly.
    if [ -w /usr/local/bin ] 2>/dev/null &&
        (assert_safe_dir /usr/local/bin) >/dev/null 2>&1; then
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
    want="$1"
    # What already EXISTS of the path is judged before anything is added to
    # it. `mkdir -p` first and the walk afterwards meant a refused fallback
    # still left its mark: `sudo` carrying a non-root HOME into a run whose
    # /usr/local/bin had been rejected would create root-owned .local/bin
    # directories under that home and only then refuse the foreign-owned
    # ancestor -- a persistent change from an install that reports having
    # installed nothing. So: the nearest existing ancestor and everything
    # above it first; the caller walks the full path, new components
    # included, once it exists. assert_safe_dir reuses `dir`, hence `want`.
    #
    # Absolute, or nothing. The fallback is built from HOME, which the
    # caller sets, and a relative HOME would make this a walk of names
    # relative to wherever the script happens to be run from -- and a walk
    # with nowhere to stop: `${x%/*}` of a name with no slash in it is the
    # name itself.
    case "$want" in
        /*) ;;
        *) die "$want is not an absolute path (HOME=${HOME:-unset}), so there is nowhere definite to install to. Set HOME to an absolute directory and run this again." ;;
    esac
    existing="$want"
    while [ ! -d "$existing" ] && [ "$existing" != "/" ]; do
        parent="${existing%/*}"
        [ -n "$parent" ] || parent="/"
        # Belt to the case above's braces: a step that removes nothing
        # would loop forever, so it stops the install instead.
        [ "$parent" != "$existing" ] ||
            die "cannot find an existing ancestor of $want"
        existing="$parent"
    done
    # As a PARENT, which is what it is: the destination will be created
    # beneath it. Judged by the destination's stricter rule, a sticky /tmp
    # under a HOME that does not exist yet would be refused -- the very
    # thing the sticky exemption exists to allow. The destination itself
    # gets its own rule from the caller's walk once it exists.
    assert_safe_dir "$existing" parent
    dir="$want"
    mkdir -p "$dir" 2>/dev/null ||
        die "cannot create $dir"
    [ -w "$dir" ] ||
        die "$dir is not writable. Re-run under sudo to install to /usr/local/bin."
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
#   * not group-writable at all, unless sticky. Who is really in a group
#     cannot be established from a shell -- see the note at that check.
#
# The same rule srelens-tui's own `update` applies to the binary it replaces.
#
# The last component is the destination unless the caller says `parent`:
# prepare_install_dir walks the nearest EXISTING ancestor before creating
# anything beneath it, and that ancestor is a parent, sticky /tmp included.
assert_safe_dir() {
    dir="$1"
    last_role="${2:-destination}"

    # Resolve first. `ls -ld` on a symlink describes the LINK -- mode
    # `lrwxrwxrwx`, owned by whoever made it -- which says nothing about where
    # the install lands. `cd` + `pwd -P` is portable; `readlink -f` is GNU.
    resolved="$(cd "$dir" 2>/dev/null && pwd -P)" || resolved=""
    [ -n "$resolved" ] ||
        die "cannot resolve $dir, so nothing can be said about where the install would land"

    SAFE_ME="$(id -un 2>/dev/null)" || SAFE_ME=""
    [ -n "$SAFE_ME" ] || die "cannot determine who is running this"

    assert_component "/" parent
    rest="${resolved#/}"
    prefix=""
    while [ -n "$rest" ]; do
        name="${rest%%/*}"
        case "$rest" in
            */*) rest="${rest#*/}" ;;
            *) rest="" ;;
        esac
        prefix="$prefix/$name"
        # The last component is where the binary lands, and it is held to a
        # stricter rule than the ones above it -- unless the caller said it
        # is a parent too.
        if [ -z "$rest" ]; then
            assert_component "$prefix" "$last_role"
        else
            assert_component "$prefix" parent
        fi
    done
}

# One component of the path, against the rules above.
assert_component() {
    path="$1"
    role="$2"

    # `ls -ld` rather than stat: stat's flags differ between GNU and BSD, and
    # this has to run under BusyBox too.
    # Fail CLOSED. An inspection that does not answer is not an answer: a
    # directory removed at the moment it is read, and recreated before the
    # staging file lands, would otherwise walk straight through a check that
    # silently skipped it.
    listing="$(ls -ld "$path" 2>/dev/null)" ||
        die "cannot inspect $path, so it cannot be shown to be safe to install into"
    [ -n "$listing" ] ||
        die "cannot inspect $path, so it cannot be shown to be safe to install into"
    perms="$(printf %s "$listing" | cut -c1-10)"
    owner="$(printf %s "$listing" | awk '{print $3}')"
    group="$(printf %s "$listing" | awk '{print $4}')"

    # Anything that is not a directory mode is not something to guess from --
    # and not a reason to proceed either.
    case "$perms" in
        d?????????) ;;
        *) die "$path is not a directory, so the install path cannot be trusted" ;;
    esac

    # Extended ACLs, which can grant write to any account while the mode
    # bits look impeccable.
    #
    # Three ways this can go, and only two of them are answers:
    #
    #   * getfacl, which is authoritative. `--skip-base` prints nothing at
    #     all for a file carrying only the three base entries, so anything on
    #     stdout is an extended ACL.
    #   * GNU ls, which marks one with a trailing `+` on the mode string.
    #   * neither, which is BusyBox without the acl package -- and there an
    #     ACL is simply invisible. `ls` does not print the marker and there
    #     is nothing else to ask.
    #
    # The third case refuses. It used to be documented as a known gap and
    # allowed, which meant a root-owned 0755 directory carrying
    # `user:attacker:rwx` sailed through on exactly the systems that could
    # not see it. A gap that is written down is still a gap.
    # shellcheck disable=SC2012  # the elif asks ls what it is, not for a listing
    if command -v getfacl >/dev/null 2>&1; then
        # Status first, then the output. A getfacl that fails -- an
        # implementation without these options, a filesystem that will not
        # answer -- produces empty output, which reads exactly like a file
        # carrying only its base entries. Same fail-open as the group lookup
        # had, one check over.
        acl_entries="$(getfacl --skip-base --omit-header "$path" 2>/dev/null)" ||
            die "cannot read the ACL of $path, so its real permissions are unknown. Check it with: getfacl $path"
        if [ -n "$acl_entries" ]; then
            die "$path carries an extended ACL, which may grant write access that its mode does not show. Inspect it with: getfacl $path"
        fi
    elif ls --version 2>/dev/null | head -n 1 | grep -q coreutils; then
        if [ "$(printf %s "$listing" | cut -c11)" = "+" ]; then
            die "$path carries an extended ACL, which may grant write access that its mode does not show. Inspect it with: getfacl $path"
        fi
    else
        die "cannot tell whether $path carries an extended ACL: this ls does not report them and getfacl is not installed. Install the acl package (on Alpine: apk add acl) and run this again."
    fi

    if [ -n "$owner" ] && [ "$owner" != "root" ] && [ "$owner" != "$SAFE_ME" ]; then
        die "$path belongs to $owner, who could replace what is inside it while this installs. Nothing can be installed there safely."
    fi

    # The sticky bit settles both write bits at once -- for a PARENT. With
    # it set only an entry's owner may unlink or rename that entry, so who
    # else may write into the directory stops mattering for the subtree we
    # already hold. That is what makes /tmp usable as an ancestor, and /tmp
    # is `drwxrwxrwt`, so treating either write bit as disqualifying would
    # refuse every install whose path runs through it.
    #
    # It settles nothing for the DESTINATION. Sticky stops another user
    # removing OUR files; it does not stop them creating `srelens-tui` there
    # first and owning it. Everything that then happens to that entry
    # happens to a file they control: its mode is read and reapplied to the
    # rollback copy, so a planted 4755 becomes a root-owned setuid binary;
    # and it can be swapped for a symlink to a directory after the check, so
    # the `mv` moves the staged binary underneath it and leaves theirs at the
    # path that then gets run. Neither race can be closed from a shell, so
    # the directory is refused instead.
    if [ "$role" = parent ]; then
        case "$(printf %s "$perms" | cut -c10)" in
            t | T) return 0 ;;
        esac
    fi

    if [ "$(printf %s "$perms" | cut -c9)" = "w" ]; then
        die "$path is writable by other users, so one of them could create or replace the binary while this installs. Nothing can be installed there safely."
    fi

    # Group-writable is refused, full stop.
    #
    # This used to try to establish that the group had nobody in it but the
    # owner -- the member list, then the passwd table for accounts whose
    # PRIMARY group it is. Neither can be trusted to be complete: an SSSD or
    # LDAP source can resolve accounts individually while declining to
    # enumerate, so `getent passwd` succeeds and returns only what is local.
    # A remote account in that group is then invisible, and the directory is
    # accepted for exactly the person it should have been refused for.
    #
    # The reason for all that machinery was that Fedora supposedly leaves
    # ~/.local/bin group-writable under a 002 umask. It does not: Fedora 41
    # sets UMASK 022 in login.defs, /etc/bashrc raises umask only when it is
    # 0, and a fresh account gets `drwxr-xr-x`. The case being protected was
    # not real, and it cost thirty-five lines that could not answer the
    # question anyway.
    if [ "$(printf %s "$perms" | cut -c6)" = "w" ]; then
        die "$path is group-writable, and who is in group $group cannot be established well enough to trust it. Make it private first: chmod g-w $path"
    fi
}
# Undo an installation that never got committed.
#
# Committed means the new binary has been run and answered. Until then the
# replacement is unvalidated, so anything that ends the script early -- a
# signal, `set -e`, a die -- has to put back what was there and take away
# what was not. Idempotent on purpose: the failure paths in main do the same
# work, and whichever gets there first leaves nothing for the other.
install_rollback() {
    # KEPT and STUCK are NOT reset here. This runs twice on a signal -- the
    # handler, then EXIT -- and the second pass finds the transaction already
    # cleared and does nothing; resetting them on the way in would erase
    # what the first pass learned before anyone could report it.
    [ -z "$INSTALL_COMMITTED" ] || return 0
    # Each branch clears what it dealt with. This runs TWICE on a signal --
    # the HUP/INT/TERM handler calls it, then `exit` fires the EXIT handler --
    # and a second pass that still saw a destination but no backup would take
    # the fresh-install branch and delete the binary the first pass had just
    # restored.
    if [ -n "$INSTALL_BACKUP" ] && [ -e "$INSTALL_BACKUP" ]; then
        # Something was there before: put it back.
        if [ -n "$INSTALL_DEST" ]; then
            if mv -f "$INSTALL_BACKUP" "$INSTALL_DEST" 2>/dev/null; then
                INSTALL_ROLLBACK_RESTORED=yes
                INSTALL_BACKUP=""
                INSTALL_DEST=""
            else
                # It could not go back -- a read-only mount, a full disk.
                # Deleting it here would destroy the only copy of what was
                # there, so it stays, and the caller says where.
                INSTALL_ROLLBACK_KEPT="$INSTALL_BACKUP"
                INSTALL_BACKUP=""
                INSTALL_DEST=""
            fi
        else
            rm -f "$INSTALL_BACKUP"
            INSTALL_BACKUP=""
        fi
    elif [ -n "$INSTALL_DEST" ]; then
        # Nothing was there before, so the binary at that path is one this
        # run put there and never got to run. On a noexec working directory
        # it has been executed by nobody at all, and leaving it is leaving an
        # unchecked binary on someone's PATH under a name they will trust.
        # `rm -f` says nothing useful -- it is happy about a file that was
        # never there -- so what matters is whether the path is gone
        # afterwards. A read-only mount or an immutable flag leaves it, and
        # reporting it removed would be the same lie the failed restore used
        # to tell.
        rm -f "$INSTALL_DEST" 2>/dev/null || true
        if [ -e "$INSTALL_DEST" ]; then
            INSTALL_ROLLBACK_STUCK="$INSTALL_DEST"
        fi
        INSTALL_DEST=""
    fi
    if [ -n "$INSTALL_STAGED" ]; then
        rm -f "$INSTALL_STAGED"
        INSTALL_STAGED=""
    fi
}

# What an interrupted or failed rollback left behind, said out loud.
#
# main's own failure path says this itself, with the reason the binary was
# rejected, and then clears both so this does not repeat it. The handlers
# have no such message of their own, so this is theirs.
report_rollback() {
    if [ -n "$INSTALL_ROLLBACK_STUCK" ]; then
        printf 'error: interrupted, and the unvalidated %s could NOT be removed. It is still installed at %s -- delete it before running %s from there\n' "$BIN" "$INSTALL_ROLLBACK_STUCK" "$BIN" >&2
        INSTALL_ROLLBACK_STUCK=""
    fi
    if [ -n "$INSTALL_ROLLBACK_KEPT" ]; then
        printf 'error: interrupted, and the previous %s could NOT be put back. It is still on disk: %s\n' "$BIN" "$INSTALL_ROLLBACK_KEPT" >&2
        INSTALL_ROLLBACK_KEPT=""
    fi
    # A restore that went through, but of a file whose extended attributes
    # were never checked: `mv` of the backup carries none, so the previous
    # copy is back and anything root had set on it -- a capability -- is not.
    if [ -n "$INSTALL_ROLLBACK_RESTORED" ] && [ -n "$INSTALL_XATTR_UNCHECKED" ]; then
        printf 'note: interrupted; the previous %s has been put back -- without any extended attributes it had, which were not checked (getfattr is not installed)\n' "$BIN" >&2
        INSTALL_ROLLBACK_RESTORED=""
    fi
}

# Install by rename where possible: a running binary being overwritten in
# place gets ETXTBSY on Linux, while replacing the directory entry does not
# disturb a process already holding the old inode.
install_binary() {
    src="$1"
    dest="$2"

    # `mv file dir` moves the file INTO the directory rather than over it,
    # so a destination that is already a directory -- or a link to one --
    # would leave the staging file inside it and nothing at the path the
    # caller asked for. `mv -T` says otherwise but is GNU-only, and this
    # has to run under BusyBox.
    if [ -d "$dest" ]; then
        die "$dest is a directory, so a binary cannot be installed at that path. Remove it and run this again."
    fi

    # And a symlink cannot be replaced faithfully. The rollback copy is
    # taken by reading $dest, which follows the link and stores its target's
    # bytes -- so a restore would put a regular file where a link had been,
    # silently breaking whatever arrangement the link was part of while
    # reporting that the previous copy was put back.
    # Metadata on the binary being REPLACED. The rollback copy carries bytes,
    # mode, owner and timestamps, and nothing else -- so anything else there
    # would come back missing, and the script would report the previous copy
    # restored while handing back something less capable than what it took.
    #
    # Only when something is actually being replaced: a first install has no
    # previous copy to be faithful to, so nobody installing for the first time
    # is sent off to fetch tools.
    if [ -e "$dest" ]; then
        # An extended ACL. getfacl is already required by the destination
        # checks, so this costs nothing extra.
        if command -v getfacl >/dev/null 2>&1; then
            # Status first, as with every other lookup in this file: a
            # getfacl that cannot read this file produces empty output, which
            # is indistinguishable from a file with no ACL.
            dest_acl="$(getfacl --skip-base --omit-header "$dest" 2>/dev/null)" ||
                die "cannot read the ACL of $dest, so a rollback could not account for it. Check with: getfacl $dest"
            if [ -n "$dest_acl" ]; then
                die "$dest carries an extended ACL, which a rollback could not put back. Remove the ACL, or move the file aside, and run this again."
            fi
        fi

        # Everything else in the xattr namespace -- a file capability above
        # all, whose loss would quietly take privileges from a binary that
        # had them.
        #
        # security.selinux is excluded deliberately. Every file on an SELinux
        # system carries one, so refusing them would refuse every update on
        # Fedora and RHEL -- and it is the one piece here the filesystem
        # re-derives anyway, since the rollback copy is created by mktemp in
        # the destination directory and labelled by the same policy.
        if command -v getfattr >/dev/null 2>&1; then
            # Status FIRST, before any filter. In a pipeline the status is
            # the last command's, so `getfattr | grep | cut | tr` reports on
            # `tr`, which is delighted by no input -- an implementation that
            # rejects an option, or an LSM that refuses enumeration, would
            # read exactly like a file with no attributes.
            attr_dump="$(getfattr -d -m - "$dest" 2>/dev/null)" ||
                die "cannot read the extended attributes of $dest, so a rollback could not account for them. Check with: getfattr -d -m - $dest"

            # Every `name=value` line is an attribute. Matching a guessed
            # shape missed names a namespace happily allows -- `user.0` is
            # legal, and `[a-z_]` does not match a digit.
            all_attrs="$(printf %s "$attr_dump" | grep "=" | cut -d= -f1)" || all_attrs=""
            selinux_label=""
            kept_attrs=""
            for attr in $all_attrs; do
                case "$attr" in
                    security.selinux) selinux_label=yes ;;
                    *) kept_attrs="$kept_attrs $attr" ;;
                esac
            done
            if [ -n "$kept_attrs" ]; then
                die "$dest carries extended attributes a rollback could not put back:$kept_attrs. Remove them, or move the file aside, and run this again."
            fi

            # An SELinux label is not dropped, it is re-derived -- the backup
            # inode is created by mktemp in this directory, so policy gives it
            # the DEFAULT label for the path. That is right for a file that
            # had the default one and wrong for a file that had been given
            # something else, and `mv` does not relabel on the way back.
            #
            # chcon copies the real one across. Without it, a privileged
            # update refuses rather than hand back a binary the policy will
            # treat differently.
            if [ -n "$selinux_label" ]; then
                if command -v chcon >/dev/null 2>&1; then
                    INSTALL_SELINUX_FROM="$dest"
                elif [ "$(id -u)" = "0" ]; then
                    die "$dest carries an SELinux context and chcon is not available to copy it onto a rollback copy. Install policycoreutils, or move the file aside, and run this again."
                fi
            fi
        elif [ "$(id -u)" = "0" ]; then
            # No getfattr, and this is a privileged install. Refuse: the
            # attribute worth caring about is a file capability, and setting
            # one needs CAP_SETFCAP -- so a binary that has one got it from
            # root, and root is exactly who is replacing it now.
            die "cannot tell whether $dest carries extended attributes such as a file capability, and a rollback could not put those back: getfattr is not installed. Install it (Debian: apt install attr; Alpine: apk add attr), or move the file aside, and run this again."
        else
            # Unprivileged and no getfattr: carry on, and say so. Setting a
            # capability needs root, so one on a user's own binary is
            # unusual -- but root CAN put one there, and the backup is made
            # with `cp -p`, which carries mode, owner and times and not
            # xattrs, so a rollback would hand the binary back without it.
            # Not a reason to make every ordinary `~/.local/bin` update need
            # a package a stock GitHub runner does not carry; a reason to say
            # now, and again if it comes to a rollback, that the previous
            # copy comes back without them.
            INSTALL_XATTR_UNCHECKED=yes
            printf 'note: getfattr is not installed, so extended attributes on %s (a file capability, say) were not checked; if this update is rolled back, the previous copy comes back without them\n' "$dest" >&2
        fi
    fi

    # A FIFO, socket or device node where the binary goes. `cp -p` reading a
    # FIFO waits for a writer that never comes, and a device node reads
    # whatever the device feels like -- neither is something to copy, and
    # neither is a binary to replace.
    if [ -e "$dest" ] && [ ! -f "$dest" ] && [ ! -L "$dest" ]; then
        die "$dest is not a regular file, so it is not a binary this can replace. Move it aside and run this again."
    fi

    if [ -L "$dest" ]; then
        die "$dest is a symlink. Install over what it points at, or remove it first: this replaces the path itself and could not put the link back if the new binary failed."
    fi
    # A second hard link to it, for the same reason. The backup is a copy
    # into a fresh inode, so a rollback would put back a file the other
    # name is no longer attached to: the sibling keeps the old inode, the
    # restored path gets a new one, and from then on the two diverge --
    # while the installer had reported the previous copy put back. Status
    # first, as with every lookup here: a stat that fails looks like an
    # empty count.
    if [ -f "$dest" ]; then
        dest_links="$(stat -c %h "$dest" 2>/dev/null)" ||
            die "cannot read the link count of $dest, so a rollback could not be promised to put it back. Check with: stat -c %h $dest"
        case "$dest_links" in
            ''|*[!0-9]*)
                die "cannot read the link count of $dest, so a rollback could not be promised to put it back. Check with: stat -c %h $dest"
                ;;
        esac
        if [ "$dest_links" -gt 1 ]; then
            die "$dest has $dest_links hard links. A rollback restores a copy, not the shared inode, so the other name(s) would be left pointing at the old file. Remove the extra links, or move the file aside, and run this again."
        fi
    fi
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
    INSTALL_STAGED="$staged"
    cp "$src" "$staged" || {
        rm -f "$staged"
        die "cannot write to $dir"
    }
    chmod 0755 "$staged"

    # Keep whatever is being replaced until the new one has been shown to
    # run. Alongside it, so the restore below stays on one filesystem.
    #
    # A copy of the old binary, taken WITHOUT unlinking the live path and
    # WITHOUT releasing the name mktemp reserved.
    #
    # Renaming the old binary aside would leave nothing at the documented
    # path if the process died in the gap. And unlinking the reserved name
    # to `ln` over it -- which is what this did -- opens a window in a
    # sticky directory somebody else can write to: they cannot remove OUR
    # file, but once it is gone they can put a symlink at that name, the
    # link fails because the name exists, and the copy behind it writes
    # through their symlink with our privileges. Writing into the inode
    # mktemp already holds closes both.
    INSTALL_BACKUP=""
    if [ -e "$dest" ]; then
        # The mode it has NOW. A rollback has to put back what was there,
        # not a guess: a binary someone deliberately kept at 0700 must not
        # come back 0755, readable and runnable by everyone on the machine.
        # `stat -c` rather than `chmod --reference`, which BusyBox lacks.
        dest_mode="$(stat -c %a "$dest" 2>/dev/null)" || dest_mode=""
        [ -n "$dest_mode" ] || {
            rm -f "$staged"
            die "cannot read the permissions of $dest, so a rollback could not restore them"
        }
        # Keep the permission bits, never the set-ID ones. `stat %a` renders
        # setuid as a fourth digit, and reapplying it would have this script
        # create a setuid copy of a file it did not write -- owned by root,
        # when the install is under sudo. The destination rules make a
        # planted binary hard to arrange; refusing to propagate the bit at
        # all means it does not matter if one ever is.
        # stat renders a set-ID bit as a fourth digit; drop it. `case`
        # rather than sed, because the regex this used to be was mangled
        # into literal parentheses and a control byte, matched nothing,
        # and stripped nothing -- while looking like it did.
        case "$dest_mode" in
            ????) dest_mode="${dest_mode#?}" ;;
        esac
        INSTALL_BACKUP="$(mktemp "$dir/.$BIN.backup.XXXXXX")" || {
            rm -f "$staged"
            die "cannot create a rollback file in $dir, so $dest will not be replaced"
        }
        # `cp -p` rather than a redirect: it writes into the inode mktemp
        # holds -- so the name is still never released -- and carries the
        # mode, owner and timestamps across as well as the bytes. What it
        # cannot carry is an extended ACL, an xattr or a file capability;
        # those are refused below rather than silently dropped.
        if ! cp -p "$dest" "$INSTALL_BACKUP" 2>/dev/null; then
            rm -f "$INSTALL_BACKUP" "$staged"
            die "cannot preserve the $BIN already at $dest, so it will not be replaced"
        fi
        # While the original is still there to copy it from.
        if [ -n "$INSTALL_SELINUX_FROM" ]; then
            chcon --reference="$INSTALL_SELINUX_FROM" "$INSTALL_BACKUP" 2>/dev/null ||
                die "cannot copy the SELinux context of $dest onto the rollback copy, so $dest will not be replaced"
        fi
        # mktemp makes it 0600; the rollback carries the mode the binary
        # actually had.
        chmod "$dest_mode" "$INSTALL_BACKUP" || {
            rm -f "$INSTALL_BACKUP" "$staged"
            die "cannot set the rollback copy to mode $dest_mode, so $dest will not be replaced"
        }
    fi

    # Recorded BEFORE the rename. A signal arriving while the shell is inside
    # `mv` runs the handler first, and an assignment after it would not have
    # happened yet -- the rollback would then think nothing had been replaced,
    # leave an unvalidated binary live and throw away the only old copy.
    INSTALL_DEST="$dest"
    if ! mv -f "$staged" "$dest"; then
        rm -f "$staged"
        # Nothing was replaced after all: $dest still holds whatever it held,
        # so the rollback must not go near it.
        INSTALL_DEST=""
        INSTALL_STAGED=""
        [ -z "$INSTALL_BACKUP" ] || rm -f "$INSTALL_BACKUP"
        INSTALL_BACKUP=""
        die "cannot replace $dest"
    fi
    INSTALL_STAGED=""
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
