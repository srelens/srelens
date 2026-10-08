#!/bin/sh
# The launcher is non-bundle code: sign without the desktop app's restricted
# App ID entitlements, then let Tauri copy it unchanged before notarization.
set -eu
target="${1:?usage: prepare-launcher.sh TARGET}"
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) echo "unsupported macOS launcher target: $target" >&2; exit 1 ;;
esac
cd "$(dirname "$0")/../.."
cargo build --release --locked --target "$target" -p srelens-plugin-host --bin srelens-sandbox-launch
mkdir -p apps/desktop/src-tauri/sidecars
launcher="apps/desktop/src-tauri/sidecars/srelens-sandbox-launch"
cp "${CARGO_TARGET_DIR:-target}/$target/release/srelens-sandbox-launch" "$launcher"
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    if [ -n "${APPLE_CERTIFICATE:-}" ]; then
        signing_tmp="$(mktemp -d)"
        keychain="$signing_tmp/launcher.keychain-db"
        password="$(uuidgen)"
        trap 'security delete-keychain "$keychain" >/dev/null 2>&1 || true; rm -rf "$signing_tmp"' EXIT
        security create-keychain -p "$password" "$keychain"
        security set-keychain-settings -lut 3600 "$keychain"
        security unlock-keychain -p "$password" "$keychain"
        printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$signing_tmp/cert.p12"
        security import "$signing_tmp/cert.p12" -k "$keychain" -P "${APPLE_CERTIFICATE_PASSWORD:?missing certificate password}" -T /usr/bin/codesign
        security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
        codesign --force --options runtime --timestamp --keychain "$keychain" --sign "$APPLE_SIGNING_IDENTITY" "$launcher"
    else
        codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$launcher"
    fi
    codesign --verify --strict "$launcher"
fi
