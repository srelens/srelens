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

Each Linux asset ships a Tauri updater signature (`.sig`). GPG signatures for
release assets are on the [roadmap](https://github.com/orgs/srelens/projects/1);
until then, verify by matching the file against the release.

## Windows

1. Download and run one of:
   - Installer: `srelens_<version>_x64-setup.exe` (recommended)
   - MSI: `srelens_<version>_x64_en-US.msi`

> **SmartScreen:** Windows code signing is on the
> [roadmap](https://github.com/orgs/srelens/projects/1), so Windows may show a
> "Windows protected your PC" prompt. Click **More info → Run anyway** to
> proceed. Signed installers will remove this step in a future release.

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
