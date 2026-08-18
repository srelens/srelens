# Installing srelens

Signed builds are published on the
[latest release](https://github.com/srelens/srelens/releases/latest) for macOS,
Linux, and Windows. Once installed, srelens updates itself from **Settings →
Updates** (Stable or Dev channel).

Prefer to build from source? See the [developer guide](DEVELOPMENT.md).

## macOS

Builds are Developer-ID signed and notarized, so they open without warnings.

1. Download the `.dmg` for your chip:
   - Apple Silicon (M1/M2/M3/…): `srelens_<version>_aarch64.dmg`
   - Intel: `srelens_<version>_x64.dmg`
2. Open the `.dmg` and drag **srelens** into **Applications**.

> **Kubeconfigs with exec auth (OIDC, cloud CLIs)?** srelens resolves your
> login-shell `PATH` at startup, so tools like `kubectl` and its plugins are
> found even when launched from the Dock. If a context still can't find its
> credential plugin, make sure the tool is installed and on your shell `PATH`.

## Linux

Pick the package for your distribution:

| Format | File | Install |
| --- | --- | --- |
| AppImage | `srelens_<version>_amd64.AppImage` | `chmod +x srelens_*.AppImage && ./srelens_*.AppImage` |
| Debian/Ubuntu | `srelens_<version>_amd64.deb` | `sudo apt install ./srelens_*.deb` |
| Fedora/RHEL | `srelens-<version>-1.x86_64.rpm` | `sudo dnf install ./srelens-*.rpm` |

Requires a WebKitGTK runtime (`webkit2gtk-4.1`); the deb/rpm pull it in
automatically. The in-app updater applies to the **AppImage** build; update
deb/rpm installs by downloading the new package.

The AppImage deliberately does **not** bundle the Wayland client libraries
(`libwayland-client/cursor/egl/server`) — bundling them breaks EGL on hosts with
a newer Mesa, which opened a blank window on rolling distros (#111). They come
from the host instead. Any desktop that can run GTK apps already has them; on a
stripped-down image without them, install your distribution's `libwayland`
runtime packages (Debian/Ubuntu: `libwayland-client0`, `libwayland-cursor0`,
`libwayland-egl1`, `libwayland-server0`).

### Verifying Linux downloads (optional)

Each Linux asset ships a Tauri updater signature (`.sig`), used by the in-app
updater. GPG signing of release assets — which will cover every platform, not
just Linux — is being enabled; see
[Verifying a download](#verifying-a-download).

## Windows

1. Download and run one of:
   - Installer: `srelens_<version>_x64-setup.exe` (recommended)
   - MSI: `srelens_<version>_x64_en-US.msi`

> **SmartScreen:** Windows code signing is on the
> [roadmap](https://github.com/orgs/srelens/projects/1), so Windows may show a
> "Windows protected your PC" prompt. Click **More info → Run anyway** to
> proceed. Signed installers will remove this step in a future release.

## Verifying a download

> **From the next release onward.** Release signing is configured, and stable
> releases published from here on carry `.asc` signatures. Releases made before
> that — including the current latest — have none, so there is nothing to verify
> on those yet.

Once signing is live, every installer on a **stable** release is published with a
detached GPG signature alongside it — `srelens_1.2.3_amd64.deb` has
`srelens_1.2.3_amd64.deb.asc`. A stable release is only made public after
signing succeeds, so a missing `.asc` there means you are not looking at a
finished release.

Dev-channel pre-releases are published as soon as they build, before signing
runs, so their signatures are best-effort: usually present, but absent if
signing failed for that build. Treat a dev pre-release without an `.asc` as
unverified rather than as evidence of tampering.

**1. Import the signing key from this repository.**

```bash
curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/KEYS | gpg --import
```

Prefer this over a keyserver search. Anyone can upload a key to a keyserver
under any name or address, so a search for "srelens" can return a key that has
nothing to do with this project.

**2. Check the fingerprint before trusting anything it signs.**

```bash
gpg --fingerprint releases@srelens.com
```

It must match, exactly:

```
6CFC 3480 3A21 C0E6 DB18  BA47 DDEE DBFF 499D 9481
```

This comparison is the step that matters. A `Good signature` line only proves
the file matches whichever key you imported — it says nothing about whether that
key is ours. Without checking the fingerprint, a substituted key verifies just
as cleanly as the real one.

**3. Verify the asset.**

```bash
gpg --verify srelens_1.2.3_amd64.deb.asc srelens_1.2.3_amd64.deb
```

`Good signature from "srelens release signing"`, **with the fingerprint above
confirmed**, means the file is authentic.

A `This key is not certified with a trusted signature` warning alongside it is
expected and is not a failure: it only means you have not personally signed our
key in your own web of trust. The fingerprint check in step 2 is what replaces
that certification.

`BAD signature`, or a key that cannot be found, means the file should not be
run — download it again from the
[releases page](https://github.com/srelens/srelens/releases).

Two kinds of file on a release are updater plumbing rather than downloads, and
are not covered by the above:

- `.sig` files are Tauri updater signatures, used by the in-app updater to check
  its own downloads. They are not GPG signatures, so `gpg --verify` does not
  apply to them.
- `latest.json` is the updater manifest. It is **not** itself signed: it lists
  each installer's URL along with that installer's `.sig`, and the updater
  checks the installer it downloads against a public key built into the app.
  A tampered manifest therefore cannot make the app accept a modified build,
  because the attacker cannot produce a signature valid under that pinned key.
  The manifest carries no GPG signature on the dev channel; on stable releases
  it happens to be signed along with everything else on the tag.

## Connect MCP clients

srelens can act as an MCP server so agents and MCP-enabled editors drive your
clusters. Open **Settings → MCP** to:

- **Run the MCP server (HTTP)** on a loopback port — it shares your authenticated clusters.
- **Install the `srelens` CLI** to `~/.local/bin` so clients can spawn `srelens --mcp-stdio`
  (ensure `~/.local/bin` is on your `PATH`).
- **Copy client config** for Claude Code, Claude Desktop, Cursor, Codex, Antigravity, and others.

The HTTP server requires a bearer token, shown (and rotatable) from that same
Settings page. Destructive tools prompt for confirmation in the app — an
agent can't delete or drain anything without your approval. Headless CLI use
needs `"_confirm": true` on the call plus a process-level opt-in:
`--mcp-allow-destructive` to change anything, or `--mcp-allow-sensitive-reads`
to read Secrets. See [MCP.md](MCP.md) for the full security model.

## Updating

srelens checks for updates from **Settings → Updates**:

- **Stable** — released versions (default).
- **Dev** — rolling pre-releases for early access.

Updates are cryptographically signed and verified before install. On Linux, the
in-app updater applies to the AppImage build only.

## Uninstalling

- **macOS** — drag **srelens** from Applications to the Trash.
- **Linux** — `sudo apt remove srelens` / `sudo dnf remove srelens`, or delete
  the AppImage.
- **Windows** — uninstall via **Settings → Apps**, or the MSI/installer entry.

Application data lives in your OS config directory; remove it manually if you
want a clean uninstall.
