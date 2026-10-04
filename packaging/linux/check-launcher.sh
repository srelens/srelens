#!/bin/sh
# Executable apps on Linux need the sandbox launcher beside srelens
# (apps/desktop/src-tauri/tauri.linux.conf.json). This fails unless each
# Linux bundle under DIR ships it as /usr/bin/srelens-sandbox-launch with
# mode 0755, and the AppImage's copy is the launcher itself.
#
#   sh packaging/linux/check-launcher.sh target/release/bundle
#
# release.yml runs it over the bundles it is about to publish; test.sh runs it
# over fixtures. Needs dpkg-deb and rpm.
set -eu

dir="${1:?usage: check-launcher.sh BUNDLE_DIR}"
path="usr/bin/srelens-sandbox-launch"

fail() {
    echo "::error::$*" >&2
    exit 1
}

# The one file under $dir matching the pattern $1.
only() {
    found=""
    for file in "$dir"/$1; do
        [ -e "$file" ] || fail "no $1 under $dir"
        [ -z "$found" ] || fail "more than one $1 under $dir"
        found="$file"
    done
    printf '%s\n' "$found"
}

# listed LISTING PACKAGE: the listing's line for $path must exist and start
# -rwxr-xr-x. A listing names it usr/bin/…, ./usr/bin/… (dpkg-deb --build) or
# /usr/bin/… (rpm); tauri's deb uses the first.
listed() {
    line="$(printf '%s\n' "$1" | awk -v p="$path" '{ n = $NF; sub(/^\.?\//, "", n) } n == p')"
    [ -n "$line" ] || fail "$2 does not ship /$path"
    case "$line" in
        -rwxr-xr-x*) ;;
        *) fail "$2 ships /$path as ${line%% *}, not -rwxr-xr-x" ;;
    esac
}

deb="$(only 'deb/*.deb')"
listed "$(dpkg-deb -c "$deb")" "$deb"

rpm="$(only 'rpm/*.rpm')"
listed "$(rpm -qlpv "$rpm" 2> /dev/null)" "$rpm"

appimage="$(only 'appimage/*.AppImage')"
case "$appimage" in
    /*) ;;
    *) appimage="$PWD/$appimage" ;;
esac
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
(cd "$work" && "$appimage" --appimage-extract > /dev/null)
copy="$work/squashfs-root/$path"
[ -f "$copy" ] || fail "$appimage does not ship /$path"
mode="$(stat -c %a "$copy")"
[ "$mode" = 755 ] || fail "$appimage ships /$path as mode $mode, not -rwxr-xr-x"
# Run with no arguments, the launcher refuses, saying what it lacks, and exits
# 125, as it does when it cannot apply a layer. Anything else is not it.
set +e
said="$("$copy" 2>&1)"
code=$?
set -e
[ "$code" -eq 125 ] || fail "$appimage's /$path exited $code, not 125: $said"
case "$said" in
    *"missing --data DIR"*) ;;
    *) fail "$appimage's /$path is not the sandbox launcher: $said" ;;
esac

echo "OK: the deb, rpm and AppImage ship /$path"
