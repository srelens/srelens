# Running srelens as a web app

srelens can run as a multi-user web server in a single Docker container, instead
of (or alongside) the desktop app. A team deploys one container; each person
signs in with their identity provider (or a local dev login while trying it
out), uploads their own kubeconfig, and gets the full srelens experience —
resource browsing, watches, logs, in-pod exec terminals, helm,
and port-forwarding — in the browser. Nothing here changes the desktop app,
which remains the primary product.

## Quick start (trial)

```sh
docker run \
  -v srelens-data:/data \
  --tmpfs /data/runtime:mode=0700,uid=10001,gid=10001 \
  -p 8080:8080 \
  -e SRELENS_DEV_LOGIN=you@example.com \
  -e SRELENS_MASTER_KEY=$(openssl rand -hex 32) \
  ghcr.io/srelens/srelens
```

`SRELENS_MASTER_KEY` is required — it seals stored kubeconfigs at rest. The
one-liner above generates a throwaway key for the trial; for anything you'll
restart, set a **stable** key (a fresh key can't decrypt earlier data). The
`--tmpfs` keeps decrypted kubeconfigs in RAM, off the persistent volume.

Open `http://localhost:8080`, sign in with the dev-login email above (no IdP
required), and add a kubeconfig from **Settings → Kubernetes**.

`SRELENS_DEV_LOGIN` is meant for local trials only — see
[Security requirements](#security-requirements) before you point this at
anything reachable by anyone else.

## Real deployment (Docker Compose)

Use the `docker-compose.yml` at the repo root as a starting point:

```sh
git clone https://github.com/srelens/srelens.git
cd srelens
SRELENS_PUBLIC_URL=https://srelens.example.com \
SRELENS_OIDC_ISSUER=https://your-idp.example.com \
SRELENS_OIDC_CLIENT_ID=... \
SRELENS_OIDC_CLIENT_SECRET=... \
SRELENS_OIDC_ALLOWED_DOMAINS=example.com \
SRELENS_MASTER_KEY=$(openssl rand -hex 32) \
docker compose up -d
```

This runs `ghcr.io/srelens/srelens` (or `docker compose build` to build the
image locally), publishes `:8080`, and persists `/data` on a named volume.
Put a TLS-terminating reverse proxy in front of it (nginx, Caddy, an ingress
controller, a cloud load balancer, …) — see below for why this matters.

## Scaling and availability

> **Run a single instance.** Web mode is **not** horizontally scalable today —
> do not run more than one replica behind a load balancer.

Each instance keeps state that isn't shared across replicas: the SQLite database
(a single file — concurrent writers from multiple containers corrupt it),
in-memory OIDC login state (sign-in would start on one replica and fail its
callback on another), the live WebSocket streams, and port-forwards (a TCP tunnel
that lives inside the container that opened it). A second replica therefore breaks
logins, streams, and port-forwards, and risks database corruption.

Scale **vertically** (more CPU/RAM for the one instance) — a single instance
serves many users comfortably, since the workload is mostly async I/O. For
resilience, use `restart: unless-stopped` (as the compose file does) and a
persisted `/data` volume; on restart, users simply re-connect.

Horizontal scaling (multiple replicas via a replicated store, streams over the
WebSocket, and port-forward owner-routing) is designed but not yet built; it is
tracked as future work.

## Environment variables

| Variable | Required | Description |
| --- | --- | --- |
| `SRELENS_PUBLIC_URL` | Recommended | The externally reachable URL of the deployment, e.g. `https://srelens.example.com` (no trailing slash needed — it's trimmed). Defaults to `http://127.0.0.1:8080`. Session cookies are only marked `Secure` when this starts with `https://`, and it's used to build the OIDC redirect URI. |
| `SRELENS_OIDC_ISSUER` | With OIDC | The identity provider's issuer URL. Must be set together with `SRELENS_OIDC_CLIENT_ID` and `SRELENS_OIDC_CLIENT_SECRET` — all three or none. |
| `SRELENS_OIDC_CLIENT_ID` | With OIDC | OAuth client id registered with your IdP. |
| `SRELENS_OIDC_CLIENT_SECRET` | With OIDC | OAuth client secret. Never logged (redacted in debug output). |
| `SRELENS_OIDC_ALLOWED_DOMAINS` | Optional | Comma-separated list of email domains allowed to sign in (e.g. `example.com,corp.io`), case-insensitive. Leave unset to allow any authenticated email through. |
| `SRELENS_MASTER_KEY` | **Required** | 64 hex characters (32 bytes) that seal every kubeconfig and OIDC token at rest (AES-256-GCM). **Server mode refuses to start without it** — the key is never written to the data volume, so a stolen volume yields only sealed ciphertext. Generate with `openssl rand -hex 32` and keep it stable (rotating it makes existing sealed data unreadable). |
| `SRELENS_DATA` | Optional | Path to the data directory holding the SQLite database and per-user runtime files. Defaults to `/data` in the shipped image (`./srelens-data` if unset outside the container). |
| `SRELENS_DEV_LOGIN` | Optional | An email address; when set, enables `POST /auth/dev-login`, a no-IdP login shortcut for local trials. **Never set this on a deployment reachable by anyone other than you** — see below. |

At least one of OIDC (all three `SRELENS_OIDC_*` vars) or `SRELENS_DEV_LOGIN`
must be configured — the server refuses to start unauthenticated.

## Security requirements

- **Terminate TLS at a reverse proxy, and set `SRELENS_PUBLIC_URL=https://…`.**
  The container itself only speaks plain HTTP. Session cookies are marked
  `Secure` only when `SRELENS_PUBLIC_URL` starts with `https://`, and the OIDC
  redirect URI is derived from the same value — get this wrong and either
  cookies leak over plaintext or the IdP rejects the redirect.
- **Never set `SRELENS_DEV_LOGIN` on a public or otherwise non-loopback
  deployment.** It mints a session for that email with no identity check at
  all — anyone who can reach the container becomes that user. It exists for
  trying srelens out locally, nothing else.
- **Set a stable `SRELENS_MASTER_KEY` (required).** Every stored kubeconfig and
  OIDC token is sealed at rest under this key, and server mode won't start
  without it. It is read only from the environment and never written to disk, so
  the persistent volume holds ciphertext alone — but that also means the key
  lives wherever your environment/secret store keeps it, so back it up and keep
  it stable (rotating or losing it makes existing sealed data permanently
  undecryptable — users would need to re-upload).
- **Keep decrypted material off the persistent disk.** kubectl/helm/kube-rs need
  real kubeconfig files, so each user's are briefly *materialized* (decrypted,
  0600, per-user) along with their private helm state under `/data/runtime`. The
  shipped `docker-compose.yml` mounts that path as **tmpfs (RAM)** so this
  plaintext never touches the durable volume and is gone on restart. If you run
  the image without that compose file, mount an equivalent RAM-backed volume at
  `/data/runtime` yourself (`--tmpfs /data/runtime:mode=0700,uid=10001,gid=10001`).

## Adding kubeconfigs

There's no local filesystem to read kubeconfigs from in a browser, so each
user adds their own from **Settings → Kubernetes**: paste kubeconfig YAML
directly, or upload a file. Uploaded kubeconfigs are sealed (AES-256-GCM)
under the server's master key and scoped to that user's account — nobody else
can see or use them. Contexts from all of a user's uploaded kubeconfigs are
merged, the same way the desktop app merges multiple local kubeconfig files.

Removing an uploaded kubeconfig is supported; renaming one currently isn't —
delete and re-add it under the new name instead.

## Cluster OIDC sign-in (Headlamp-style)

Some Kubernetes clusters put their API server behind an OIDC identity provider,
and normally reach it through a `kubelogin` / `oidc-login` exec plugin. That
plugin can't run inside the headless container, which is why such clusters
otherwise fail to connect. Instead, srelens runs the OIDC authorization-code +
PKCE flow **in your browser** and uses the resulting `id_token` as the Bearer
for API calls — the same approach Headlamp takes — so no exec plugin is needed.

**This is separate from the app sign-in.** The `SRELENS_OIDC_*` variables above
authenticate you *to srelens*; cluster OIDC authenticates you *to a Kubernetes
API server*. They can use entirely different identity providers.

You opt a cluster into managed sign-in through **Settings → Contexts → Add
cluster**: fill in the API server URL, a CA certificate (or check *skip TLS
verify*), and the OIDC issuer + client id (+ optional client secret and extra
scopes). srelens synthesizes and stores the kubeconfig, stamping it with an
internal marker so *only* these clusters use the managed browser flow.

**Clusters from an existing kubeconfig are left alone.** A context that already
authenticates with its own `kubectl` exec plugin — `kubelogin`/`oidc-login`,
`aws eks get-token`, `gke-gcloud-auth-plugin`, etc. — keeps using that plugin
natively; srelens does **not** take it over, so on desktop your working setup is
untouched. (On the web container exec plugins can't run at all — so to use such
a cluster in web mode, re-add it via **Add cluster** to get the managed flow.)

**When you use an OIDC cluster that has no valid token, srelens prompts
"Sign in to `<cluster>`"** (from any view or stream). Clicking it runs the
sign-in flow and returns you to the app. **Settings → Contexts** lists your
OIDC clusters with their sign-in status and a per-cluster **Sign out**. srelens
refreshes the token server-side while it can, and re-prompts when it can't.

### Operator requirement: register the redirect URI

Each OIDC cluster's identity-provider client **must allow the redirect URI**

```
${SRELENS_PUBLIC_URL}/auth/cluster/callback
```

(the srelens analogue of kubelogin's `http://localhost:8000` callback). Use a
**public client with PKCE** — no client secret is required (a confidential
secret is supported but optional). `SRELENS_PUBLIC_URL` must be set correctly
for this URL to be right.

### Desktop app

The **desktop app** runs the same managed OIDC sign-in — no `kubelogin` needed
there either. Instead of the web callback, it uses a loopback redirect
`http://127.0.0.1:<port>/auth/cluster/callback` (RFC 8252, like `kubelogin`),
opening your system browser and capturing the code on a one-shot local
listener. Tokens are sealed at rest under a local key in the app config
directory. The cluster's IdP client must permit loopback redirects — most allow
any `http://127.0.0.1:*` port for a public client; if yours requires an exact
URI, pin a fixed port with `SRELENS_CLUSTER_LOGIN_PORT` and register
`http://127.0.0.1:<that-port>/auth/cluster/callback`. Non-OIDC exec plugins
(aws, gke-gcloud-auth-plugin) still run natively on desktop.

### Limitations

- The managed OIDC token authenticates **kube-rs API calls only** — which
  includes the web in-pod **exec** terminal (it streams through the API). But
  **helm** runs a `helm` subprocess that can't use it (kubectl ≥ 1.26 removed
  the built-in `auth-provider: oidc` support), so helm doesn't work against an
  OIDC-only cluster. The desktop **host shell** shares this subprocess limit.
- **Private-CA identity providers** — an IdP served under a private root the
  container doesn't trust — are not yet supported (discovery uses the system
  trust store).
- This covers OIDC only. Non-OIDC exec plugins (`aws eks get-token`, `gcloud`)
  still require [extending the image](#extending-the-image-with-cloud-clis).

## Web-mode behavior

- **Terminal:** web users get an RBAC-scoped **in-pod exec** terminal — the
  same model Headlamp uses: srelens opens an interactive session *inside a
  pod/container* over the Kubernetes API, authorized by the user's own cluster
  credentials. The **host shell** (`bash` inside the srelens container, the
  `kubectl · <ctx>` tab on desktop) is **desktop-only** and is refused on the
  web surface: on a shared multi-user server a container-host shell runs as the
  single shared UID and would let any user read every other user's materialized
  kubeconfigs and sealed tokens. `POST /api/command/start_terminal` returns
  `403` in web mode; `start_pod_exec` stays available.
- **Port-forwarding:** a forwarded port binds to loopback inside the container;
  the browser reaches it through an in-app link that proxies through the
  server at `/pf/{id}/…`, authenticated by your session and scoped to forwards
  you own. Only plain HTTP and WebSocket upgrades are proxied this way —
  **HTTPS upstreams and raw TCP forwards are not supported** in web mode (the
  desktop app can still do arbitrary TCP forwarding locally).
- **Toolbox installs and helm `repo`/`plugin` operations are disabled on the
  web surface.** Installing `kubectl`/`helm`/`krew` plugins, `helm repo
  add`/`update`, and `helm plugin install` either write process-wide state or,
  in the case of `helm plugin install <url>`, download and execute code — so on
  a shared container one user's action would reach every other user (or run as
  the shared UID). These are rejected at **both** API layers: the capability
  surface (`WEB_DENIED_CAPABILITIES`) and the streaming command surface
  (`start_helm_op` refuses a `repo`/`plugin` subcommand), regardless of what
  the UI shows. Read-only toolbox operations (status, diagnose, search plugins)
  and the rest of helm (install, upgrade, rollback, uninstall, template,
  search, list, get) remain available. Each user's helm invocation also runs
  against a **private helm home** (`HELM_CONFIG_HOME`/`HELM_CACHE_HOME`/
  `HELM_DATA_HOME` under their runtime dir), so even allowed operations never
  share repository config, cache, or plugins across users.

## Extending the image with cloud CLIs

The shipped image bundles `kubectl` and `helm` but not cloud-provider CLIs.
If your kubeconfigs use an exec plugin (`aws eks get-token`, `gcloud
container clusters get-credentials`-style auth, etc.), extend the image:

```dockerfile
FROM ghcr.io/srelens/srelens

USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
      awscli \
    && rm -rf /var/lib/apt/lists/*
# or install gcloud, azure-cli, etc. following each vendor's Debian instructions
USER srelens
```

Build and run your extended image the same way as the base image — it still
listens on `:8080`, expects `/data` as its state volume, and honors the same
environment variables.
