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
  *.tar.gz)         cp "$work/fixtures/\$(basename "\$url")" "\$out" ;;
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

echo "install"

dest="$work/bin"
out="$(sh "$script" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "the latest release installs" "Installed: $dest/srelens-tui" "$out" "$rc" 0
check "the checksum is reported, not assumed" "Checksum verified:" "$out" "$rc" 0

if [ -x "$dest/srelens-tui" ] && "$dest/srelens-tui" --version >/dev/null 2>&1; then
    ok "the installed binary runs: $("$dest/srelens-tui" --version)"
else
    no "the installed binary does not run"
fi

# Installing over an existing copy is the update path, and must not fail on
# ETXTBSY or leave the staging file behind.
out="$(sh "$script" --install-dir "$dest" 2>&1)" && rc=0 || rc=$?
check "installing over an existing copy succeeds" "Installed:" "$out" "$rc" 0
if [ -z "$(find "$dest" -name '.srelens-tui.install.*' 2>/dev/null)" ]; then
    ok "no staging file is left behind"
else
    no "a staging file was left in $dest"
fi

echo
echo "passed $pass, failed $fail"
[ "$fail" -eq 0 ]
