<p align="center">
  <img src="docs/assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">The Kubernetes control room—built in Rust, ready for engineers and AI agents.</h3>

<p align="center">
  <b>English</b> ·
  <a href="docs/i18n/README.zh-CN.md">简体中文</a> ·
  <a href="docs/i18n/README.ja.md">日本語</a> ·
  <a href="docs/i18n/README.ko.md">한국어</a> ·
  <a href="docs/i18n/README.es.md">Español</a> ·
  <a href="docs/i18n/README.pt-BR.md">Português</a> ·
  <a href="docs/i18n/README.de.md">Deutsch</a> ·
  <a href="docs/i18n/README.fr.md">Français</a>
</p>

<p align="center">
  srelens is an open-source, local-first Kubernetes workspace for SREs, platform
  engineers and DevOps engineers. Investigate, analyse and take safe action across
  clusters — from a <b>desktop app</b>, from a <b>terminal UI</b>, or through an
  <b>MCP server</b> your AI agent can call. One pure-Rust core drives all three.
</p>

<p align="center">
  <a href="https://srelens.com">Website</a> ·
  <a href="#install">Install</a> ·
  <a href="#desktop-app">Desktop app</a> ·
  <a href="#terminal-ui-srectl">Terminal UI</a> ·
  <a href="#mcp-server">MCP server</a> ·
  <a href="docs/USAGE.md">User guide</a> ·
  <a href="docs/DEVELOPMENT.md">Developer guide</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/srelens/srelens/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/srelens/srelens?logo=github&color=eab308"></a>
  <a href="https://www.reddit.com/r/srelens/"><img alt="Reddit: r/srelens" src="https://img.shields.io/reddit/subreddit-subscribers/srelens?label=r%2Fsrelens&logo=reddit&logoColor=white&color=FF4500"></a>
  <a href="https://deepwiki.com/srelens/srelens"><img alt="Ask DeepWiki" src="https://img.shields.io/badge/Ask-DeepWiki-2563eb"></a>
</p>

<p align="center">
  <a href="https://github.com/srelens/srelens/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/srelens/srelens?display_name=release&label=release&color=22c55e"></a>
  <a href="https://github.com/srelens/srelens/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/srelens/srelens/total?label=downloads&color=3b82f6"></a>
  <a href="https://aur.archlinux.org/packages/srelens-bin"><img alt="AUR version" src="https://img.shields.io/aur/version/srelens-bin?label=aur&logo=archlinux&logoColor=white&color=1793d1"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/github/license/srelens/srelens?color=675e80"></a>
  <a href="https://github.com/srelens/srelens/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/srelens/srelens/actions/workflows/ci.yml/badge.svg?branch=dev"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/srelens/srelens"><img alt="OpenSSF Scorecard" src="https://api.scorecard.dev/projects/github.com/srelens/srelens/badge"></a>
</p>

<p align="center">
  <img alt="Rust core" src="https://img.shields.io/badge/core-Rust-8b5cf6">
  <img alt="Tauri v2" src="https://img.shields.io/badge/desktop-Tauri_v2-e457c2">
  <img alt="Terminal UI" src="https://img.shields.io/badge/terminal-srectl-14b8a6">
  <img alt="MCP server" src="https://img.shields.io/badge/agents-MCP-fb923c">
  <img alt="Project status: stable" src="https://img.shields.io/badge/status-stable-22c55e">
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/screenshots/gui-overview-light.webp">
    <img alt="The srelens desktop app in its new design: a cluster overview with node capacity, workloads that are not ready, and objects by kind" src="docs/assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## Pick your surface

