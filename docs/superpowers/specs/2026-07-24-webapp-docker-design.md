# srelens web mode: multi-user webapp in Docker — design

**Date:** 2026-07-24
**Status:** Shipped (branch refactor/streams-eventsink)

## Goal

Expose srelens — today a Tauri v2 desktop app — as a multi-user webapp shipped as a
single Docker image. A team deploys one container; members log in via their
identity provider, upload their own kubeconfigs, and get the full srelens
experience (resource browsing, watches, logs, pod exec, terminal, helm,
port-forwarding) in the browser. The desktop app remains the primary product and
must be unaffected.

## Decisions (from brainstorming)

| Question | Decision |
|---|---|
| Audience | Multi-user team server |
| Auth to srelens | OIDC/SSO (authorization code + PKCE) |
| Cluster credentials | Per-user kubeconfig upload, encrypted at rest |
| Local terminal | Container shell per user, scoped to the user's kubeconfig |
| Port-forward | HTTP reverse proxy through the server (`/pf/{id}/…`) |
| Server state | Embedded SQLite on a volume |
| Rollout | Multi-user from day one (no single-user interim release) |
| Approach | A: native Rust web server crate reusing existing crates |

## Why the architecture supports this

- All capability calls funnel through one generic Tauri command
  (`invoke_capability`) into a pure `Registry` (`crates/capability`) of ~80
  capabilities implemented in `crates/kube`.
- The frontend has a single transport chokepoint:
  `apps/desktop/src/transport/transport.ts` (only file importing
  `@tauri-apps/api/core`); 96 `invokeCapability` + 15 `invokeCommand` call sites
  sit above it in `src/lib/*.ts`.
- `crates/mcp/src/http.rs` already proves the "axum server over the registry"
  pattern, and `srelens --mcp-http` already runs headless.
- The remaining gap is streaming: watch/logs/exec/terminal/helm/forward use
  Tauri `emit`/`listen` with dynamically generated channel names.

## Architecture

New workspace crate **`crates/server`** (`srelens-server`) + a `serve` mode in
the existing binary:

```
srelens serve --addr 0.0.0.0:8080 --data /data
```

```
crates/server/src/
  lib.rs               axum app assembly, router, shared state, background sweeps
  auth/                mod.rs (AuthConfig), idp.rs (IdentityProvider trait +
                        pending-login store), oidc.rs (real OIDC adapter),
                        routes.rs (login/callback/dev-login/logout),
                        session.rs (cookies, CSRF/require_session middleware)
  db.rs                sqlx SQLite open + embedded migrations
  stores.rs            users, sessions, kubeconfigs, settings queries
  crypto.rs            master key load/generate, AES-256-GCM seal/open
  users.rs             UserEnvs: per-user materialization, cache, eviction
  streams.rs           per-user stream registry (watch/logs/exec/terminal/helm)
  assets.rs            built Vite frontend embedded via rust-embed, SPA fallback
  api.rs               POST /api/capability/:id (+ WEB_DENIED_CAPABILITIES)
  api_command.rs       POST /api/command/:cmd
  api_kubeconfigs.rs   kubeconfig list/upsert-by-name (put)/delete
  ws/                  mod.rs, route.rs (/api/ws upgrade + Origin check),
                        hub.rs (per-user connection registry + fanout)
```

Note: this differs from the original sketch above (`db/` directory, `ws.rs`,
`static.rs`, no `stores.rs`/`crypto.rs`/`users.rs`/`streams.rs` split) — the
file map here reflects what was actually built.

The plan below was executed out of its listed top-to-bottom order, as
2 → 3A → 3B1 → 3B2 → 4A (see the phased task breakdown in
`.superpowers/sdd/`), not strictly sequentially.

### EventSink refactor (desktop code sharing)

The streaming modules in `apps/desktop/src-tauri/src/` (`watch.rs`, `logs.rs`,
`exec.rs`, `terminal.rs`, `helm.rs`, `forward.rs`) currently end in
`app.emit(&channel, payload)`. Their cores move to shared code emitting into a
generic trait:

```rust
trait EventSink: Send + Sync + 'static {
    fn emit(&self, channel: &str, payload: serde_json::Value);
}
```

Implemented once by Tauri (`AppHandle::emit`) and once by the WS layer. Desktop
keeps the exact same runtime behavior with one added indirection. Exit/close
notifications are ordinary events on named channels (`exec:exit:<id>`,
`forward:closed:<id>`, …); the WS-protocol `{op:"closed"}` frame is generated
by the WS layer itself, not by the sink trait. The cores live in a new
workspace crate `crates/streams` (`srelens-streams`) so both the desktop crate
and the server crate can depend on them.

