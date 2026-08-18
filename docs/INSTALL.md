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

> **srelens 0.6.0 and earlier are unsigned.** Release signing begins with
> 0.6.1; there are no `.asc` files on releases before it, so there is nothing to
> verify on those.

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

**2. Find the fingerprint for the release you are verifying.**

Signing keys are rotated over time, and `KEYS` keeps the retired ones so older
releases stay verifiable. Pick the row covering your download — a release is
signed by whichever key was current when it was published, not by the newest
one.

| Releases | Fingerprint |
| --- | --- |
| 0.6.1 and later — **current** | `6CFC3480 3A21C0E6 DB18BA47 DDEEDBFF 499D9481` |

**3. Verify the asset, binding the result to that fingerprint.**

```bash
EXPECTED=6CFC34803A21C0E6DB18BA47DDEEDBFF499D9481   # from the table above

STATUS=$(gpg --verify --status-fd 1 \
  srelens_1.2.3_amd64.deb.asc srelens_1.2.3_amd64.deb 2>/dev/null)

grep -q "^\[GNUPG:\] VALIDSIG $EXPECTED " <<<"$STATUS" \
  && ! grep -q '^\[GNUPG:\] REVKEYSIG' <<<"$STATUS" \
  && echo "AUTHENTIC" \
  || echo "REJECT - do not run this file"
```

Signing keys expire on a schedule, so verifying an **older** release will
eventually report `EXPKEYSIG` — GnuPG says that whenever the key is expired
*now*, even though the signature was made while it was valid. That is expected
for an archived release and the check above deliberately tolerates it: expiry
is a rotation schedule, not evidence of anything wrong. A revoked key is a
different matter and is always rejected, because revocation is how a lost or
compromised key is announced.

If you see `EXPKEYSIG` on a **current** release, treat it as suspicious and
report it — our release job refuses to sign with an expired key, so that
combination should not occur.

Do not substitute `gpg --fingerprint releases@srelens.com` for this. That looks
up keys by address, and **anyone can create a key carrying our exact name and
address** — if such a key is in your keyring, that command lists it too and
`gpg --verify` will happily print `Good signature from "srelens release
signing"` for it. The check above avoids the problem entirely: `VALIDSIG`
reports the full fingerprint of the key that *actually made this signature*, so
matching it leaves no room for a look-alike key to be mistaken for ours.

`AUTHENTIC` means the signature was made by our key and the key was neither
revoked nor expired when checked.

If you would rather read the output yourself, `gpg --verify
srelens_1.2.3_amd64.deb.asc srelens_1.2.3_amd64.deb` prints it in human form —
but judge it by the rules below, not by the `Good signature` line alone.

One warning is expected and harmless:

- `This key is not certified with a trusted signature` — it only means you have
  not personally signed our key in your own web of trust. The fingerprint check
  in step 2 is what replaces that certification.

This one means stop, even though `gpg` prints `Good signature` and exits
successfully:

- `This key has been revoked` — the key was retired, possibly because it was
  lost or compromised. A signature made with a revoked key proves only that
  *someone* holding it signed the file, which may not be us.

And this one depends on which release you are verifying:

- `This key has expired` — expected on an older release whose signing key has
  since been rotated out, harmless there. On a current release it should not
  happen; treat it as suspicious.

`gpg --verify` reports whether the signature is cryptographically intact, not
whether the key is still fit to trust, so its exit status alone is not the
answer. Read the warnings.

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
