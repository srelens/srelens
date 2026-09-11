# Extension platform

Tracking: [#163](https://github.com/srelens/srelens/issues/163). Architecture:
[plugin ADR](design/plugin-architecture.md).

The platform has two authoring paths over one host:

- **Native srelens extensions:** a versioned manifest, brokered capabilities and
  UI contributions. Native extensions must not depend on Lens APIs.
- **Freelens/OpenLens extensions:** a compatibility adapter maps their lifecycle,
  APIs and UI registrations to the native layer. This is a required platform
  feature, not a replacement for native authoring.

## What is implemented

`crates/plugin-host` provides the native API 0.1 contract and a runnable
**declarative host prototype**. A manifest binds existing host capabilities to
extension-specific operations and declares pages, detail tabs and row actions.
The broker validates explicit grants, fixes resource arguments, rejects caller
overrides, derives the public input schema and inherits host consent annotations.
It registers operations under `plugin/<extension-id>/<operation>` in the shared
capability registry. MCP discovers and gates those operations through its existing
request path.

Registration is all-or-nothing. Unregistering removes operations from the mutable
registry and revokes their handlers in older snapshots. Calls already admitted
may finish. A host must rebuild its MCP snapshot after a lifecycle change to
refresh discovery; an older snapshot may still list a revoked tool, but cannot
execute it. Do not advertise live tool-list updates until that lifecycle wiring
exists.

Both desktop designs now load local declarative manifests through **Settings →
Extensions**. The backend owns installation, grants, enable/disable, updates,
removal and per-extension JSON settings. Developer mode is off by default;
unsigned installs require enabling it and reviewing the requested capability
before granting permission. An unsigned-extension notice remains visible while
any extension is enabled. Turning developer mode off disables all extensions.

The app deliberately accepts a narrower surface than the developer broker:
`k8s.listCustomResource` bindings with fixed, nonempty group/version/plural/kind
and fixed resource scope, plus explicitly granted `k8s.listEvents` readers. Only `context` and `namespace` are forwarded from the
host view. Core-group resources, caller-supplied resource selectors, arbitrary executable
entry points and operations requiring consent are rejected by the declarative path. Reads remain subject
to the selected cluster's RBAC; the extension receives no kubeconfig or token.

The inventory lives next to the desktop settings file, using the settings path
with its extension replaced by `extensions.json`. Saves use a private temporary
file, sync and atomic replacement under a cross-process lock. Updating an ID
preserves its settings and assigns a new revision. Every read checks the durable
inventory and revision, so disabled/removed/replaced installations cannot be
invoked through an old registry instance. Already admitted reads may finish.
No preference or installation is persisted in browser storage. Stored extension
settings are JSON data; this declarative version does not interpolate settings
into capability arguments.

The application exposes `extensions.list`, `extensions.configure` and
`extensions.read`, plus the read-only `extensions.freelensRead` compatibility broker, through the shared capability registry and MCP. Configure is
mutating and uses the normal MCP consent gate. App-installed operations currently
use the `extensions.read` facade with installation ID, revision, operation and
context; individual `plugin/...` tool discovery remains a developer-harness
feature. The app facade refuses a host reader with stronger consent annotations.
All four capabilities are unavailable on the multi-user web host until per-user
extension state is implemented.

## Run the Freelens FluxCD archive

Both desktop designs can import the **original @freelensapp/fluxcd-extension
5.3.1 release archive** from
[the upstream release](https://github.com/freelensapp/freelens-fluxcd-extension/releases/tag/v5.3.1).

1. Enable developer mode in **Settings → Extensions**.
2. Choose `freelensapp-fluxcd-extension-5.3.1.tgz` in the archive picker.
3. Review and grant `freelens.flux.read`, then open **FluxCD (Freelens)** for a
   connected cluster. Existing native Flux manifests remain separate installs.
4. Use the extension navigation for its dashboard and resource pages. Click a
   resource name to open its registered detail components; **Object** shows the
   full custom resource, and Escape closes the panel.

This executes the package's actual CommonJS renderer and React/MobX components;
it does not translate the tarball into a native table manifest. The host supplies
compatible list, chart, navigation, store and detail primitives using an isolated
React 17 runtime. Native app React 19 and native extension authoring remain separate.
Duplicate upstream menu/detail registrations are deduplicated. Resource stores
resolve by declared API version, kind and plural rather than trusting a conflicting
`apiBase` (the upstream HelmChart v1 declaration names the HelmRepository path).

This first compatibility target is deliberately version-specific. The backend
verifies SHA-256
`27b433c2738e6228cd06c79fe98d0141101180679474dd6e81a56f12aacb4ddf`
before reading the bounded archive. Other releases, repacked files and unrelated
Freelens/OpenLens extensions fail with an explicit unsupported-package error.
No archive is extracted to disk; npm installation, lifecycle scripts and the
package's Node main process are not run. Additional packages need a compatibility
and permission audit, not just a new filename in the picker.

The renderer runs in an `allow-scripts`-only iframe with an opaque origin. Its CSP
blocks direct network reads, forms and external assets; the parent permits only
blob frames. Messages are accepted only from the mounted frame and expose only
resource/event reads. The host supplies the pinned context and installation
revision, and the backend rechecks the durable grant and served Flux CRDs for
reads. The renderer receives no kubeconfig, token, Tauri API or filesystem API.
Reads refresh every 30 seconds while a page is mounted. Discovery/RBAC failures
remain errors; an empty discovery result is reported separately.

**Current limits:** read-only compatibility. Reconcile, suspend/resume, edit,
delete and core-resource/Secret reads are not exposed. Core-resource references
are displayed as text; use the host browser for those resources. Marketplace,
signing, generic Node/Electron execution and arbitrary third-party packages remain
future work. Developer mode and the archive integrity allowlist are required.

## Try a native extension

In either desktop design:

1. Open **Settings → Extensions** and enable developer mode.
2. Paste `examples/extensions/argocd.json` or `flux.json`, review the manifest,
   then install and grant `k8s.listCustomResource` (Flux also requests
   `k8s.listEvents` for its dashboard).
3. Choose a cluster and open an extension page. The new design also adds pages
   beneath **Extensions → extension name** in the cluster sidebar, with nested
   page groups; its routes pin the cluster.
   Classic opens pages inline in the manager with its own controls and theme.
4. Open a Namespace's resource overview. Its **Extensions** section contains the
   declared detail view and an **Extension actions** menu, scoped to that namespace.
5. Disable/remove the extension to remove its contributions, or install the same
   ID again to update it. Open views refresh against the new revision. JSON
   settings are preserved across updates and restarts, and deleted on removal.

Installation and inventory discovery do not contact clusters. Page reads happen
when opened; namespace detail contributions read only the selected resource's
cluster and namespace. Refresh explicitly repeats a read. These are read-only
lists; native Sync/Reconcile actions and arbitrary custom renderers are not part
of this stage.

The examples use the existing CRD reader. They do not install CRDs or connect to
any cluster merely by validating the manifest.

```sh
cargo run -p srelens-plugin-host --example extension_host -- \
  examples/extensions/argocd.json --grant=k8s.listCustomResource

cargo run -p srelens-plugin-host --example extension_host -- \
  examples/extensions/flux.json --grant=k8s.listCustomResource

# Generate the JSON schema used for authoring and validation.
cargo run -p srelens-plugin-host --example extension_host -- --schema
```

Argo CD binds `argoproj.io/v1alpha1` Applications and exposes health/sync columns.
Flux binds `kustomize.toolkit.fluxcd.io/v1` Kustomizations and
`helm.toolkit.fluxcd.io/v2` HelmReleases, including Ready status. The cluster must
serve those versions. RBAC/discovery errors remain errors, never empty success.

To expose only the example's operations to an MCP client, add `--mcp`. Supply
`context` explicitly on every call, and optionally `namespace`; omitting namespace
lists across namespaces. For example, a `tools/call` request is:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"plugin/org.srelens.argocd/applications","arguments":{"context":"staging","namespace":"argocd"}}}
```

The explicit `--grant` flag is a developer-harness grant for the named host
capability, not an application install-consent flow. The harness uses the normal
kubeconfig sources and never supplies credentials to extension code. Its default
MCP policy denies mutating and sensitive calls. Do not use `Registry::invoke` or
`McpServer::call_tool` as a substitute for the MCP request handler's consent gate.

## Authoring contract

See [argocd.json](../examples/extensions/argocd.json) and
[flux.json](../examples/extensions/flux.json) for complete manifests.

- `id`: reverse-domain identifier; `version`: SemVer; `srelensApiVersion`: a
  compatible range for the extension API, independently versioned from the app.
- `kind`: `declarative` in this prototype.
- `permissions`: exact host capability IDs used by bindings. The host supplies
  grants separately; manifest declarations are not self-authorization.
- `capabilities`: local name/title, trusted-host `target`, fixed `arguments`, and
  allowed caller `inputs`. Every required target argument must be supplied by one
  of those sources. Their keys cannot overlap. Target handlers validate values.
- `contributions`: `pages`, `detailTabs`, `rowActions`, each referencing a declared
  local capability. Detail/action `forKinds` values include the API group to avoid
  confusing CRDs with equally named built-ins. Use `/Pod` for the core API
  group and `argoproj.io/Application` for an Argo CD custom kind.

The author cannot supply a handler, JavaScript, schema or safety annotations.
Annotations come from the trusted core; mutations, destructive operations and
sensitive reads cannot lose their confirmation requirement. Fixed arguments are
excluded from the public input schema and rejected if a caller tries to override
them. Only host capabilities can be targets; plugin-to-plugin chaining is not
supported in API 0.1.

## Delivery plan and exit checks

The current PR includes the broker and local declarative app lifecycle. #163 stays open until both authoring paths
work end-to-end.

1. **Native contract and broker (implemented):** executable manifest examples,
   collision/permission/input validation, revocation and real MCP consent tests.
2. **Application lifecycle and declarative UI (local desktop implemented):** backend-owned install inventory,
   grants and per-extension settings; atomic save/update/remove; developer mode
   off by default for unsigned local manifests; signed distributed packages remain pending;
   enable/disable and contribution removal in both classic and new UI. One sample
   must visibly add a page, detail tab and menu action without editing app source.
   Cluster identity belongs in extension routes and broker calls. Web instances
   must use per-user state, never a process-global extension inventory.
3. **Native executable and renderer SDKs:** supervised JSON-RPC sidecars and
   sandboxed iframe bridge; managed runtime, quotas, cancellation and teardown;
   deny ambient network/filesystem/process access. Unsupported OS sandbox backends
   must refuse executable extensions. Persist settings only through the backend.
4. **Freelens/OpenLens adapter:** package API aliases, main lifecycle, React/MobX
   iframe runtime, Kubernetes stores/watch adapters and registration translation.
   Rebuild and run the real Flux extension first, then ArgoCD. Unsupported APIs
   must fail with the API name and migration guidance; no silent stub success.
5. **GitOps workflows and distribution:** native ArgoCD/Flux pages and detail
   panels, confirm-gated Sync/Refresh/Reconcile, reference Trivy integration,
   signed update verification, permission-diff consent, marketplace and revocation.

The sequence refines the ADR: the declarative broker can be proven without
executing untrusted code, but neither executable runtime is released before
sandboxing and trust verification. Native and compatibility runtimes share grants,
registry, contribution lifecycle and consent; they do not create parallel stores.

## Freelens/OpenLens compatibility matrix

The following is a **porting plan**, not a supported-runtime claim. As of this
foundation, existing extension packages cannot run in srelens.

Source audit on 2026-09-11:
[Flux package](https://github.com/freelensapp/freelens-fluxcd-extension/blob/e76a2f11a77add08dd9ed0ace02d9f2f59bdd58d/package.json),
[Flux renderer](https://github.com/freelensapp/freelens-fluxcd-extension/blob/e76a2f11a77add08dd9ed0ace02d9f2f59bdd58d/src/renderer/index.tsx),
[ArgoCD package](https://github.com/Sebastian-Prokesch/freelens-argocd-extension/blob/e41c3fed658472b239bf6c49d6d5cc6060cd7522/package.json),
[ArgoCD renderer](https://github.com/Sebastian-Prokesch/freelens-argocd-extension/blob/e41c3fed658472b239bf6c49d6d5cc6060cd7522/src/renderer/index.tsx).
Both audited projects expose separate main/renderer entries and React 17/MobX
renderer dependencies. Srelens uses a Tauri webview; those bundles cannot be loaded
as host-renderer modules or assumed to share its React instance.

| Extension surface | Native destination | Compatibility work still required |
| --- | --- | --- |
| `Main.LensExtension`, `Renderer.LensExtension`, activation/disposal | Supervised instance lifecycle | Node host + iframe adapter, cleanup on failure/disable |
| `@freelensapp/extensions`, legacy `@k8slens/extensions` | `@srelens/lens-compat` API aliases | Versioned export mappings and actionable unsupported errors |
| Cluster pages and menus | Page contributions and cluster-scoped routes | Translate registrations and mount isolated React components |
| Kubernetes detail/menu registrations | Detail tabs and row actions | Match group/kind; pin resource identity and enforce consent |
| `KubeObject`, `KubeApi`, stores and watches | Brokered CRD capabilities/streams | Store semantics, reconnect, cancellation and disposal |
| React 17, MobX, renderer styling | Isolated iframe runtime | Bundle dependencies there; adapt theme tokens without host CSS access |
| `clusterFrameComponents` / dialogs | Isolated overlay contribution | Focus, keyboard dismissal and bridge ownership |
| Extension preferences/store | Backend-owned per-extension settings | Serialization, scoped keys, migration and observable adapter |
| Node filesystem/network/process/Electron APIs | Explicit broker methods | Deny ambient access; add allow-listed adapters case by case |

Binary/drop-in compatibility with arbitrary historical Lens packages is not
promised. The initial acceptance target is rebuilding representative supported
extensions against the compatibility shim, then demonstrating their pages, resource
reads, settings and guarded mutations inside the app. OpenLens API coverage must
be tested against real legacy imports rather than inferred from Freelens branding.

## Native dashboard and navigation contributions

The Flux 0.2.0 example includes Overview, Kustomizations, Helm releases, Sources
(Git repositories, Helm repositories, Helm charts, Buckets, OCI repositories),
Image Automation (repositories, policies, update automations), and Notifications
(alerts, providers, receivers). Both desktop designs render the same workspace
using their own controls. Namespace and search filters are scoped to the open
cluster; they are temporary view state, not persisted preferences.

Pages may declare `group` to nest their navigation. Resource pages can declare
`statusColumns` with zero-based `ready`, optional `suspended`, and optional
`progressing` printer-column indices. Suspended takes precedence over progressing,
which takes precedence over Ready. Missing/unknown Ready conditions remain Unknown.
A `dashboard` declares `pages` referencing resource pages with status columns,
and optional `events: { capability, apiGroups }`. Event summaries filter by the
involved object's API group, so an unrelated kind with the same name is excluded.
Failed reads retain their error and retry; they never become zero-count summaries.
The events section uses the workspace namespace and search controls; dashboard
counts reflect the namespace and are not changed by event search.

Install the updated Flux example again to upgrade an existing installation and
review its new event-read grant. The application does not silently replace an
installed manifest or expand its grants. Flux controllers/CRDs must already exist
on the selected cluster. API versions are declared by the manifest; unsupported
versions produce an explicit error, not a claim that the cluster has no Flux.

These are native declarative views, not execution of the Freelens extension's
React bundle. Reconcile/suspend writes, arbitrary custom renderer code and the
Freelens runtime remain separate platform work.
