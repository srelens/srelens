# Security Policy

srelens is a local-first Kubernetes IDE that reads your kubeconfigs and talks
to your clusters, so we take security seriously. Thank you for helping keep
users safe.

## Supported versions

Security fixes target the **latest release**. Please upgrade to the newest
version (the in-app updater or the [Releases](https://github.com/srelens/srelens/releases/latest)
page) before reporting.

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately via **GitHub Private Vulnerability Reporting** — on the
repository's **Security** tab, click **Report a vulnerability**. This keeps
the report confidential until a fix is released.

Please include:

- A description of the issue and its impact
- Steps to reproduce (or a proof of concept)
- Affected version and OS
- Any suggested remediation

We aim to acknowledge reports within a few days and to coordinate a fix and
disclosure timeline with you. We'll credit reporters who wish to be named once
a fix ships.

## Scope

In scope: the desktop app and its handling of kubeconfigs and cluster
credentials, exec credential plugin invocation, the cluster connection layer
(TLS, auth), the MCP server surface (transports, consent, destructive-action
gating), the in-app updater, and the build artifacts. Out of scope:
vulnerabilities in Kubernetes itself, in your clusters, or in third-party
credential plugins you configure.

## Security model (for context)

- **No telemetry** — nothing is tracked or transmitted.
- **No account** — there is no srelens backend; the app talks only to the
  clusters in your kubeconfig.
- **Kubeconfigs stay local** — srelens reads your kubeconfig files in place;
  credentials are never copied, stored elsewhere, or logged. Exec credential
  plugins (e.g. `kubectl oidc-login`, cloud CLIs) run locally exactly as
  `kubectl` would run them.
- **MCP is opt-in** — the MCP server runs only when explicitly started
  (`--mcp-stdio` / `--mcp-http`); the HTTP transport binds to loopback only
  unless explicitly exposed with `--mcp-expose-http`, and destructive tools
  require an explicit confirmation argument.
- **Signed builds** — macOS builds are Developer-ID signed and notarized;
  in-app updater artifacts are signed and verified before install. Windows
  code signing and GPG-signed release assets are on the
  [roadmap](https://github.com/orgs/srelens/projects/1).
- **MIT-licensed** — the source is open for review.

## Known dependency advisories

Some GitHub Dependabot alerts reflect **transitive Linux-only dependencies**
that cannot be patched in this repository alone:

- **`glib` (< 0.20)** — pulled in by Tauri's GTK3 / WebKitGTK stack on Linux.
  The unsoundness in `VariantStrIter` is fixed in `glib` 0.20+, but gtk-rs 0.18
  (GTK3) is unmaintained and Tauri has not yet completed its GTK4 migration.
  We track upstream: [tauri#12048](https://github.com/tauri-apps/tauri/issues/12048),
  [wry#1474](https://github.com/tauri-apps/wry/issues/1474). Risk is limited to
  Linux builds; macOS and Windows are unaffected.
