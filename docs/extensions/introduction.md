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
- Follow the logs of, run a fixed command in, or forward a port to the pods of a
  workload it reads, or of a namespace a person granted
  ([Logs, exec and port-forwards](manifest.md#logs-exec-and-port-forwards)).
- As an executable app (a preview, API 0.6), run a program it ships as a sandboxed sidecar
  that answers the operations it declares
  ([Executable apps](manifest.md#executable-apps)).
- Offer its readers, actions and operations to AI agents as MCP tools, under the same
  consent as the rest of srelens ([MCP.md](../MCP.md#installed-apps-tools)).

Executable apps are a preview, and so is API 0.6, which adds them, until the API is frozen
as 1.0 ([specification.md](specification.md#versioning)). Where they run:

- **Windows:** out of the box.
- **Linux:** needs the sandbox launcher `srelens-sandbox-launch`, a cgroup v2 directory
  delegated to srelens and a kernel with Landlock enabled. The bundles do not ship the
  launcher, so set up the launcher and the cgroup by hand, naming them with
  `SRELENS_SANDBOX_LAUNCHER` and `SRELENS_SANDBOX_CGROUP_ROOT`;
  [DEVELOPMENT.md](../DEVELOPMENT.md#everyday-commands) has the recipe.
- **macOS:** not yet. srelens refuses to start any sidecar until its memory and CPU
  watchdog has been checked with Seatbelt on a macOS 27 Mac.
- **The web host:** nowhere. It keeps no files for its apps, so it runs no sidecar
  ([capabilities.md](capabilities.md#web-host)).

## What an app cannot do

- Run code on this computer outside the OS sandbox. A declarative app runs no code: no
  JavaScript, subprocess, iframe, npm install or lifecycle script is executed. An
  executable app's program runs only as a sidecar in the sandbox, with no kubeconfig,
  no network and no files but its own directory, and nowhere without one. An exec
  binding runs one command the manifest fixes, inside a pod, never a shell, and only
  after a person confirms that exact command.
- Read kubeconfig, tokens or files. Every read goes through the host, under the
  selected cluster's RBAC.
- Open a network connection. An app granted `network.http` asks the host to send a
  fixed request to one of the hosts a person approved
  ([Network requests](manifest.md#network-requests)); it reaches nothing else.
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
| [inspector.md](inspector.md) | The Extension Inspector and per-app logs: levels, redaction, local-only metrics |
| [security.md](security.md) | Trust boundary and what is not yet protected |
| [threat-model.md](threat-model.md) | Assets, adversaries, mitigations with their code, residual risk and open work |
| [distribution.md](distribution.md) | Catalog, signed releases, local installation |
| [packages.md](packages.md) | The `.srelens-extension` package: layout, digest list, signature, limits, logos |
| [testing.md](testing.md) | Developer harness and the test suites |
| [migration.md](migration.md) | Upgrading, downgrading and moving between API versions |

## Try an app

The quickest path is **Settings → Apps → Catalog** → Flux or Argo CD →
**Review installation**, on the desktop or the web host. On the web the apps you
install are yours alone ([#515](https://github.com/srelens/srelens/issues/515)).

To exercise the local installer instead:

1. Copy `examples/extensions/argocd.json` or `flux.json` and change its `id` to one
   outside any publisher's namespace, for example `org.example.argocd`. IDs under
   `org.srelens.` install only as releases signed by srelens.
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
