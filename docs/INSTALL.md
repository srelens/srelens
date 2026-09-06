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
automatically. See [Updating](#updating) for how each format updates itself.

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

## Terminal UI (`srelens-tui`)

The terminal UI ships as one self-contained binary, separate from the desktop
app and with nothing to install. Every release carries an archive per platform:

| Platform | Asset |
| --- | --- |
| Linux x86-64 | `srelens-tui-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86-64, static | `srelens-tui-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 | `srelens-tui-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| Linux arm64, static | `srelens-tui-<version>-aarch64-unknown-linux-musl.tar.gz` |
| macOS Apple Silicon | `srelens-tui-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `srelens-tui-<version>-x86_64-apple-darwin.tar.gz` |
| Windows x86-64 | `srelens-tui-<version>-x86_64-pc-windows-msvc.zip` |

Take the **musl** build if your distribution is Alpine, or if the glibc build
reports a version error — it is statically linked and depends on nothing on the
host. Otherwise prefer the glibc build.

**Linux and macOS.** Extract and put it on your `PATH`:

```bash
tar -xzf srelens-tui-<version>-<target>.tar.gz
chmod +x srelens-tui
sudo mv srelens-tui /usr/local/bin/
srelens-tui --version
```

**Windows.** Extract the `.zip` and move `srelens-tui.exe` somewhere on your
`PATH`, then run `srelens-tui --version` in a terminal. Windows may warn that
the file came from the internet, for the same reason the desktop installer
does: code signing is on the roadmap ([#32]).

The macOS builds on a **stable** release are signed with the same Apple
Developer ID as the desktop app and notarized with the same account, so
Gatekeeper admits them — a stable release cannot be cut without them, the same
rule that governs the GPG signatures below. Dev-channel pre-releases are built
even when those credentials are unavailable, so treat an unsigned macOS binary
there as a pre-release that skipped signing rather than as evidence of
tampering. The
notarization ticket is **not stapled** — Apple only staples to `.app`, `.dmg`
and `.pkg`, and this ships as a tarball — so the first run of a quarantined
copy is checked against Apple online. If that first run happens offline and
macOS refuses the binary, either reconnect and try again or clear the
quarantine flag yourself:

```bash
xattr -d com.apple.quarantine ./srelens-tui
```

Most people never see this at all: extracting a `.tar.gz` with `tar` in a
terminal does not mark the contents as quarantined in the first place.

**Checking the download.** Each release lists the SHA-256 of every TUI archive
in `srelens-tui-<version>-SHA256SUMS.txt`. Download it next to the archive and
check the one file you took. The tool differs per platform — `sha256sum` is GNU
coreutils, so it is absent on a stock macOS and on Windows.

Linux:

```bash
sha256sum -c --ignore-missing srelens-tui-<version>-SHA256SUMS.txt
```

macOS (`shasum` ships with the system; `-c -` reads the one line you pass it,
and `--ignore-missing` does not exist here):

```bash
grep "srelens-tui-<version>-<target>.tar.gz$" \
  srelens-tui-<version>-SHA256SUMS.txt | shasum -a 256 -c -
```

Windows (PowerShell):

```powershell
$archive = "srelens-tui-<version>-x86_64-pc-windows-msvc.zip"
$expected = (Select-String -Path "srelens-tui-<version>-SHA256SUMS.txt" -Pattern ([regex]::Escape($archive))).Line.Split(" ")[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLower()
if ($expected -eq $actual) { "OK" } else { "MISMATCH — do not run this file" }
```

That proves the file arrived intact, not who built it. For that, the archives
carry detached GPG signatures like every other release asset — see
[Verifying a download](#verifying-a-download) below, which applies to them
unchanged.

Run `srelens-tui --help` for the full set of flags. `srelens-tui info` lists
the contexts found in your kubeconfig with the cluster and server each names —
it reads the file and does not contact any cluster, so it tells you what is
configured, not what is reachable. `srelens-tui toolbox` reports whether
`kubectl`, `helm` and `krew` are on your `PATH`.

> **Windows: anything that looks for another program on your `PATH` may not
> find it.** Executable lookup is Unix-shaped in several places — it shells out
> to `which`, which Windows does not have, and the in-process fallback matches
> a bare program name without consulting `PATHEXT`, so it never sees
> `kubectl.exe` or `helm.exe`. What that affects: `srelens-tui toolbox` reports
> every tool as missing, Helm operations may report Helm as absent, and the
> Cursor AI provider may not find its binary. Browsing clusters, logs, YAML and
> everything else that talks to the API server is unaffected. Tracked in
> [#445].

[#445]: https://github.com/srelens/srelens/issues/445

[#32]: https://github.com/srelens/srelens/issues/32

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

Updates are cryptographically signed and verified before install.

On Linux what happens next depends on how srelens was installed, and the app
works this out for itself rather than asking you:

| Install | In-app update |
| --- | --- |
| AppImage | Replaces the AppImage in place. Nothing else needed. |
| `.deb` / `.rpm` | Downloads the new package and applies it with `dpkg -i` / `rpm -U`. This needs administrator rights, so your desktop will ask for your password; Settings warns you before the prompt appears. |
| AUR (`srelens-bin`) | Not offered. pacman owns the files, so srelens points you at `paru -Syu` / `yay -Syu` instead of desyncing its database. |

A package update can fail in two different ways, and they need different
answers.

**Before anything is applied** — no `pkexec` or polkit agent, the password
prompt cancelled, a locked package database. Nothing has changed; download the
new `.deb`/`.rpm` from
[Releases](https://github.com/srelens/srelens/releases/latest) and install it
the way you installed the first one.

**Part-way through** — `dpkg`/`rpm` accepted the package and then failed while
unpacking or configuring it (a full disk, an unmet dependency, a failing
maintainer script). The package is left unpacked or unconfigured and srelens may
not start until the package manager finishes the job:

```bash
sudo dpkg --configure -a        # Debian/Ubuntu: finish a half-configured install
sudo apt --fix-broken install   # …and pull in whatever it was missing
sudo dnf reinstall ./srelens-*.rpm   # Fedora/RHEL, from the downloaded file
```

srelens is distributed as a downloaded package rather than from a repository,
so the `dnf` line needs the `.rpm` file itself — plain `dnf reinstall srelens`
has nowhere to fetch it from and fails.

The error `dpkg` or `rpm` printed names the actual cause; fix that first, since
the commands above will otherwise stop at the same point.

## Uninstalling

- **macOS** — drag **srelens** from Applications to the Trash.
- **Linux** — `sudo apt remove srelens` / `sudo dnf remove srelens`, or delete
  the AppImage.
- **Windows** — uninstall via **Settings → Apps**, or the MSI/installer entry.

Application data lives in your OS config directory; remove it manually if you
want a clean uninstall.
