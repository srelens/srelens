#!/bin/sh
# Tests for prepare-launcher.sh's signing keychain.
#
#   sh packaging/macos/test.sh
#
# Runs anywhere with a POSIX sh, macOS or not: fake `security`, `codesign`,
# `cargo` and `uuidgen` go ahead of the real ones on PATH, and the script runs
# from a copy in a temporary repository layout, so nothing touches the real
# keychains, the checkout, or a cargo build.
#
# This exists because the launcher's signing only ever runs on a signed macOS
# release runner, and it failed there first: v0.16.0 shipped no macOS build
# when codesign could not find an identity in a keychain that was not on the
# search list (#862).
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/prepare-launcher.sh"
[ -f "$script" ] || { echo "prepare-launcher.sh not found next to $0" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A repository layout the script can `cd` into and write its sidecar under.
mkdir -p "$work/repo/packaging/macos" "$work/target/aarch64-apple-darwin/release" "$work/bin"
cp "$script" "$work/repo/packaging/macos/prepare-launcher.sh"
: > "$work/target/aarch64-apple-darwin/release/srelens-sandbox-launch"

# `security list-keychains -d user` prints the list the way the real one does,
# indented and quoted, one per line, or fails when FAKE_LIST_FAILS is set.
# `list-keychains -d user -s ...` records each argument on its own line, and
# fails the restore (the one call without the temporary keychain) when
# FAKE_RESTORE_FAILS is set. `find-identity` records the keychain it was asked
# about in FAKE_CALLS, where codesign records each call too, so their order shows.
cat > "$work/bin/security" <<'EOF'
#!/bin/sh
if [ "$1" = list-keychains ] && [ "${4:-}" = -s ]; then
    shift 4
    echo "--- set" >> "$FAKE_LOG"
    temporary=""
    for keychain in "$@"; do
        echo "[$keychain]" >> "$FAKE_LOG"
        case "$keychain" in */launcher.keychain-db) temporary=yes ;; esac
    done
    [ -n "$temporary" ] || [ -z "${FAKE_RESTORE_FAILS:-}" ] || exit 1
    exit 0
fi
if [ "$1" = list-keychains ]; then
    [ -z "${FAKE_LIST_FAILS:-}" ] || exit 1
    printf '    "%s"\n' "/Users/a/Library/Keychains/login.keychain-db" \
        "/Users/a/Library/Keychains/Release Signing.keychain-db"
fi
if [ "$1" = find-identity ]; then
    for keychain in "$@"; do :; done
    echo "find-identity $keychain" >> "$FAKE_CALLS"
fi
exit 0
EOF
cat > "$work/bin/codesign" <<'EOF'
#!/bin/sh
echo codesign >> "$FAKE_CALLS"
exit "${FAKE_CODESIGN_STATUS:-0}"
EOF
cat > "$work/bin/cargo" <<'EOF'
#!/bin/sh
exit 0
EOF
cat > "$work/bin/uuidgen" <<'EOF'
#!/bin/sh
echo 00000000-0000-0000-0000-000000000000
EOF
chmod +x "$work/bin/security" "$work/bin/codesign" "$work/bin/cargo" "$work/bin/uuidgen"

failures=0
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

# Run the script against the fakes; the exit status lands in $status and every
# search-list change in $FAKE_LOG.
run() {
    FAKE_LOG="$work/log"
    FAKE_CALLS="$work/calls"
    : > "$FAKE_LOG"
    : > "$FAKE_CALLS"
    status=0
    env PATH="$work/bin:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_CALLS="$FAKE_CALLS" CARGO_TARGET_DIR="$work/target" \
        APPLE_SIGNING_IDENTITY="Developer ID Application: Test" \
        APPLE_CERTIFICATE="$(printf 'not a p12' | base64)" APPLE_CERTIFICATE_PASSWORD=secret \
        "$@" sh "$work/repo/packaging/macos/prepare-launcher.sh" aarch64-apple-darwin \
        > "$work/out" 2>&1 || status=$?
}

# The search-list changes the run made, one path per line.
changes() { cat "$FAKE_LOG"; }
# The Nth change alone.
change() { awk -v want="$1" '/^--- set$/ { n++; next } n == want' "$FAKE_LOG"; }

expected_original='[/Users/a/Library/Keychains/login.keychain-db]
[/Users/a/Library/Keychains/Release Signing.keychain-db]'

echo "case: the temporary keychain joins the search list, and the original comes back whole"
run
[ "$status" = 0 ] || fail "signed run exited $status: $(cat "$work/out")"
temporary="$(change 1 | sed -n 1p)"
case "$temporary" in
    *"/launcher.keychain-db]") ;;
    *) fail "the first search list does not start with the temporary keychain: $temporary" ;;
esac
[ "$(change 1 | sed 1d)" = "$expected_original" ] \
    || fail "the original keychains did not follow it whole: $(changes)"
[ "$(changes | grep -c '^--- set$')" = 2 ] || fail "expected one change and one restore: $(changes)"
[ "$(change 2)" = "$expected_original" ] \
    || fail "the original search list was not restored whole: $(changes)"
# The diagnostic shows what the temporary keychain holds, before the signing.
case "$(sed -n 1p "$FAKE_CALLS")" in
    "find-identity "*"/launcher.keychain-db") ;;
    *) fail "find-identity did not report the temporary keychain before signing: $(cat "$FAKE_CALLS")" ;;
esac
[ "$(sed -n 2p "$FAKE_CALLS")" = codesign ] || fail "signing did not follow the diagnostic: $(cat "$FAKE_CALLS")"

echo "case: a search list that cannot be read is never replaced"
run FAKE_LIST_FAILS=1
[ "$status" != 0 ] || fail "the run carried on without the original search list"
[ ! -s "$FAKE_LOG" ] || fail "the search list was changed after it could not be read: $(changes)"

echo "case: the original search list comes back even when signing fails"
run FAKE_CODESIGN_STATUS=1
[ "$status" != 0 ] || fail "a failed codesign did not fail the run"
[ "$(change 2)" = "$expected_original" ] \
    || fail "the original search list was not restored after the failure: $(changes)"

echo "case: a search list that cannot be restored fails the run"
run FAKE_RESTORE_FAILS=1
[ "$status" != 0 ] || fail "the run passed with the search list still naming a deleted keychain"
[ "$(changes | grep -c '^--- set$')" = 2 ] || fail "the restore was not attempted: $(changes)"

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed" >&2
    exit 1
fi
echo "all checks passed"
