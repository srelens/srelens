# Extensions

srelens **apps** (extensions) add pages, dashboards, resource details and actions for
Kubernetes projects such as Flux and Argo CD, without changing srelens itself. An app
is a versioned JSON manifest. srelens renders everything it contributes with its own
components and brokers every cluster call it makes.

Tracking: [#163](https://github.com/srelens/srelens/issues/163). Architecture decisions:
[plugin ADR](../design/plugin-architecture.md).

## What an app can do

- Add navigation pages that list custom resources, optionally grouped, with columns,
  status and dashboards.
- Add detail tabs and menu entries to resource views.
- Get host-rendered resource inspection and, for supported Flux and Argo CD kinds,
  confirmed GitOps actions.

## What an app cannot do

- Run code. No JavaScript, subprocess, iframe, npm install or lifecycle script is
  executed.
- Read kubeconfig, tokens, files or the network. Every read goes through the host,
  under the selected cluster's RBAC.
- Write to the cluster, except through the host's own confirmed actions.

Freelens and OpenLens packages are not supported.

## Pages in this directory

| Page | Covers |
|---|---|
| [specification.md](specification.md) | The normative contract: versioning, compatibility, deprecation, identifiers, API changelog |
| [architecture.md](architecture.md) | Broker, app lifecycle, inventory, quarantine, where it is heading |
| [manifest.md](manifest.md) | Manifest fields, limits and validation rules |
| [permissions.md](permissions.md) | Permissions, grants, what an app may read, RBAC and consent |
| [ui-contributions.md](ui-contributions.md) | Pages, dashboards, detail tabs, requirement checks, resource inspection |
| [capabilities.md](capabilities.md) | The `extensions.*` capabilities, MCP, host GitOps actions |
| [streams.md](streams.md) | The generic stream contract: frames, view ownership, limits, metrics |
| [security.md](security.md) | Trust boundary and what is not yet protected |
| [threat-model.md](threat-model.md) | Assets, adversaries, mitigations with their code, residual risk and open work |
| [distribution.md](distribution.md) | Catalog, signed releases, local installation |
| [testing.md](testing.md) | Developer harness and the test suites |
| [migration.md](migration.md) | Upgrading, downgrading and moving between API versions |

## Try an app

On the desktop, the quickest path is **Settings → Apps → Catalog** → Flux or Argo CD →
**Review installation**. Apps are not yet available on the web host
([#515](https://github.com/srelens/srelens/issues/515)).

To exercise the local installer instead:

1. Copy `examples/extensions/argocd.json` or `flux.json` and change its `id` to one
   outside the reserved namespace, for example `org.example.argocd`. IDs under
   `org.srelens.` install only as signed releases.
2. Open **Settings → Apps → Install a local manifest**, paste it, review the
   manifest, then install and grant `k8s.listCustomResource`. Flux also requests
   `k8s.listEvents` for its dashboard.
3. Open pages beneath **Apps → app name** in the connected cluster's sidebar. The
   classic design opens separate app tabs; both designs pin the cluster. Settings has
   no cluster selector or page launcher; it manages app-wide installation only.
4. Open a Namespace's resource overview. Its **Apps** section contains the declared
   detail view and an **App links** menu, scoped to that namespace.
5. Disable or remove the app to remove its contributions, or install the same ID
   again to update it. Open views refresh against the new revision. Settings the new version still declares and accepts are
   preserved across updates and restarts, and deleted on removal.

Installation and inventory discovery do not contact clusters. Page reads happen when
a page opens, and namespace detail contributions read only the selected resource's
cluster and namespace.
