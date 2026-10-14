# AUR package (`srelens-bin`)

Arch / CachyOS / Manjaro / EndeavourOS:

```sh
paru -S srelens-bin     # or: yay -S srelens-bin
```

## Why this channel exists

An AppImage vendors its own copies of platform libraries and loads them ahead of
the system's. Releases **up to 0.2.1** bundled `libwayland-{client,cursor,egl,server}`
that way, so on a rolling distro with a much newer Mesa the host's EGL resolved
against those stale libs, `eglGetDisplay()` failed with `EGL_BAD_PARAMETER`, and
srelens opened a **blank window with no error** (#111, first reported on
CachyOS + GNOME). Later AppImages exclude those libs and CI asserts they stay
out, so the AppImage is no longer affected — but it remains a *class* of bug that
bundling invites and this package cannot have.

This package links the system's own `webkit2gtk-4.1` / `gtk3` / wayland / Mesa,
so that entire class of failure is *structurally impossible* rather than patched
around.

## Design notes

- **`-bin`, not a source build.** AUR policy requires the `-bin` suffix for a
  package that ships a prebuilt binary. It repackages the upstream `.deb`, whose
  payload is exactly `usr/bin` + a `.desktop` entry + hicolor icons.
- **The sandbox launcher comes with the `.deb`.** `usr/bin/srelens-sandbox-launch`,
  which executable apps need beside `srelens`, is in the payload `package()`
  unpacks, so nothing here names it.
- **Dependencies come from the binary's real `DT_NEEDED`,** not guesswork:
  `webkit2gtk-4.1` (which supplies javascriptcore + libsoup3) and `gtk3` (which
  pulls cairo/gdk-pixbuf/glib/dbus). Note the binary never links `libwayland`
  directly — it arrives transitively via GTK/Mesa, which is exactly why the
  AppImage bundling it was both unnecessary and harmful (fixed since 0.2.1).
- **`kubectl` and `helm` are `optdepends`, not `depends`.** srelens deliberately
  drives whatever toolchain is already on your machine rather than bundling one;
  hard-requiring them would contradict that design.
- The `.desktop` entry upstream ships an empty `Categories=`, which can hide the
  app from desktop menus; the package fills it in until that's fixed upstream.

## In-app updates

srelens's built-in updater cannot drive this install — the binary is repacked
from the `.deb`, so it self-identifies as a dpkg bundle on a system with no
dpkg, and self-replacing `/usr/bin/srelens` would desync pacman's database
anyway. The app detects this and, when a new version is out, points at the
package manager (`paru -Syu` / `yay -Syu`) instead of offering an in-app
install; the update itself arrives through the `aur` job below, which bumps the
AUR package on every stable release. (Builds up to 0.2.0 predate that detection
and show a "Download & install" button that fails with `InvalidUpdaterFormat` —
harmless, but update via the package manager.)

## How it's published

`PKGBUILD` here is the source of truth. On every **stable** release (cut from
`main`), the `aur` job in `.github/workflows/release.yml` runs in an
`archlinux:base-devel` container and:

1. clones `ssh://aur@aur.archlinux.org/srelens-bin.git`,
2. rewrites `pkgver` to the released version,
3. runs `updpkgsums` to pin real `sha256sums` from the published release assets,
4. regenerates `.SRCINFO` (the AUR reads its metadata from that file, not the PKGBUILD),
5. **actually builds the package** and lints it with `namcap` — a PKGBUILD that
   doesn't build must never reach users,
6. pushes, and no-ops if nothing changed.

The push uses plain `git`/`ssh` rather than a third-party action **on purpose**:
an AUR PKGBUILD executes arbitrary code on a user's machine at build time, so
push access to this package is a high-value target. Handing the AUR SSH key to an
external action would put that in someone else's supply chain to save ~20 lines
of shell.

Dev pre-releases (`srelens-v<version>-<run>`, cut from `dev`) are deliberately
**not** published: AUR users expect stable versions, and an Arch `pkgver` cannot
contain the `-<run>` suffix anyway.

## Publishing by hand

The steps live in their own reusable workflow (`.github/workflows/aur-publish.yml`),
so the same publish can be run manually for any already-released version:

**Actions → Publish AUR → Run workflow**, optionally with a version (omit it to
use the newest stable release).

This exists because AUR fell four releases behind: the publish was welded to the
release pipeline, it failed silently on every stable release from v0.2.0 to
v0.4.1, and the only way to retry was to cut another release.

**AUR holds exactly one version.** `pkgver` is a scalar and users install
whatever it currently points at, so there is no back-catalogue to backfill —
publishing older tags in sequence would simply land on the newest. Only the
latest version is ever worth pushing.

## One-time setup (needed before the first publish)

The automation cannot run until a human claims the package name on the AUR:

1. Create an account at <https://aur.archlinux.org> and add an SSH public key to it.
2. Claim the name with a first manual push:
   ```sh
   git clone ssh://aur@aur.archlinux.org/srelens-bin.git
   cd srelens-bin
   cp /path/to/srelens/packaging/aur/PKGBUILD .
   # set pkgver to the released version, then:
   updpkgsums
   makepkg --printsrcinfo > .SRCINFO
   makepkg -si            # verify it builds AND runs locally
   git add PKGBUILD .SRCINFO && git commit -m "Initial import" && git push
   ```
3. Add these repository secrets:
   - `AUR_USERNAME` — your AUR account name
   - `AUR_EMAIL` — the email on that account
   - `AUR_SSH_PRIVATE_KEY` — the private half of the key from step 1

   There is deliberately **no** `AUR_KNOWN_HOSTS` secret. The publish job fetches
   AUR's host key with `ssh-keyscan` and refuses to continue unless its
   fingerprint matches a constant pinned in `.github/workflows/release.yml`
   (currently ED25519 `SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`) — so
   an unexpected key still stops the push, but the trust decision lives in
   reviewable code rather than in an opaque secret whose formatting nobody can
   check. A secret that lost its newlines on paste is what broke the v0.4.0
   publish, with ssh reporting only "Host key verification failed".

   If AUR rotates its host key, verify the new one against Arch's published
   fingerprints and update the constant in the workflow:
   ```
   ssh-keyscan -t ed25519 aur.archlinux.org | ssh-keygen -lf -
   ```

   **Keep every ssh path in that job absolute, and pass `known_hosts` to ssh
   explicitly.** A container job runs with `HOME=/github/home` while the
   container user is root, whose passwd entry says `/root`. The shell expands
   `~` via `$HOME`; ssh resolves its *default* `UserKnownHostsFile` against the
   password database. Relying on `~` therefore writes the file to one path and
   reads it from another — which is how v0.4.1 managed to log "host key
   verified" and then fail with "Host key verification failed" on the very next
   line.

The current stable release is **`srelens-v0.2.0`**, so the initial import can be
done against it today (the PKGBUILD builds cleanly against its published `.deb`).
Rolling `dev` pre-releases (`srelens-v<version>-<run>`) are never published: AUR
expects released versions, and an Arch `pkgver` cannot carry the `-<run>` suffix.

## Verifying a change locally (on Arch)

```sh
cd packaging/aur
updpkgsums && makepkg --printsrcinfo > .SRCINFO
namcap PKGBUILD
makepkg -si
```