### Frontend transport

`transport.ts` becomes an interface with two implementations chosen at startup
(`window.__TAURI_INTERNALS__` present → Tauri; else → web):

- `invokeCapability(id, input)` → `fetch POST /api/capability/:id`
- `invokeCommand(cmd, args)`   → `fetch POST /api/command/:cmd`
- `on/subscribe(channel)`      → single `/api/ws` socket, auto-reconnect with
  re-subscribe of live channels
- `relaunchApp`/`appVersion`   → web no-op / `/api/version`

One build serves both targets. A `platform.isWeb` flag hides desktop-only UI
(updater, native pickers, reveal-log) and swaps file save → browser download;
kubeconfig import uses the existing paste flow plus browser file upload.

### WebSocket protocol

Single socket, JSON frames, mirroring the Tauri emit/listen model so
`src/lib/{watch,logsStream,terminal,…}.ts` keep their shape:

```
client → {op:"sub", channel} | {op:"unsub", channel} | {op:"input", channel, data}
server → {channel, payload}  | {op:"closed", channel, reason}
```

Backpressure: per-channel bounded queues; watches coalesce (they emit full
snapshots), log streams drop-with-marker for slow consumers. Streams are owned
by the user's socket set; teardown when the last socket closes (60 s reconnect
grace).

## Auth & sessions

- OIDC authorization-code + PKCE (`openidconnect` crate). Config env:
  `SRELENS_OIDC_ISSUER`, `SRELENS_OIDC_CLIENT_ID`, `SRELENS_OIDC_CLIENT_SECRET`,
  `SRELENS_PUBLIC_URL`, optional `SRELENS_OIDC_ALLOWED_DOMAINS`. Local-dev
  bypass: `SRELENS_DEV_LOGIN=<email>` enables `POST /auth/dev-login`, which
  skips the IdP entirely.
  `SRELENS_OIDC_ALLOWED_GROUPS` group-based gating is **deferred** — not
  implemented; only the email-domain allowlist is enforced today.
- Routes: `GET /auth/login` (mints a random binder token, stores its hash
  against the pending OIDC state, and sets it as the short-lived
  `srelens_login` cookie scoped to `/auth`), `GET /auth/callback` (verify the
  `srelens_login` binder cookie hash matches the pending state before doing
  anything else — a mismatch means the login was started in a different
  browser and is rejected; reject `email_verified == false` from the IdP
  claims; gate by allowed email domain; upsert user by `iss`+`sub` → create
  session), `POST /auth/dev-login`, `POST /auth/logout`.
- Server-side sessions: opaque 256-bit token, stored hashed in SQLite; cookie
  `HttpOnly; Secure; SameSite=Lax`; 12 h idle / 7 d absolute expiry. Revocation =
  row delete. OIDC tokens never reach the browser.
- Auth middleware on `/api/*`, `/pf/*`, WS upgrade → `UserCtx{user_id, …}`.
  Browser → login redirect; API → 401 JSON.
- CSRF: OIDC state+nonce for the login flow, plus a `srelens_login` binder
  cookie (see below) to bind a login attempt to the browser that started it.
  For `/api` mutations: requiring the presence of a non-empty
  `X-Srelens-Csrf` header (the value itself is not checked against the
  session) forces the browser into a CORS preflight, which a cross-origin
  page cannot satisfy for a custom header — cheap to enforce given the single
  fetch chokepoint. The WS upgrade can't carry a custom header, so it relies
  on cookie auth plus an `Origin` check instead (any present `Origin` must
  match the configured public URL).

## Data model (SQLite, sqlx, embedded migrations)

```
users        id, iss, sub, email, display_name, created_at, last_login_at
sessions     token_hash, user_id, created_at, expires_at, last_seen_at
kubeconfigs  id, user_id, name, ciphertext, nonce, created_at, updated_at
settings     user_id, key, value_json
```

- Kubeconfigs encrypted AES-256-GCM under a master key from
  `SRELENS_MASTER_KEY` or auto-generated `/data/master.key` (0600); per-row
  random nonce. Users add (paste/upload) and delete kubeconfigs.
  **Deferred:** in-place rename of a stored kubeconfig — today, renaming means
  delete-and-re-add.
- Frontend settings stay in `localStorage` in web mode; the `settings` table is
  reserved for state that must be server-side.

## Per-user isolation

