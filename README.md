<p align="center">
  <img src="docs/assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">The Kubernetes control room—built in Rust, ready for engineers and AI agents.</h3>

<p align="center">
  srelens is an open-source, local-first Kubernetes desktop workspace for SREs,
  platform engineers, and DevOps engineers. Investigate, analyse, and take safe
  action across clusters from one application built with Tauri v2, React 19,
  and a pure-Rust core.
</p>

<p align="center">
  <a href="https://srelens.com">Website</a> ·
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="docs/USAGE.md">User guide</a> ·
  <a href="#mcp-server">MCP server</a> ·
  <a href="docs/DEVELOPMENT.md">Developer guide</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/srelens/srelens/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/srelens/srelens?display_name=release&label=release&color=22c55e"></a>
  <a href="https://github.com/srelens/srelens/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/srelens/srelens/total?label=downloads&color=3b82f6"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/github/license/srelens/srelens?color=675e80"></a>
  <a href="https://www.reddit.com/r/srelens/"><img alt="Reddit: r/srelens" src="https://img.shields.io/badge/reddit-r%2Fsrelens-FF4500?logo=reddit&logoColor=white"></a>
</p>

<p align="center">
  <img alt="Rust core" src="https://img.shields.io/badge/core-Rust-8b5cf6">
  <img alt="Tauri v2" src="https://img.shields.io/badge/desktop-Tauri_v2-e457c2">
  <img alt="MCP server" src="https://img.shields.io/badge/agents-MCP-fb923c">
  <img alt="Project status: beta" src="https://img.shields.io/badge/status-beta-f59e0b">
</p>

---

## Why srelens?

Kubernetes troubleshooting often means moving between terminals, dashboards, YAML
editors, logs, and cluster contexts. srelens brings that investigation loop into
one local-first desktop workspace.

- **One workspace from investigation to action** — browse resources, inspect events
  and YAML, follow logs, use terminals, manage port forwards, and take cluster
  actions without constantly switching tools.
- **Built for engineers and AI agents** — supported backend capabilities are also
  available through the built-in MCP server.
- **Local-first cluster access** — srelens uses credentials from your local
  kubeconfig and connects directly to Kubernetes API servers, without routing
  cluster access through a srelens cloud service.
- **Safe operations** — destructive actions are identified and confirmation-gated.
- **Open source** — licensed under MIT, with public code, releases, issues, and
  roadmap on GitHub.

srelens uses the operating system WebView through Tauri v2 and a Rust backend built
with `kube-rs` and `tokio`. It is independently developed and is not affiliated with
Mirantis Lens or the Freelens project.

See the [user guide](docs/USAGE.md) for how to use each of these.

- **Multi-cluster workspace** — discover kubeconfig contexts, add or paste more
  files, give each context a name, logo, and colour, and switch clusters from the
  cluster hotbar. Contexts that share a name across files (e.g. `default`) are
  disambiguated so every cluster stays visible and reachable.
- **Live Kubernetes resources** — browse workloads, networking, storage, RBAC,
  admission, autoscaling, and custom resources with live watch updates, search,
  column pickers, namespace scoping, and bulk actions.
- **Resource details and YAML** — inspect manifests, events, relationships, and
  metrics, and edit schema-aware YAML with validation, dry-run diffs, and
  server-side apply.
- **Logs** — stream pod or workload logs with previous-instance (post-crash)
  logs, timestamps, tail and since-window controls, per-source colouring,
  container filtering, and buffer or all-container export.
- **Terminals and shells** — open pod exec sessions, a context-scoped local
  terminal, ephemeral debug containers for distroless pods, and privileged node
  shells.
- **Port forwarding** — create, inspect, copy, and stop forwards across every open
  cluster.
- **Helm** — list and inspect releases, and install, upgrade, roll back, or
  uninstall them with a values editor and rendered-diff preview.
- **Toolbox** — install and manage `kubectl`, `krew`, `helm`, and krew plugins, and
  diagnose a context's exec-auth tool requirements.
- **Metrics** — node and pod CPU and memory when `metrics-server` is available.
- **Operational actions** — scale workloads, restart rollouts, evict or delete pods,
  suspend or trigger CronJobs, and cordon or drain nodes, with confirmation gates
  for destructive actions.
- **Command palette** — keyboard-first navigation (Cmd/Ctrl-K) across views,
  contexts, and resources.
- **Application logs** — read srelens's own rotating log file from Settings to
  diagnose issues after they happen.
- **MCP access** — expose supported backend capabilities to MCP-capable clients over
  stdio or loopback HTTP.

## Install

