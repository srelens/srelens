#!/bin/sh
# Tests for check-launcher.sh.
#
#   sh packaging/linux/test.sh
#
# Needs dpkg-deb, ar, rpm and rpmbuild (on Ubuntu: apt-get install rpm). Each case
# builds a deb, an rpm and a stand-in AppImage, as tauri leaves them under
# target/release/bundle, and runs the check over them.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
check="$here/check-launcher.sh"
[ -f "$check" ] || { echo "check-launcher.sh not found next to $0" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0
# How bundle builds its deb: as tauri does, or (dpkg) with dpkg-deb --build.
deb_style=tauri

# A stand-in launcher: run with no arguments, the real one says this and
# exits 125. $1 is the exit status it gives instead.
launcher() {
    printf '#!/bin/sh\necho "srelens-sandbox-launch: missing --data DIR" >&2\nexit %s\n' "$1"
}

# bundle DIR DEB_MODE RPM_MODE APPIMAGE_STATUS: a bundle directory. A mode of
# "none" leaves the launcher out of that package.
bundle() {
    dir="$1"
    mkdir -p "$dir/deb" "$dir/rpm" "$dir/appimage"

    root="$work/debroot"
    rm -rf "$root"
    mkdir -p "$root/DEBIAN" "$root/usr/bin"
    printf 'Package: srelens\nVersion: 1.0.0\nArchitecture: all\nMaintainer: t\nDescription: t\n' \
        > "$root/DEBIAN/control"
    if [ "$2" != none ]; then
        launcher 125 > "$root/usr/bin/srelens-sandbox-launch"
        chmod "$2" "$root/usr/bin/srelens-sandbox-launch"
    fi
    deb="$dir/deb/srelens_1.0.0_amd64.deb"
    if [ "$deb_style" = dpkg ]; then
        # dpkg-deb names each path with a leading "./".
        dpkg-deb --root-owner-group --build "$root" "$deb" > /dev/null
    else
        # As tauri builds it: an ar archive whose data.tar.gz names its paths
        # without the "./".
        printf '2.0\n' > "$work/debian-binary"
        tar -C "$root/DEBIAN" -czf "$work/control.tar.gz" control
        tar -C "$root" --owner=0 --group=0 -czf "$work/data.tar.gz" usr
        rm -f "$deb"
        (cd "$work" && ar rc "$deb" debian-binary control.tar.gz data.tar.gz)
    fi

    top="$work/rpmtop"
    rm -rf "$top"
    mkdir -p "$top/SPECS"
    launcher 125 > "$work/launcher"
    {
        printf 'Name: srelens\nVersion: 1.0.0\nRelease: 1\nSummary: t\nLicense: MIT\nBuildArch: noarch\n'
        printf '%%description\nt\n%%install\nmkdir -p %%{buildroot}/usr/bin\n'
        if [ "$3" != none ]; then
            printf 'install -m %s %s %%{buildroot}/usr/bin/srelens-sandbox-launch\n' "$3" "$work/launcher"
            printf '%%files\n%%attr(%s,root,root) /usr/bin/srelens-sandbox-launch\n' "$3"
        else
            printf 'touch %%{buildroot}/usr/bin/srelens\n%%files\n/usr/bin/srelens\n'
        fi
    } > "$top/SPECS/srelens.spec"
    rpmbuild --define "_topdir $top" --define '__os_install_post %{nil}' \
        -bb "$top/SPECS/srelens.spec" > /dev/null 2>&1
    cp "$top"/RPMS/noarch/*.rpm "$dir/rpm/srelens-1.0.0-1.x86_64.rpm"

    # The stand-in AppImage unpacks itself, as --appimage-extract does. Its $1
    # is the stand-in's own argument, written out literally.
    {
        # shellcheck disable=SC2016
        printf '#!/bin/sh\n[ "$1" = --appimage-extract ] || exit 2\nmkdir -p squashfs-root/usr/bin\n'
        printf "cat > squashfs-root/usr/bin/srelens-sandbox-launch <<'EOF'\n"
        launcher "$4"
        printf 'EOF\nchmod 755 squashfs-root/usr/bin/srelens-sandbox-launch\n'
    } > "$dir/appimage/srelens_1.0.0_amd64.AppImage"
    chmod 755 "$dir/appimage/srelens_1.0.0_amd64.AppImage"
}

# expect NAME STATUS TEXT DIR: the check over DIR exits STATUS and says TEXT.
expect() {
    set +e
    out="$(sh "$check" "$4" 2>&1)"
    got=$?
    set -e
    if [ "$got" -eq "$2" ] && printf '%s' "$out" | grep -qF "$3"; then
        echo "ok   $1"
    else
        echo "FAIL $1: status $got (want $2), output: $out"
        failures=$((failures + 1))
    fi
}

# case_ NAME STATUS TEXT DEB_MODE RPM_MODE APPIMAGE_STATUS
case_() {
    bundle "$work/$1" "$4" "$5" "$6"
    expect "$1" "$2" "$3" "$work/$1"
}

case_ all-three-ship-it 0 "OK: the deb, rpm and AppImage ship" 755 755 125
case_ the-deb-lacks-it 1 "srelens_1.0.0_amd64.deb does not ship /usr/bin/srelens-sandbox-launch" none 755 125
case_ the-deb-copy-is-0644 1 "srelens_1.0.0_amd64.deb ships /usr/bin/srelens-sandbox-launch as -rw-r--r--, not -rwxr-xr-x" 644 755 125
case_ the-rpm-lacks-it 1 "srelens-1.0.0-1.x86_64.rpm does not ship /usr/bin/srelens-sandbox-launch" 755 none 125
case_ the-rpm-copy-is-0644 1 "srelens-1.0.0-1.x86_64.rpm ships /usr/bin/srelens-sandbox-launch as -rw-r--r--, not -rwxr-xr-x" 755 644 125
case_ the-appimage-copy-is-not-the-launcher 1 "exited 0, not 125" 755 755 0
deb_style=dpkg
case_ a-deb-that-names-its-paths-with-dot-slash 0 "OK: the deb, rpm and AppImage ship" 755 755 125
case_ a-dot-slash-deb-copy-is-0644 1 "not -rwxr-xr-x" 644 755 125
deb_style=tauri

# A package its tool cannot read is the tool's report, not "does not ship".
bundle "$work/unreadable-rpm" 755 755 125
printf 'not an rpm' > "$work/unreadable-rpm/rpm/srelens-1.0.0-1.x86_64.rpm"
expect an-rpm-rpm-cannot-read 1 "rpm cannot list" "$work/unreadable-rpm"

# Two of a kind: the check does not pick one.
bundle "$work/two-debs" 755 755 125
cp "$work/two-debs/deb/srelens_1.0.0_amd64.deb" "$work/two-debs/deb/srelens_1.0.1_amd64.deb"
expect two-debs 1 "more than one deb/*.deb under" "$work/two-debs"

rm "$work/all-three-ship-it/appimage/"*.AppImage
expect no-appimage 1 "no appimage/*.AppImage under" "$work/all-three-ship-it"

[ "$failures" -eq 0 ] || { echo "$failures failed"; exit 1; }
echo "all passed"