- **`UserEnvs`** (`crates/server/src/users.rs`): a single
  `Mutex<HashMap<i64, Arc<UserEnv>>>` keyed by user id. `env_for(db, key,
  user_id)` returns the cached `UserEnv` (touching its last-used stamp) or
  builds one: decrypts the user's stored kubeconfigs, materializes each as an
  individual file under `<data>/runtime/users/<id>/kc-<kubeconfig_id>.yaml`
  (0600 files, 0700 directories, existing dir wiped before rewrite), then
  builds a `ClientCache` and a per-user `Registry` (via the shared
  `RegistryFactory`) over those paths. No capability code changes — the
  registry is constructed once per user, not per request.
- **On-disk hygiene:** the whole `<data>/runtime/` tree is wiped at process
  startup (`UserEnvs::wipe_runtime`), so a prior crash never leaves decrypted
  kubeconfigs behind across restarts.
- **Eviction:** a background sweep every 5 min (`evict_idle`) drops any
  `UserEnv` idle longer than 30 min (`USER_ENV_IDLE_SECS`), removing its
  materialized files. **Logout teardown:** `POST /auth/logout` resolves the
  user from the session token before revoking it and calls
  `state.user_envs.invalidate(user_id)` immediately, so decrypted runtime
  files don't linger for up to 30 min after a user signs out.
- **Terminal:** in-container PTY (`portable-pty`), `KUBECONFIG` → tmpfs
  single-context overlay (same trick as desktop), cwd `/data/home/<user_id>`,
  shell bash, max 4 PTYs/user, killed on session expiry. Users get shell access
  to the container by design (trusted team tool); container runs non-root.
- **Port-forward:** binds `127.0.0.1:0` in-container; `/pf/{forward_id}/…`
  proxies with ownership check; HTTP(S) + WS upgrade only (raw TCP from the
  user's machine is out of scope). Forwards die with the session.
- **`k8s.deleteContext` is denied on web** (`api::WEB_DENIED_CAPABILITIES`):
  it mutates the per-request materialized kubeconfig, which is rebuilt from
  the database on every `env_for` call, so the edit would silently revert on
  the next rebuild. Blocked with 400 until a database-backed context-rename
  flow exists (see Deferred, below).
- **Helm repo config and toolbox installs are container-global, not
  per-user:** `helm repo add`/`update` and toolbox plugin installs write into
  shared container state rather than a per-user sandbox. This is accepted for
  a trusted team tool (same trust model as terminal shell access) rather than
  fixed now; to be spelled out in `docs/WEB.md` when it's written (tracked as
  Plan 6).

## Docker packaging

Multi-stage `Dockerfile` at repo root:

1. `node:22-slim` + pnpm → build `apps/desktop/dist`.
2. `rust:*-slim` → build `srelens` with `server` feature; `dist/` embedded via
   `rust-embed` (single self-contained binary).
3. `debian:bookworm-slim` runtime: non-root user, bash, ca-certificates,
   pinned kubectl + helm, curl/jq. Cloud auth CLIs (aws/gcloud) not bundled —
   documented "extend this image" recipe for exec-plugin kubeconfigs.

Contract: listens `:8080`; state volume `/data`; `GET /healthz` + `GET /readyz`;
plain HTTP (TLS via operator's reverse proxy; cookies `Secure` when
`SRELENS_PUBLIC_URL` is https). Sample `docker-compose.yml` + `docs/WEB.md`.
Image `ghcr.io/srelens/srelens`, amd64+arm64, built in the existing release CI.

## Error handling

- Capability errors keep their structured JSON shape (as in the MCP path) with
  proper status codes: 400 invalid input, 401/403 auth, 404 unknown capability,
  502 cluster unreachable.
- WS: per-stream `{op:"closed", channel, reason}`; malformed client frames close
  the socket; one bad channel never kills the connection.
- Kubeconfig decryption failure (rotated master key) → explicit "re-upload
  needed" state, not a 500.
- Credential redaction check on all server logs.

## Testing

- **Unit (crates/server):** auth middleware (expired/forged sessions), AES-GCM
  round-trip, CSRF, WS framing, pf-proxy ownership.
- **Integration:** axum app + in-memory SQLite + stub OIDC provider fixture;
  completeness test that every desktop-reachable capability id is reachable via
  `/api/capability` (mirrors `crates/mcp/src/completeness.rs`).
- **Frontend:** transport contract tests via the existing `Invoker` injection;
  web transport against a mock WS server.
- **E2E smoke:** docker build + run against `kind`: login (stub IdP), upload
  kubeconfig, list pods, stream logs, open terminal.

## Out of scope

