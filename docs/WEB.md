# Running srelens as a web app

srelens can run as a multi-user web server in a single Docker container, instead
of (or alongside) the desktop app. A team deploys one container; each person
signs in with their identity provider (or a local dev login while trying it
out), uploads their own kubeconfig, and gets the full srelens experience —
resource browsing, watches, logs, pod exec, an in-container terminal, helm,
and port-forwarding — in the browser. Nothing here changes the desktop app,
which remains the primary product.

## Quick start (trial)

```sh
docker run \
  -v srelens-data:/data \
  -p 8080:8080 \
  -e SRELENS_DEV_LOGIN=you@example.com \
  ghcr.io/srelens/srelens
```

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

## Environment variables

| Variable | Required | Description |
| --- | --- | --- |
| `SRELENS_PUBLIC_URL` | Recommended | The externally reachable URL of the deployment, e.g. `https://srelens.example.com` (no trailing slash needed — it's trimmed). Defaults to `http://127.0.0.1:8080`. Session cookies are only marked `Secure` when this starts with `https://`, and it's used to build the OIDC redirect URI. |
| `SRELENS_OIDC_ISSUER` | With OIDC | The identity provider's issuer URL. Must be set together with `SRELENS_OIDC_CLIENT_ID` and `SRELENS_OIDC_CLIENT_SECRET` — all three or none. |
| `SRELENS_OIDC_CLIENT_ID` | With OIDC | OAuth client id registered with your IdP. |
| `SRELENS_OIDC_CLIENT_SECRET` | With OIDC | OAuth client secret. Never logged (redacted in debug output). |
| `SRELENS_OIDC_ALLOWED_DOMAINS` | Optional | Comma-separated list of email domains allowed to sign in (e.g. `example.com,corp.io`), case-insensitive. Leave unset to allow any authenticated email through. |
| `SRELENS_MASTER_KEY` | Recommended | 64 hex characters (32 bytes) used to encrypt kubeconfigs at rest (AES-256-GCM). If unset, a key is generated on first boot and saved to `/data/master.key` (mode 0600) — fine as long as `/data` is persisted, but an explicit key is more portable across redeploys/volume changes. |
| `SRELENS_DATA` | Optional | Path to the data directory holding the SQLite database, master key, and per-user runtime files. Defaults to `/data` in the shipped image (`./srelens-data` if unset outside the container). |
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
- **Set a stable `SRELENS_MASTER_KEY`, or make sure `/data` is persisted.**
  Kubeconfigs are encrypted at rest under this key. If it's auto-generated and
  `/data` isn't a real persistent volume, every restart loses the key and every
  stored kubeconfig becomes permanently undecryptable (users would need to
  re-upload).

## Adding kubeconfigs

There's no local filesystem to read kubeconfigs from in a browser, so each
user adds their own from **Settings → Kubernetes**: paste kubeconfig YAML
directly, or upload a file. Uploaded kubeconfigs are sealed (AES-256-GCM)
under the server's master key and scoped to that user's account — nobody else
can see or use them. Contexts from all of a user's uploaded kubeconfigs are
merged, the same way the desktop app merges multiple local kubeconfig files.

Removing an uploaded kubeconfig is supported; renaming one currently isn't —
delete and re-add it under the new name instead.

## Web-mode behavior

- **Terminal:** each user gets an in-container shell (a real PTY, `bash`),
  locked to whichever kube context they opened it against via a private
  single-context kubeconfig overlay — the same mechanism the desktop terminal
  uses. This gives users shell access inside the srelens container itself
  (to run `kubectl`/`helm` ad hoc); srelens is meant to be a trusted team tool,
  and the container runs as a non-root user.
- **Port-forwarding:** a forwarded port binds to loopback inside the container;
  the browser reaches it through an in-app link that proxies through the
  server at `/pf/{id}/…`, authenticated by your session and scoped to forwards
  you own. Only plain HTTP and WebSocket upgrades are proxied this way —
  **HTTPS upstreams and raw TCP forwards are not supported** in web mode (the
  desktop app can still do arbitrary TCP forwarding locally).
- **Toolbox and helm repo management are disabled on the web surface.**
  Installing `kubectl`/`helm`/`krew` plugins, and running `helm repo
  add`/`helm repo update`, all touch process-wide state shared by every user
  in the container (installed binaries, helm's repo list) — one user's action
  would affect everyone else's session, so these are rejected with an error at
  the API layer regardless of what the UI shows. Read-only toolbox operations
  (status, diagnose, search plugins) and the rest of helm (install, upgrade,
  rollback, uninstall, template, search, list, get — which use per-context
  temp kubeconfigs, not shared state) remain available.

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
