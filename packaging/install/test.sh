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

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM

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

out="$(sh "$script" --install-dir= 2>&1)" && rc=0 || rc=$?
check "an empty --install-dir= is refused too" "needs a value" "$out" "$rc" 1

echo "platform"

mkdir -p "$work/fake"
cat > "$work/fake/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) echo "${FAKE_OS:-Linux}" ;;
  -m) echo "${FAKE_ARCH:-x86_64}" ;;
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

archive="srelens-tui-$version-x86_64-unknown-linux-musl.tar.gz"
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

dest="$work/cksum"
out="$(PATH="$work/fake:$PATH" sh "$script" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
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
dest="$work/latest"
out="$(SERVE_GOOD=1 PATH="$work/fake:$PATH" sh "$script" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "the newest release is resolved from the API" "srelens-tui $version" "$out" "$rc" 0
check "and installed" "Installed: $dest/srelens-tui" "$out" "$rc" 0

echo "install"

dest="$work/bin"
out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "the release installs" "Installed: $dest/srelens-tui" "$out" "$rc" 0
check "the checksum is reported, not assumed" "Checksum verified:" "$out" "$rc" 0

if [ -x "$dest/srelens-tui" ] && "$dest/srelens-tui" --version >/dev/null 2>&1; then
    ok "the installed binary runs: $("$dest/srelens-tui" --version)"
else
    no "the installed binary does not run"
fi

# Installing over an existing copy is the update path, and must not fail on
# ETXTBSY or leave the staging file behind.
out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "installing over an existing copy succeeds" "Installed:" "$out" "$rc" 0
if [ -z "$(find "$dest" -name '.srelens-tui.install.*' 2>/dev/null)" ]; then
    ok "no staging file is left behind"
else
    no "a staging file was left in $dest"
fi

echo "through a pipe"

# How the documented one-liner actually runs. Options cannot follow a bare
# `sh` -- it reads them as its own -- so the docs say `sh -s --`, and this
# proves that form reaches the script's parser.
dest="$work/piped"
out="$(cat "$script" | sh -s -- --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "options survive sh -s --" "Installed: $dest/srelens-tui" "$out" "$rc" 0

echo "staging file"

# The staging name must not be derivable from the pid: installed as root
# into a directory another user can write to, a predictable name can be
# pre-created as a symlink, and cp writes through it. mktemp names cannot
# be aimed at, and a symlink sitting in the directory is left alone.
dest="$work/staging"
mkdir -p "$dest"
echo "do not touch me" > "$work/canary"
ln -sf "$work/canary" "$dest/.srelens-tui.install.99999"
out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
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

# An unpredictable staging name does not survive a directory other users
# can unlink from: they can take the staged file away and leave a symlink,
# or replace the finished binary before it is run. Only the directory's own
# permissions close that, so an unsafe one is refused outright.
dest="$work/world"
mkdir -p "$dest"
chmod 0777 "$dest"
out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "a world-writable destination is refused" "writable by anyone" "$out" "$rc" 1
if [ -e "$dest/srelens-tui" ]; then
    no "it installed into the world-writable directory anyway"
else
    ok "nothing was installed there"
fi

# The sticky bit is what makes /tmp safe: only an entry's owner may unlink
# it, so the staged file cannot be taken away. That case must still work.
dest="$work/sticky"
mkdir -p "$dest"
chmod 1777 "$dest"
out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "world-writable WITH the sticky bit still installs" "Installed:" "$out" "$rc" 0

# Root writing into a directory root does not own is the case worth
# refusing outright: the owner can arrange the swap at leisure and gets a
# root-written file out of it.
if [ "$(id -u)" = "0" ] && command -v useradd >/dev/null 2>&1; then
    id -u tester >/dev/null 2>&1 || useradd -m tester >/dev/null 2>&1 || true
    dest="$work/theirs"
    mkdir -p "$dest"
    if chown tester "$dest" 2>/dev/null; then
        out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
        check "root into another user's directory is refused" "belongs to tester" "$out" "$rc" 1
        if [ -e "$dest/srelens-tui" ]; then
            no "it installed into the other user's directory anyway"
        else
            ok "nothing was installed there either"
        fi
    else
        echo "  skip  could not chown a directory to another user"
    fi
else
    echo "  skip  not root, or no useradd: cannot test the root-into-foreign-dir refusal"
fi

# A symlink is not the directory it points at. `ls -ld` on one reports
# `lrwxrwxrwx` owned by whoever made the link, so inspecting the path as
# given would describe the link and let --install-dir <link> past every
# check while the install lands somewhere else entirely.
target="$work/unsafe-target"
mkdir -p "$target"
chmod 0777 "$target"
link="$work/looks-fine"
ln -sfn "$target" "$link"
out="$(sh "$script" --version "$version" --install-dir "$link" 2>&1)" && rc=0 || rc=$?
check "a symlink to an unsafe directory is refused" "writable by anyone" "$out" "$rc" 1
if [ -e "$target/srelens-tui" ]; then
    no "it installed through the symlink anyway"
else
    ok "nothing was installed through the symlink"
fi

# The sticky bit stops OTHER users unlinking entries, but never the
# directory's owner, who may remove anything inside it. A world-writable
# sticky directory belonging to someone else is still theirs to tamper
# with -- the same condition self_update.rs already applies.
if [ "$(id -u)" = "0" ] && command -v useradd >/dev/null 2>&1; then
    id -u tester >/dev/null 2>&1 || useradd -m tester >/dev/null 2>&1 || true
    dest="$work/their-sticky"
    mkdir -p "$dest"
    chmod 1777 "$dest"
    if chown tester "$dest" 2>/dev/null; then
        out="$(sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
        check "a sticky directory owned by someone else is refused" "owned by tester" "$out" "$rc" 1
    else
        echo "  skip  could not chown a sticky directory to another user"
    fi
else
    echo "  skip  not root, or no useradd: cannot test the foreign sticky directory"
fi

# A safe symlink must still install -- through the directory it points at,
# named canonically. Approving the resolved path but staging and running
# through the path as given would leave the link repointable after the check.
target="$work/real-bin"
mkdir -p "$target"
link="$work/link-to-real"
ln -sfn "$target" "$link"
out="$(sh "$script" --version "$version" --install-dir "$link" 2>&1)" && rc=0 || rc=$?
check "a safe symlink installs into its target" "Installed: $target/srelens-tui" "$out" "$rc" 0
if [ -x "$target/srelens-tui" ]; then
    ok "the binary landed in the resolved directory"
else
    no "nothing landed in the resolved directory"
fi

echo "hashing tool"

# Every image this was first tested on has sha256sum, which is why the
# shasum branch could ship computing SHA-1 and pass. Running with a PATH
# that deliberately lacks sha256sum is the only way to take that branch.
if command -v shasum >/dev/null 2>&1; then
    limited="$work/limited"
    mkdir -p "$limited"
    missing=''
    for tool in sh uname curl sed head cut grep tar gzip chmod mktemp dirname mkdir cp mv rm ln cat ls awk id shasum; do
        path="$(command -v "$tool" 2>/dev/null)" || { missing="$missing $tool"; continue; }
        ln -sf "$path" "$limited/$tool"
    done
    if [ -n "$missing" ]; then
        echo "  skip  no shasum-only run:$missing not found"
    else
        dest="$work/shasum"
        out="$(PATH="$limited" sh "$script" --version "$version" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
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