Download the latest beta for your platform from
[GitHub Releases](https://github.com/srelens/srelens/releases/latest).

| Platform | Packages | Notes |
| --- | --- | --- |
| macOS | `.dmg` for Apple Silicon and Intel | Developer ID signed and notarized |
| Linux | `.AppImage`, `.deb`, `.rpm` | AppImage supports the in-app updater |
| Windows | `.exe`, `.msi` | Windows may show a SmartScreen prompt while code signing remains on the roadmap |

See the [installation guide](docs/INSTALL.md) for platform-specific installation,
first-launch, updating, verification, and uninstall instructions.

**Run as a web app (Docker):** srelens can also run as a multi-user web server
in a container. Users sign in with OIDC (or a local dev login for trials) and
each gets a fully isolated environment built only from their own uploaded
kubeconfigs. OIDC-protected clusters work with a browser-based, Headlamp-style
sign-in — srelens runs the authorization-code + PKCE flow and injects the
id_token itself, so no `kubelogin`/exec plugin is needed. Kubeconfigs and tokens
are sealed at rest under a required `SRELENS_MASTER_KEY` that is never written to
disk; decrypted files live only in tmpfs. Some desktop-only actions (host shell,
raw helm repo/plugin) are gated off the shared surface — web users get
RBAC-scoped in-pod exec terminals instead. See [docs/WEB.md](docs/WEB.md) for
deployment, environment variables, and the full security model.

## MCP server

srelens includes an MCP server generated from the same capability registry used by
the desktop backend. Supported backend capabilities can therefore be used by
MCP-capable clients without creating a separate cluster integration layer.

Open **Settings → MCP** to:

- run the MCP server over loopback HTTP, protected by a bearer token you can
  reveal, rotate, or revoke — rotating restarts the running server so the new
  token takes effect at once (dropping any in-flight request and invalidating
  configs that used the old value); revoking also stops the server;
- install the `srelens` CLI for stdio connections, which need no token — the
  client already holds your privileges by spawning the process;
- copy client configuration for supported MCP clients.

You can also start the server directly:

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
```

Destructive tools prompt for confirmation in the app. Headless runs have no
dialog to show, so they need `"_confirm": true` on the call *and* a
process-level opt-in — `--mcp-allow-destructive` to change anything, or
`--mcp-allow-sensitive-reads` to read Secrets. The two are independent, so
reading a Secret never implies permission to drain a node, and neither flag
alone authorizes anything without `_confirm`. There's no GUI toggle for
stdio. See [docs/MCP.md](docs/MCP.md) for the full security model, including
the Host-header check, audit log, and token storage.

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

> MCP access uses your locally authenticated cluster contexts. Review tool calls
> and use appropriate Kubernetes RBAC permissions, especially with critical
> clusters.

## Quick start

### Prerequisites

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 22+
- [pnpm](https://pnpm.io) 9+
- [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
- A reachable Kubernetes cluster for cluster-dependent workflows

### Run locally

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev
```

### Useful commands

| Command | Purpose |
| --- | --- |
| `pnpm dev` | Launch the desktop application in development mode |
| `pnpm test` | Run JavaScript and TypeScript tests |
| `cargo test` | Run Rust workspace tests |
| `pnpm build` | Build the production frontend |
| `pnpm tauri build` | Create packaged desktop binaries |

See the [developer guide](docs/DEVELOPMENT.md) for architecture, testing standards,
and instructions for adding capabilities.

## Architecture

```text
React 19 + TypeScript
        │
        │ Tauri commands and events
        ▼
Tauri v2 desktop shell
        │
        ▼
Pure-Rust backend
├── capability registry
├── Kubernetes integration with kube-rs
├── live watches, logs, exec, and port forwarding
├── Helm and metrics
└── MCP server over stdio and loopback HTTP
```

Repository layout:

```text
apps/desktop/
  src/                   React and TypeScript desktop UI
  src-tauri/             Tauri application and Rust command bridge
crates/
  capability/            Backend capability registry
  kube/                  Kubernetes clients, watches, actions, Helm, and metrics
  mcp/                   MCP server
docs/                    Installation, usage, development, and project documentation
```

## Project status

srelens is currently in **beta**. It is ready for evaluation and everyday testing,
but users should review release notes and take extra care when using it with
critical clusters.

Breaking changes may still occur before a stable release. Feedback, bug reports,
and reproducible troubleshooting details are welcome.

- [Latest release](https://github.com/srelens/srelens/releases/latest)
- [All releases](https://github.com/srelens/srelens/releases)
- [Issues and roadmap](https://github.com/srelens/srelens/issues)

## Community

- [r/srelens on Reddit](https://www.reddit.com/r/srelens/) — announcements,
  questions, and feedback
- [Issues](https://github.com/srelens/srelens/issues) — bugs and feature requests
- [Website](https://srelens.com)

## Contributing

Contributions are welcome. Start with:

- [Contribution guide](CONTRIBUTING.md)
- [Developer guide](docs/DEVELOPMENT.md)
- [MCP agent integration guide](docs/MCP.md)
- [Open issues](https://github.com/srelens/srelens/issues)

Please review the [Code of Conduct](.github/CODE_OF_CONDUCT.md) before
participating.

## License

srelens is open source under the [MIT License](LICENSE).

---

<p align="center">
  <sub>Not affiliated with Mirantis Lens or the Freelens project.</sub>
</p>
