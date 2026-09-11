# srelens Plugin Architecture — Design / ADR

- **Date:** 2026-07-23
- **Status:** Proposed (supersedes the #42 "Extension platform design spike"; reframes #43, #44)
- **Owners:** srelens core team
- **Related issues:** #42 (design spike), #43 (renderer API surface), #44 (extension host sidecar + MCP bridging), #23 (MCP consent), #56 (AI assistant), #153 (topology surface)

## Implementation status (2026-09-11)

Native authoring **and** Freelens/OpenLens compatibility are both required.
The first implementation is a declarative contract/broker prototype in
`crates/plugin-host`; it executes no third-party code and is not yet loaded by
application UIs. See [the current implementation and delivery plan](../EXTENSIONS.md)
for runnable GitOps examples, limitations and the compatibility matrix.
This stages the broker before executable runtimes; it does not remove the native
subprocess or compatibility-runtime work below.

## 1. Context & problem

srelens is a Tauri desktop app: a **Rust core** (`crates/kube`, Tauri commands), a **React/webview frontend** (`apps/desktop/src`), and an **MCP server** built directly from a single capability **`Registry`** (`srelens_capability::Registry` → `build_registry_with()` → `srelens_mcp::McpServer`). Today every backend capability is registered in one place (`capabilities.rs`) and is simultaneously available to the UI and to AI agents over MCP.

We want a **pluggable architecture** so integrations like **ArgoCD, Flux, and Trivy** — and third parties — can extend srelens with **both backend and frontend** behaviour, in the spirit of Freelens/Lens, **without forking srelens**.

Two hard constraints shape every decision:

1. **srelens is Tauri, not Electron.** There is no Node runtime in the app and the renderer is CSP-locked. The literal Lens model (a Node main process plus Node running inside the renderer, sharing React/Mobx singletons) does not map directly.
2. **srelens's ethos is "safe operations": brokered, capability-annotated, consent-gated.** An extension model that grants ambient authority (Lens-style full-trust renderer code) is at odds with this.

## 2. Decision drivers

- Preserve the **registry-as-single-source-of-truth**: anything a plugin adds should be reachable by the UI *and* by AI agents over MCP, with identical consent gating. This is srelens's differentiator.
- **No ambient authority.** Plugins start with zero access; everything is brokered and declared.
- **Language-agnostic** backend (ArgoCD/Flux tooling is Go; Trivy is a binary; others may be Python/JS).
- **Freelens compatibility** as a real porting path, without importing Lens's weak renderer-trust model.
- **Dogfooding**: the API must be proven by building real first-party plugins on it.

## 3. Decisions (summary)

| # | Decision | Choice |
|---|----------|--------|
| D1 | Backend runtime | **Language-agnostic, sandboxed subprocess** speaking JSON-RPC (MCP framing) over stdio; registers capabilities into the shared `Registry`. |
| D2 | Frontend model | **Declarative-first** UI contributions rendered by host components; **sandboxed `<iframe>` escape hatch** for bespoke UI. No third-party JS in the host renderer. |
| D3 | AI/MCP | Plugin capabilities flow into the shared `Registry` → **auto-exposed over MCP** with the same consent gating. |
| D4 | Freelens compat | **Source-compat via `@srelens/lens-compat` shim** (rebuild against the shim) running on a **Node extension host** + `Renderer.*` shim → ContributionRegistry. Best-effort, documented matrix. |
| D5 | Dogfooding | Design the API **and** build **ArgoCD/Flux/Trivy as first-party reference plugins**. Do not extract existing built-ins. |
| D6 | Distribution | **All three channels in v1**: local side-load (file/folder/URL/git), a curated **first-party set** shipped with releases, and a **hosted marketplace/registry**. |
| D7 | Trust/signing | **Signing required** for marketplace and distributed plugins (verified on install/update). **Unsigned local side-loads allowed only behind a developer-mode flag** (off by default, with prominent warnings). |

## 4. Architecture overview

```
                        ┌────────────────────────── srelens (Tauri) ──────────────────────────┐
                        │                                                                      │
 AI agent ──MCP──▶ srelens_mcp ──▶ ┌───────────── shared Registry (capabilities) ───────────┐ │
                        │          │  core caps (k8s, helm, toolbox …)                        │ │
 Frontend (webview)     │          │  plugin/<id>/<cap>  ◀── registered at plugin handshake   │ │
   ├─ ContributionRegistry ◀───────┴──────────────────────────────────────────────────────┘ │
   │    ├─ declarative panels (Table / DetailTab / Form / actions) ── invokeCapability() ─────┼─▶ broker
   │    └─ sandboxed <iframe> ── postMessage bridge SDK ──────────────────────────────────────┼─▶ broker
   │                                                                                          │
   │                         ┌──────────── crates/plugin-host ────────────┐                   │
   │  Rust core (crates/kube)│  supervisor · sandbox · broker · protocol  │                   │
   └─────────────────────────┤                                            │                   │
                             │   ▲ JSON-RPC/stdio                         │                   │
                             └───┼────────────────────────────────────────┘                   │
                        ┌────────┴─────────┐        ┌───────────────────────────┐              │
                        │ native backend   │        │ Node extension host       │  (lens-compat)│
                        │ (Go/Rust/Py/JS)  │        │  @srelens/lens-compat     │              │
                        └──────────────────┘        └───────────────────────────┘              │
                        └──────────────────────────────────────────────────────────────────────┘
```

Everything privileged flows through the **broker**; the two extension surfaces (backend process, frontend iframe) both hold **no ambient authority**.

## 5. Plugin anatomy

A plugin is a signed archive containing a manifest, optional backend artifact(s), and optional frontend assets.

```jsonc
// plugin.json
{
  "id": "com.srelens.trivy",
  "name": "Trivy",
  "version": "0.1.0",
  "srelensApiVersion": "^1.0",
  "kind": "native",                 // "native" | "lens-compat"
  "publisher": { "name": "srelens", "signature": "…" },

  "backend": {                       // optional
    "type": "native",               // "native" | "node-host"
    "entry": { "darwin-arm64": "bin/trivy-plugin", "linux-x64": "bin/trivy-plugin", "…": "…" }
  },

  "frontend": {                      // optional
    "contributions": "contrib.json",// declarative descriptors
    "iframe": "ui/"                  // optional bespoke UI assets
  },

  "capabilities": [
    { "name": "scan", "readOnly": true, "input": { /* JSON schema */ }, "output": { /* JSON schema */ } }
  ],

  "contributions": {
    "detailTabs": [
      { "id": "vulns", "title": "Vulnerabilities", "forKinds": ["pods","deployments"], "capability": "scan" }
    ],
    "pages": [ /* nav pages */ ],
    "rowActions": [ /* … */ ],
    "settings": [ /* … */ ]
  },

  "permissions": {
    "k8s": [ { "verbs": ["get","list"], "resources": ["pods"], "groups": [""] } ],
    "exec": [ "trivy" ],
    "network": [ "https://ghcr.io" ],
    "fs": []
  }
}
```

**Manifest = the whole contract.** The consent screen, the broker's runtime enforcement, the ContributionRegistry, and the MCP catalog are all derived from it.

## 6. Backend runtime & broker (D1, D3)

- The host (`crates/plugin-host`) spawns each plugin backend as a **supervised child process** with OS-level sandboxing where available (no ambient FS/network/exec). Communication is **JSON-RPC over stdio**, reusing MCP message framing so the two protocols share code.
- **Handshake:** plugin advertises its capabilities; host validates them against the manifest and inserts them into the shared `Registry` under `plugin/<id>/<cap>`, carrying the `readOnly|mutating` annotation.
- **Broker:** the only way a plugin reaches anything privileged. Host-exposed broker methods:
  - `k8s.get/list/watch/apply/delete` — scoped to the manifest's `permissions.k8s`; reuses `crates/kube` clients (plugins never construct their own kube client).
  - `exec(name, args)` — only for binaries in `permissions.exec`.
  - `http(request)` — only to hosts in `permissions.network`.
  - `fs` — only within `permissions.fs` paths.
  Every call is checked against grants **and** the user's consent policy; mutating calls follow the same confirm policy as first-party actions and MCP (#23).
- **Consequences of landing in the `Registry`:**
  - Frontend invokes any capability through one generic Tauri command: `invokeCapability("plugin/trivy/scan", args)`.
  - The **MCP server exposes it automatically** — a completeness test asserts every registered plugin capability appears as an MCP tool (extends #44).

## 7. Frontend contribution model (D2)

- A renderer-side **`ContributionRegistry`** mirrors the backend registry pattern (cf. today's `NAV_KINDS` / capability registry). Manifests register: **nav pages, resource-detail tabs, table columns, row/menu actions, status-bar items, settings panes.**
- **Declarative panels (default).** The host renders its *own* themed primitives (`Table`, detail tabs, forms, `StatusPill`, action buttons) bound to the outputs of the plugin's capabilities. This covers the majority of integrations (list Applications, show a report, submit a "sync" form). Benefits: **CSP-safe, theme- and a11y-consistent, no third-party JS in the host renderer.**
- **Iframe escape hatch.** For bespoke UI the plugin ships assets loaded into a **sandboxed `<iframe>`** (isolated origin, strict CSP). It talks to the host through a typed **bridge SDK** over `postMessage`: `invokeCapability`, `openResource`, `notify`, `getThemeTokens`, `subscribe(watch)`. The iframe has **no access to host DOM or state**.

## 8. Freelens compatibility layer (D4)

An adapter built **on top of** the native model — it does not introduce a second privileged runtime.

- **`@srelens/lens-compat`** re-exposes Lens's `Main.LensExtension` / `Renderer.LensExtension` base classes and the `Main.*` / `Renderer.*` registration namespaces.
- The **Node extension host** is one concrete `backend.type: "node-host"` implementation: a **sandboxed Node sidecar** (managed/bundled Node) that loads the extension's compiled `main` half. Lens `Main.*` APIs map onto broker + `Registry` registrations.
- The **`Renderer.*` shim** maps Lens renderer registrations (`registerClusterPage`, `KubeObjectDetailItem`, `KubeObjectMenuItem`, `KubeObjectStatusText`, …) onto the srelens `ContributionRegistry`. The extension's renderer bundle runs inside the **sandboxed iframe** with a React + shim runtime; Lens `Store` / `K8sApi` objects are **emulated over broker calls**.
- **Source-compat, best-effort.** Freelens extensions **rebuild** against the shim (they are not drop-in). A published **compatibility matrix** documents each Lens API as supported / stubbed / unsupported. Unsupported APIs throw clear, actionable errors.

## 9. Security, trust & consent (D7)

- **Manifest permissions are the contract.** Install surfaces a **consent screen** enumerating exactly what the plugin may do (k8s scopes, exec, network hosts, fs paths). Runtime enforcement lives in the broker.
- **Mutating capabilities** are gated by the same confirm/consent policy as first-party actions and MCP (#23).
- **Signing:** marketplace and distributed plugins **must be signed**; the host verifies signature + integrity on install and on every update, and **re-prompts consent when an update widens permissions**. First-party plugins are signed by srelens.
- **Developer mode (off by default):** a settings flag permits **unsigned local side-loads** for plugin authors, with a persistent warning banner while any unsigned plugin is enabled. Normal users never run unsigned code.
- **Isolation recap:** backend = sandboxed supervised subprocess; frontend = sandboxed iframe; neither holds ambient authority. Per-plugin **enable / disable / revoke** in Settings (#43); disabling tears down the process and iframe and removes its `Registry` entries so its MCP tools disappear too.

## 10. Distribution & lifecycle (D6)

Three channels, all in v1:

1. **Local side-load** — install from a file/folder, URL, or git ref (dev + power users; unsigned only in developer mode).
2. **Curated first-party set** — Trivy/Flux/ArgoCD etc., shipped or fetched from srelens releases, signed by srelens.
3. **Hosted marketplace/registry** — browse/search/install signed third-party plugins from within srelens.

**Lifecycle:** discover → download → **verify signature** → **consent** → enable → run. Updates re-verify and re-consent on permission changes. Remove tears down process + iframe + `Registry` entries + settings. `srelensApiVersion` compatibility is checked at load; namespaced capabilities (`plugin/<id>/…`) prevent collisions.

## 11. Reference plugins (D5) — dogfooding the API

- **Trivy** — backend `scan(image|namespace)` capability execs `trivy` (declared `exec`) → structured report. Frontend: a declarative report panel + a **"Vulnerabilities" resource-detail tab** on pods/deployments. MCP: `plugin/trivy/scan` callable by agents.
- **Flux** — brokered reads of Flux CRDs (Kustomizations / HelmReleases / GitRepositories; declared CRD read perms) + a `reconcile` **mutating** capability (annotate-to-trigger). Frontend: a nav page with reconciliation status, detail tabs, and a confirm-gated **"Reconcile now"**.
- **ArgoCD** — brokered ArgoCD API/CRD access (declared network host + CRD perms). Frontend: a nav page listing **Applications** (sync/health), an app-detail view that **reuses the topology surface (#153)**, and confirm-gated **"Sync" / "Refresh"**; iframe escape hatch if a richer app view is wanted.

Each reference plugin simultaneously validates a backend capability set, a frontend contribution type, and an MCP tool — proving all three surfaces of the API.

## 12. Codebase shape

- **New crate `crates/plugin-host`** — process supervision, sandboxing, broker, JSON-RPC/stdio protocol.
- **Extend `srelens_capability::Registry`** to accept dynamically-registered, namespaced, permission-annotated plugin capabilities (core registration stays as-is).
- **Frontend `apps/desktop/src/plugins/`** — `ContributionRegistry`, declarative renderers, iframe bridge SDK, plugin settings/consent UI.
- **Published packages** — `@srelens/plugin-sdk` (native authoring) and `@srelens/lens-compat` (Freelens shim).
- **Unchanged** — the existing capability → MCP path; plugin capabilities ride it for free (guarded by a completeness test).

## 13. Phasing (supersedes #42–#44)

0. **ADR** (this document) — replaces #42.
1. **Native backend runtime**: `plugin-host` supervisor + broker + sandbox + `Registry` integration + generic `invokeCapability` + MCP bridging (+ completeness test). Replaces the #44 core.
2. **Frontend contributions**: `ContributionRegistry` + declarative panels + settings/enable-disable. Replaces #43.
3. **Iframe escape hatch** + typed bridge SDK.
4. **Freelens compat**: Node extension host + `Renderer.*` shim + compatibility matrix.
5. **Reference plugins**: Trivy → Flux → ArgoCD.
6. **Trust & distribution hardening**: signing/verification, developer-mode flag, then the hosted marketplace/registry.

## 14. Risks & mitigations

- **Sandboxing depth varies by OS.** Mitigate with a defined broker boundary as the primary control; OS sandbox is defense-in-depth. Prototype the sidecar boundary early (carried over from #42's acceptance criteria).
- **Node host weight/security (lens-compat).** Managed, sandboxed, updated separately; lens-compat is opt-in and clearly labeled best-effort.
- **Declarative model too limited.** The iframe hatch is the release valve; watch which contributions push authors to the iframe and promote common patterns into declarative primitives.
- **Marketplace trust/abuse.** Signing + review + permission-diff-on-update; revocation list.
- **API churn.** `srelensApiVersion` semver + a deprecation policy before 1.0 of the plugin API.

## 15. Open questions (to resolve during Phase 1)

- Exact JSON-RPC capability schema vs. reusing MCP tool schema verbatim.
- Watch/subscription semantics over the broker (backpressure, teardown on disable).
- Marketplace hosting/ownership and the plugin review process.
- Whether multi-window (#150) implies per-window or shared plugin instances.
