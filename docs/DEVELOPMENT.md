# Developer guide

This guide covers everything you need to build, test, and extend srelens.

## Prerequisites

- **Rust** (stable) — install via [rustup](https://rustup.rs). The toolchain is pinned by `rust-toolchain.toml` (stable + `llvm-tools-preview` for coverage).
- **Node.js 22+** and **pnpm 9+**.
- **Tauri v2 system dependencies** — see the [official prerequisites](https://v2.tauri.app/start/prerequisites/) (on macOS: Xcode command-line tools; on Linux: webkit2gtk and friends).
- A kubeconfig with at least one reachable cluster ([kind](https://kind.sigs.k8s.io) or k3d works well for development).

## Getting started

```sh
pnpm install       # installs all workspace JS dependencies
pnpm dev           # runs `tauri dev`: Vite dev server + Rust backend with hot reload
```

The first `pnpm dev` compiles the full Rust dependency tree and takes a few minutes; subsequent runs are incremental.

> **Note:** Cargo build caches embed absolute paths. If you move or rename the repository directory, run `cargo clean` before building again.

### Everyday commands

| Command | What it does |
| --- | --- |
| `pnpm dev` | Launch the desktop app in dev mode |
| `pnpm test` | All JS/TS tests via Vitest, with coverage |
| `pnpm --filter @srelens/desktop test:watch` | Vitest in watch mode |
| `cargo test` | All Rust tests across the workspace |
| `cargo llvm-cov --workspace --summary-only` | Rust tests with a coverage report |
| `pnpm build` | Production frontend build (`vite build`) |
| `pnpm tauri build` | Packaged, installable desktop binaries |
| `pnpm tauri icon apps/desktop/src-tauri/icons/icon.svg` | Regenerate the full app icon set from the source SVG |

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│ WebView — React 19 + TS (apps/desktop/src)               │
│   components/  feature views (browser, logs, terminal…)  │
│   lib/         data layer: watches, actions, helm, exec  │
│   transport/   the ONLY module that touches Tauri APIs   │
│   ui/          design-system primitives + styles         │
└───────────────┬──────────────────────────────────────────┘
        Tauri commands + events
┌───────────────▼──────────────────────────────────────────┐
│ Rust backend (apps/desktop/src-tauri)                    │
│   capabilities.rs  registers every capability (one place)│
│   bridge.rs        invoke_capability → registry dispatch │
│   watch/exec/forward/logs  long-lived streams → events   │
└───────────────┬──────────────────────────────────────────┘
                │
   ┌────────────┴─────────────┬───────────────────────┐
   │ crates/capability        │ crates/kube           │
   │ registry: id, schemas,   │ kubeconfig, clusters, │
   │ annotations, handler     │ watches, actions,     │
   │                          │ helm, metrics, CRDs   │
   ├──────────────────────────┴───────────────────────┤
   │ crates/mcp — MCP server (stdio + HTTP) generated  │
   │ from the same registry; completeness test in CI   │
   └───────────────────────────────────────────────────┘
```

### The capability registry (core pattern)

Every backend operation is a `Capability` (`crates/capability`): an id (e.g. `k8s.listPods`), JSON input/output schemas, safety annotations (`read_only`, `destructive`, `requires_confirm`), and an async handler. `apps/desktop/src-tauri/src/capabilities.rs` is the **single place** capabilities are registered.

One registration produces two surfaces:

1. **Tauri** — the UI calls `invoke_capability(id, payload)` through the transport shim.
2. **MCP** — `crates/mcp` turns each capability into an MCP tool with the same schema.

A unit test (`every_capability_is_mcp_exposed`) asserts the registry and the MCP tool list match exactly, so "everything is MCP-accessible" is enforced by CI, not convention.

### Long-lived streams

Watches, pod exec, log tails, and port-forwards don't fit request/response. They are dedicated Tauri commands (`start_resource_watch`, `start_pod_exec`, `start_log_stream`, `start_port_forward`, plus matching stop/input commands) that push data to the WebView via Tauri **events**. The frontend side lives in `apps/desktop/src/lib/` (`watch.ts`, `exec.ts`, `logsStream.ts`, `forward.ts`).

### The transport shim

`apps/desktop/src/transport/transport.ts` is the only frontend module that imports `@tauri-apps/api`. Everything else — stores, components, tests — depends on the shim's interface, which makes the UI testable in jsdom and keeps the Tauri coupling swappable.

### Running the MCP server

```sh
cargo run -p srelens-desktop --no-default-features -- --mcp-stdio
cargo run -p srelens-desktop --no-default-features -- --mcp-http 127.0.0.1:8765
```

The HTTP transport binds to loopback, requires a bearer token (`crates/mcp/src/auth.rs`, supplied via `SRELENS_MCP_TOKEN` — never a flag, since argv is readable via `ps`), and checks the `Host` header on every route to block DNS rebinding. Gated tools go through an injected `ConfirmPolicy` (`crates/mcp/src/policy.rs`), which receives a `ConsentKind` derived from the capability's annotations: a gated capability that is `read_only` is a `SensitiveRead`, anything else gated is `Destructive`. The desktop app prompts in-app for both; headless runs need `"_confirm": true` on the call plus the matching flag (`--mcp-allow-destructive` or `--mcp-allow-sensitive-reads`) — `_confirm` alone never authorizes anything, and neither flag implies the other. See the [user guide](USAGE.md#mcp-server-for-ai-agents) for the full security model.

## Testing standards

Development is **test-driven — this is mandatory, not aspirational**:

1. Write a failing test that motivates the change (red).
2. Make it pass with the simplest implementation (green).
3. Refactor with the tests as a safety net.

**Coverage floors, enforced in CI:**

- TypeScript: **85% lines** (Vitest thresholds in `vitest.config.ts`).
- Rust: **55% lines today, ratcheting toward 85%** — the Tauri runtime shell is excluded from measurement, and much of `crates/kube` needs a live cluster to exercise; the floor rises as cluster-bound integration tests land. Never lower it.

Test placement conventions:

- Rust: unit tests in `#[cfg(test)]` modules next to the code.
- TypeScript: `Foo.test.tsx` / `foo.test.ts` beside `Foo.tsx` / `foo.ts`. Component tests use Testing Library against the transport shim (mocked), not Tauri.

## Adding a new capability (walkthrough)

1. **Write the handler test-first** in the right `crates/kube` module (or a new one): a `pub fn <name>_capability(cache: …) -> Capability` returning schemas derived with `schemars` and an async handler.
2. **Register it** in `apps/desktop/src-tauri/src/capabilities.rs` — the MCP surface appears automatically, and `every_capability_is_mcp_exposed` keeps you honest.
3. **Annotate safety** — mark it `read_only`, or `destructive`/`requires_confirm` so MCP hints and UI confirmations are driven from one place.
4. **Consume it in the UI** — call it through the data layer in `apps/desktop/src/lib/`, never directly from a component.
5. Run `cargo test && pnpm test` and check coverage before opening a PR.

## Continuous integration

`.github/workflows/ci.yml` runs on every push and PR:

- **frontend** — Vite build + Vitest with the 85% coverage threshold.
- **backend** — `cargo llvm-cov` with the ratcheting coverage floor (see above).

A separate Release workflow runs on pushes to `dev` (rolling pre-releases) and `main` (Conventional-Commit-driven stable releases); see `.github/workflows/release.yml`.

Both CI jobs must be green.

## Conventions

- **Commits** — imperative subject line, body explaining *why* when non-obvious.
- **Formatting** — `cargo fmt` for Rust; the existing Prettier-ish style for TS (match surrounding code).
- **UI** — primitives come from `src/components/ui` (shadcn/radix) and `src/ui`; feature views compose them. Styling lives in `src/ui/styles.css` design tokens — avoid ad-hoc inline styles.
- **No direct Tauri imports** outside `src/transport/`.