- Auto-updater, native dialogs/menus, reveal-in-Finder (hidden via
  `platform.isWeb`).
- MCP endpoints in web mode (multi-user MCP auth is a future design).
- Raw-TCP port-forward reachability, HA/multi-replica, Postgres, Kubernetes
  impersonation, admin UI (user gating via IdP + allowed email domains only;
  `SRELENS_OIDC_ALLOWED_GROUPS` group-based gating is deferred).

## Alternatives considered (rejected)

- **B: Desktop-in-container (Xvfb + noVNC pixel streaming).** Near-zero code
  change but one app instance per user, heavy RAM, degraded UX, no real
  auth/multi-user model. An ops hack, not a product feature.
- **C: Separate web BFF over the existing MCP HTTP server.** MCP JSON-RPC has
  no streaming (watches/logs/exec/terminal), no auth, no sessions — all of it
  would be rebuilt in a second service plus a serialization hop. Approach A
  with extra steps.
- **W1: Compile the core to browser WASM, no backend.** kube-rs does not build
  for `wasm32-unknown-unknown` (tokio net/process/fs, hyper sockets, rustls,
  watcher runtime) — the engine would need a fetch-based rewrite. Browsers
  block apiserver calls without `--cors-allowed-origins`, which managed
  clusters (EKS/GKE/AKS) cannot set. Client-cert auth and exec-plugin auth
  (aws/gcloud binaries) are impossible in a browser, and exec/attach WS
  subprotocol auth headers cannot be set from JS. Helm, toolbox, and PTY
  terminal require process spawning. Infeasible.
- **W2: Browser WASM core + thin proxy container.** The attractive property
  (credentials never stored server-side) evaporates because client-cert and
  exec-plugin auth must run server-side anyway; helm/terminal still need the
  server; and the W1 engine rewrite is still required. Strictly more work than
  Approach A for weaker results.
- **W3: Package the server as server-side WASM (WASI/Spin/wasmCloud).** WASI
  has no process spawning (helm, kubectl in terminals) and no PTYs; the
  benefits (cold start, density) do not apply to a long-running stateful
  server. Native container it is.

## Key existing files

`apps/desktop/src/transport/transport.ts` ·
`apps/desktop/src-tauri/src/{bridge.rs,capabilities.rs,main.rs}` ·
`apps/desktop/src-tauri/src/{watch,logs,exec,forward,terminal,helm}.rs` ·
`crates/mcp/src/http.rs` · `crates/kube/src/client_cache.rs`

## Deferred beyond this feature

Shipped as `crates/streams`, `crates/registry`, `crates/server`
(`srelens-server`), the multi-stage `Dockerfile`, `docker-compose.yml`, and
`docs/WEB.md`, on `refactor/streams-eventsink`. The following were
deliberately left out rather than solved partially — each is a real,
truthful gap in what shipped, not a bug:

- **`SRELENS_OIDC_ALLOWED_GROUPS` group-based gating** — not implemented;
  `AuthConfig` (`crates/server/src/auth/mod.rs`) only enforces the
  email-domain allowlist (`SRELENS_OIDC_ALLOWED_DOMAINS`).
- **Kubeconfig rename** — there is no in-place rename of a stored kubeconfig
  row; renaming means delete-and-re-add under the new name. `k8s.deleteContext`
  stays denied on the web surface for the same underlying reason (the
  materialized kubeconfig is rebuilt from the database on every `env_for`
  call, so an in-place edit would silently revert).
- **HTTPS-upstream and raw-TCP port-forwards** — `/pf/{id}/…`
  (`crates/server/src/pf_proxy.rs`) only proxies plain HTTP and WebSocket
  upgrades to the loopback-bound forward; an HTTPS upstream or arbitrary raw
  TCP forwarding (as the desktop app supports locally) is out of scope.
- **Per-user helm repo config** — `helm repo add`/`helm repo update` write into
  process-wide helm state shared by every user in the container, so both are
  denied on web (`api::WEB_DENIED_CAPABILITIES`) rather than given a per-user
  sandbox. The rest of helm (install/upgrade/rollback/uninstall/template/
  search/list/get), which use per-context temp kubeconfigs, remain allowed.
- **MCP endpoints in web mode** — the MCP HTTP server is a separate surface
  from `srelens-server`; multi-user MCP auth over the web deployment is a
  future design, not attempted here. (The in-container terminal is likewise
  never exposed through the MCP capability registry.)
- **HA / multi-replica / Postgres** — the server assumes a single replica with
  embedded SQLite on one volume; there is no shared-state story for running
  more than one instance behind a load balancer.