|  | Desktop app (GUI) | Terminal UI (`srectl`) |
| --- | --- | --- |
| **What it is** | A native window built with Tauri v2 and React 19 | One self-contained binary built with ratatui |
| **Best for** | Daily cluster work, YAML editing, topology, many clusters side by side | SSH sessions, jump hosts, keyboard-only flow, k9s muscle memory |
| **Runs on** | macOS, Linux, Windows — or in a browser via [web mode](docs/WEB.md) | Linux (glibc and static musl) and macOS on x86-64 and arm64; Windows on x86-64 |
| **Install** | [Download a release](#install) | `brew install srelens/tap/srectl` or [one line](#install) |
| **AI** | Built-in assistant, plus the MCP server in **Settings → MCP** | `:ai` assistant, plus `srectl mcp` over stdio |

Both talk to your clusters directly with the credentials in your local
kubeconfig. Nothing is routed through a srelens cloud service.

## Why srelens?

Kubernetes troubleshooting usually means hopping between terminals, dashboards,
YAML editors, logs and cluster contexts. srelens brings that investigation loop
into one local-first workspace.

- **One workspace from investigation to action** — browse resources, read events
  and YAML, follow logs, open shells, forward ports and take cluster actions
  without switching tools.
- **Built for engineers and AI agents** — the backend capabilities you use in the
  app are the same ones exposed through the built-in MCP server.
- **Local-first cluster access** — srelens connects straight to your API servers
  with your own kubeconfig credentials.
- **Safe operations** — destructive actions are identified and confirmation-gated,
  in the app, in the terminal and over MCP.
- **Small and fast** — a Rust core on the operating system's WebView. srelens
  `v0.5.0` shipped a **17.1 MiB** macOS installer and a **19.8 MiB** Linux `.deb`,
  against 190.2 MiB and 146.8 MiB for Freelens `v1.10.3` — roughly **11× and 7×
  smaller**. The [performance baselines](docs/PERFORMANCE.md) show how to
  reproduce that, including where the gap is narrower.
- **Open source** — MIT licensed, with public code, releases, issues and roadmap.

## Desktop app

The desktop app is a full Kubernetes workspace in a native window. The
screenshots below are the **new design**, which is being rolled out screen by
screen: turn it on in **Settings → Appearance → Design → New design**, and switch
back the same way at any time. It has six themes: Light, Paper, Dark, Midnight,
Glass and High contrast.

<table>
  <tr>
    <td width="50%"><img alt="Pods list filtered to one namespace, with a crash-looping pod marked in red" src="docs/assets/screenshots/gui-pods.webp"><br><sub><b>Live resources</b> — watch-driven tables with filters, columns and bulk actions</sub></td>
    <td width="50%"><img alt="Pod detail view with readiness, restarts, CPU, memory, and quick actions for logs, shell and port forward" src="docs/assets/screenshots/gui-pod-detail.webp"><br><sub><b>Resource details</b> — status, events, owners, and one-click logs, shell and forward</sub></td>
  </tr>
  <tr>
    <td><img alt="Log stream for a failing pod, showing a fatal connection-refused line flagged in red" src="docs/assets/screenshots/gui-logs.webp"><br><sub><b>Logs</b> — multi-pod streams, previous instance, filters, export and AI summaries</sub></td>
    <td><img alt="Schema-aware YAML editor for a Deployment with validation, diff, dry run and apply" src="docs/assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b> — schema-aware editing, server dry run, diff and server-side apply</sub></td>
  </tr>
  <tr>
    <td><img alt="Service topology graph showing a failing deployment and the external database it calls" src="docs/assets/screenshots/gui-topology.webp"><br><sub><b>Topology</b> — who calls whom, and where the failure starts</sub></td>
    <td><img alt="Helm releases with a rendered diff between two revisions" src="docs/assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b> — install, upgrade, roll back and uninstall with a rendered diff</sub></td>
  </tr>
</table>

What you can do in it — the [user guide](docs/USAGE.md) covers each in depth:

- **Multi-cluster workspace** — discover kubeconfig contexts, add or paste more
  files, give each context a name, logo and colour, and switch clusters from the
  cluster rail. Contexts that share a name across files stay distinct.
- **Live Kubernetes resources** — workloads, networking, storage, RBAC, admission,
  autoscaling and custom resources, with live watches, search, column pickers,
  namespace scoping and bulk actions.
- **Resource details and YAML** — manifests, events, relationships and metrics;
  schema-aware YAML with validation, dry-run diffs and server-side apply.
- **Logs** — pod or workload streams with previous-instance logs, timestamps,
  tail and since-window controls, per-source colours, container filters and export.
- **Terminals and shells** — pod exec, a context-scoped local terminal, ephemeral
  debug containers for distroless pods, and privileged node shells.
- **Port forwarding** — create, inspect, copy and stop forwards across every open
  cluster.
- **Helm** — inspect releases, and install, upgrade, roll back or uninstall them
  with a values editor and rendered-diff preview.
- **Operational actions** — scale, restart rollouts, evict or delete pods, suspend
  or trigger CronJobs, cordon and drain nodes, all behind confirmation gates.
- **Toolbox** — install and manage `kubectl`, `krew`, `helm` and krew plugins, and
  diagnose a context's exec-auth requirements.
- **Metrics** — node and pod CPU and memory when `metrics-server` is available.
- **AI assistant** — ask about the cluster in front of you, summarise a log
  stream, or explain a Helm diff.
- **Command palette** — keyboard-first navigation (Cmd/Ctrl-K) across views,
  contexts and resources.
- **Web mode** — run the same app as a multi-user web server in Docker, with OIDC
  sign-in and per-user isolation. See [docs/WEB.md](docs/WEB.md).

## Terminal UI (`srectl`)

`srectl` is srelens in your terminal: one binary, k9s-style navigation, and the
same Rust core as the desktop app. It is at home over SSH, on a jump host, or
anywhere a window is not.

<p align="center">
  <img alt="srectl showing a live Pods table with namespace, readiness, status, restarts, IP and node columns" src="docs/assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="Resource relationship tree from Deployment to ReplicaSet to Pod, with its container, ConfigMap and Service" src="docs/assets/screenshots/tui-tree.webp"><br><sub><b>Relationship tree</b> — owners, children, config and services in one view</sub></td>
    <td width="50%"><img alt="Live log stream for a pod with line numbers and follow, timestamp, wrap and save controls" src="docs/assets/screenshots/tui-logs.webp"><br><sub><b>Logs</b> — follow, search, timestamps, previous instance and save</sub></td>
  </tr>
  <tr>
    <td><img alt="Helm release overview with tabs for values diff, revisions, manifest and notes" src="docs/assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b> — revisions, values diff, manifest and rollback</sub></td>
    <td><img alt="GPU inspector showing GPU and VRAM allocation per node and the pods requesting GPUs" src="docs/assets/screenshots/tui-gpu.webp"><br><sub><b>GPU inspector</b> — GPU and VRAM allocation per node and per pod</sub></td>
  </tr>
  <tr>
    <td><img alt="Argo CD applications with sync and health status across hub and spoke clusters" src="docs/assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b> — sync, health and drift across hub and spoke clusters</sub></td>
    <td><img alt="AI assistant answering which nodes have a T4 GPU after calling the listNodes tool" src="docs/assets/screenshots/tui-assistant.webp"><br><sub><b>AI assistant</b> — asks the cluster through tools, with SRE playbooks</sub></td>
  </tr>
</table>

- **k9s-style navigation** — `:` opens the command prompt (`:pod`, `:deploy`,
  `:svc`, `:no`, `:ns`, `:helm`, `:pf`, `:ctx` …), `/` filters, `j`/`k` move,
  `Enter` drills down and `Esc` goes back.
- **Act on what you see** — logs (`l`), shell (`s`), describe (`d`), YAML (`y`),
  edit (`e`), port forward (`f`), delete (`Ctrl-d`) and a quick-actions palette
  (`x`) for restart, scale and more.
- **Investigate** — relationship tree (`t`), `:top` hotspots by CPU and memory,
  `:topo` topology, a cluster overview, and node SSH for recovery.
- **Beyond the basics** — Argo CD (`:argo`), GPU and VRAM (`:gpuinfo`), and BGP
  peering (`:bgp`) views.
- **AI assistant** — `:ai` with Anthropic (Claude), OpenAI, Google Gemini, any
  OpenAI-compatible endpoint such as Ollama, or Cursor Agent.
- **Headless helpers** — `srectl info` lists the contexts in your kubeconfig,
  `srectl toolbox` checks for `kubectl`, `helm` and `krew`, and `srectl mcp` runs
  the MCP server over stdio.
- **Signed self-update** — `srectl update` installs a release only when its
  checksums are signed by a srelens release key built into the binary.

```bash
srectl                      # start on the current context
srectl -n payments          # start on the Pods view, scoped to a namespace
srectl --context prod -A    # pick a context, all namespaces
srectl pods/my-pod          # open straight onto a resource
srectl --help               # every flag
```

## Install

Every release is on [GitHub Releases](https://github.com/srelens/srelens/releases/latest).
The [installation guide](docs/INSTALL.md) covers first launch, updating,
verifying downloads and uninstalling on each platform.

### Desktop app

| Platform | Packages | Notes |
| --- | --- | --- |
| macOS | `.dmg` for Apple Silicon and Intel | Developer ID signed and notarized |
| Linux | `.AppImage`, `.deb`, `.rpm`, and [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) on the AUR | The AppImage supports the in-app updater |
| Windows | `.exe`, `.msi` | Windows may show a SmartScreen prompt while code signing remains on the roadmap |

### Terminal UI

macOS (Homebrew):

```bash
brew install srelens/tap/srectl
```

Linux, in one line:

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

Or download `srectl-<version>-<target>.tar.gz` (or `.zip` on Windows) from the
release, for Linux (glibc and static musl) and macOS on x86-64 and arm64, and
for Windows on x86-64. macOS builds on a stable release are signed and notarized.

### Web app (Docker)

srelens also runs as a multi-user web server in a container. Users sign in with
OIDC (or a local dev login for trials) and each gets an isolated environment
built only from their own uploaded kubeconfigs. OIDC-protected clusters sign in
from the browser, with no `kubelogin` or exec plugin needed, and kubeconfigs and
tokens are sealed at rest under a required `SRELENS_MASTER_KEY`. See
[docs/WEB.md](docs/WEB.md) for deployment and the full security model.

## MCP server

srelens includes an MCP server generated from the same capability registry as
the backend, so MCP-capable clients get the same cluster operations without a
separate integration layer.

In the desktop app, open **Settings → MCP** to:

- run the server over loopback HTTP, protected by a bearer token you can reveal,
  rotate or revoke — rotating restarts the running server so the new token takes
  effect at once, and revoking stops it;
- install the `srelens` CLI for stdio connections, which need no token, because
  the client already holds your privileges by spawning the process;
- copy client configuration for supported MCP clients.

Or start it directly — from the desktop binary, or from the terminal UI:

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

Example stdio configuration:

```json
{
  "mcpServers": {
    "srelens": {
      "command": "srelens",
      "args": ["--mcp-stdio"]
    }
  }
}
```

Destructive tools prompt for confirmation in the app. Headless runs have no
dialog to show, so they need `"_confirm": true` on the call *and* a
process-level opt-in: `--mcp-allow-destructive` to change anything, or
`--mcp-allow-sensitive-reads` to read Secrets (`--allow-destructive` and
`--allow-sensitive-reads` on `srectl mcp`). The two are independent, so reading
a Secret never implies permission to drain a node. See [docs/MCP.md](docs/MCP.md)
for the full security model, including the Host-header check, audit log and
token storage.

> MCP access uses your locally authenticated cluster contexts. Review tool calls
> and use appropriate Kubernetes RBAC permissions, especially with critical
> clusters.

## Build from source

### Prerequisites

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 26 (development default; Node 24 LTS is also supported). Run `nvm install && nvm use` to use the version in `.nvmrc`.
- [pnpm](https://pnpm.io) 9+
- [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
- A reachable Kubernetes cluster for cluster-dependent workflows

### Run locally

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### Useful commands

| Command | Purpose |
| --- | --- |
| `pnpm dev` | Launch the desktop application in development mode |
| `cargo run -p srectl` | Run the terminal UI |
| `pnpm test` | Run JavaScript and TypeScript tests |
| `cargo test` | Run Rust workspace tests |
| `pnpm build` | Build the production frontend |
| `pnpm tauri build` | Create packaged desktop binaries |

See the [developer guide](docs/DEVELOPMENT.md) for architecture, testing standards
and how to add a capability.

## Architecture

```text
 Desktop app (React 19 + Tauri v2)   Terminal UI (srectl)   MCP clients   Web app
                 │                           │                  │            │
                 └───────────────┬───────────┴──────────────────┴────────────┘
                                 ▼
                         Pure-Rust core
                         ├── capability registry
                         ├── Kubernetes integration with kube-rs
                         ├── live watches, logs, exec and port forwarding
                         ├── Helm and metrics
                         └── MCP server over stdio and loopback HTTP
```

Repository layout:

```text
apps/
  desktop/               Tauri desktop app: React UI (src/) and Rust bridge (src-tauri/)
  tui/                   srectl, the terminal UI
packages/
  core/                  Shared frontend service layer
  ui-kit/                Design-system components
  ui-next/               The new desktop design
crates/
  capability/            Backend capability registry
  kube/                  Kubernetes clients, watches, actions, Helm and metrics
  mcp/                   MCP server
  registry/              Where every backend capability is registered
  server/                Web mode server
  streams/               Streaming cores shared by desktop and web
  agent/, llm/           AI assistant and agent-CLI bridge
  plugin-host/           Extension host
docs/                    Installation, usage, development and project documentation
```

## Project status

srelens is **stable and in active development**, with regular releases for
macOS, Linux and Windows. macOS builds are Developer ID signed and notarized,
and supported packages update in place. New capabilities land continuously, so
the release notes are worth a glance when you upgrade.

- [Latest release](https://github.com/srelens/srelens/releases/latest)
- [All releases](https://github.com/srelens/srelens/releases)
- [Issues and roadmap](https://github.com/srelens/srelens/issues)

## Community

- [r/srelens on Reddit](https://www.reddit.com/r/srelens/) — announcements,
  questions and feedback
- [Issues](https://github.com/srelens/srelens/issues) — bugs and feature requests
- [Website](https://srelens.com)

## Contributing

Contributions are welcome. Start with the [contribution guide](CONTRIBUTING.md),
the [developer guide](docs/DEVELOPMENT.md), the
[MCP agent integration guide](docs/MCP.md) and the
[open issues](https://github.com/srelens/srelens/issues). Please review the
[Code of Conduct](.github/CODE_OF_CONDUCT.md) before participating.

Translations live in [`docs/i18n`](docs/i18n). The English README is the
source of truth; a fix to a translation, or a new language, is welcome as a
pull request.

## License

srelens is open source under the [MIT License](LICENSE).

---

<p align="center">
  <sub>Not affiliated with Mirantis Lens or the Freelens project.</sub>
</p>
