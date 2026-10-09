#!/bin/sh
# Touch ID unlock (#819) needs the App ID entitlement, a restricted one: macOS
# launches the app only if its embedded provisioning profile grants that
# entitlement, names the certificate the app is signed with, and hasn't
# expired. Notarization checks none of it, so a mismatch ships an app no Mac
# will open. This fails unless the signed APP passes all three.
#
#   sh packaging/macos/check-profile.sh target/aarch64-apple-darwin/release/bundle/macos/srelens.app
#
# release.yml runs it over each signed macOS app it is about to publish.
set -eu

app="${1:?usage: check-profile.sh APP}"
profile="$app/Contents/embedded.provisionprofile"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
    echo "::error::$*" >&2
    exit 1
}

# PlistBuddy, not plutil: entitlement keys contain dots, which plutil reads as
# a key path. PlistBuddy prints its errors to stdout, so keep only a success.
key() {
    value="$(/usr/libexec/PlistBuddy -c "Print :$2" "$1" 2>/dev/null)" && printf '%s\n' "$value"
}

[ -f "$profile" ] || fail "$app has no Contents/embedded.provisionprofile"
security cms -D -i "$profile" > "$tmp/profile.plist" 2>/dev/null || fail "cannot decode $profile"

# Both ISO 8601 UTC, so they order as strings.
expires="$(plutil -extract ExpirationDate raw -o - "$tmp/profile.plist")"
expr "$expires" \> "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > /dev/null || fail "the profile expired on $expires"

codesign -d --extract-certificates="$tmp/cert" "$app" 2>/dev/null || fail "cannot read how $app is signed"
[ -f "$tmp/cert0" ] || fail "$app is not signed with a certificate"
leaf="$(base64 < "$tmp/cert0")"
i=0
while cert="$(plutil -extract "DeveloperCertificates.$i" raw -o - "$tmp/profile.plist" 2>/dev/null)"; do
    [ "$cert" = "$leaf" ] && break
    i=$((i + 1))
done
[ "$cert" = "$leaf" ] || fail "$app is signed with a certificate its profile does not name — regenerate the profile for the current APPLE_CERTIFICATE"

codesign -d --entitlements - --xml "$app" > "$tmp/signed.plist" 2>/dev/null || fail "cannot read the entitlements $app is signed with"
app_id="$(key "$tmp/signed.plist" com.apple.application-identifier)" || fail "$app is not signed with com.apple.application-identifier"
granted="$(key "$tmp/profile.plist" Entitlements:com.apple.application-identifier)" || true
[ "$app_id" = "$granted" ] || fail "$app claims $app_id, but its profile grants ${granted:-no App ID}"

echo "OK: $app claims $app_id, granted by its profile for its signing certificate until $expires"
