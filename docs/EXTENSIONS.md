# Extension platform

Tracking: [#163](https://github.com/srelens/srelens/issues/163). Architecture:
[plugin ADR](design/plugin-architecture.md).

The platform supports **native srelens extensions**: a versioned manifest,
brokered capabilities and UI contributions rendered with the app's components.
Freelens/OpenLens packages and their JavaScript runtimes are not supported.

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

Installation and enablement are app-wide, not per kubeconfig context. Enabled
pages remain available across clusters. When a page opens, the host checks the
CRDs and served versions required by that page (including dashboard summaries).
A missing CRD/version produces a requirements page with the exact API names and
a **Check again** action; it does not uninstall or disable the extension. CRD
discovery failures are reported as unverifiable requirements and do not prevent
resource reads allowed by the user's existing RBAC. Switching clusters never
reuses another cluster's discovery result.

The app deliberately accepts a narrower surface than the developer broker:
`k8s.listCustomResource` bindings with fixed, nonempty group/version/plural/kind
and fixed resource scope, plus explicitly granted `k8s.listEvents` readers. Only `context` and `namespace` are forwarded from the
host view. Core-group resources, caller-supplied resource selectors, executable
entry points and operations requiring consent are rejected. Reads remain subject
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
`extensions.read` through the shared capability registry and MCP. Configure is
mutating and uses the normal MCP consent gate. App-installed operations currently
use the `extensions.read` facade with installation ID, revision, operation and
context; individual `plugin/...` tool discovery remains a developer-harness
feature. The app facade refuses a host reader with stronger consent annotations.
All extension capabilities are unavailable on the multi-user web host until per-user
extension state is implemented.

No third-party code, subprocess, iframe, npm install or lifecycle script
is executed. Catalog downloads contain JSON data only. Only native srelens manifests are accepted. Signed distribution,
executable runtimes and sandboxing remain future work; local developer-mode
support does not claim those protections.

## Browse the native catalog

In either desktop design, open **Settings → Extensions → Catalog**.
The **Extensions** tab lists installed extensions. In developer mode, a collapsed
**Install a local manifest** section exposes JSON installation tools. The
**Catalog** tab loads discovery on first opening and keeps its search/list state
when switching tabs.

The catalog comes from [srelens/extensions](https://github.com/srelens/extensions);
each entry points to its own repository and versioned GitHub release asset.
Flux and Argo CD use locally bundled project logos in Settings and navigation.
Other native extensions receive a name-based initials mark. Page icons follow
their role (overview, sources, Helm, notifications) rather than repeating the
extension glyph. Logos identify integrations and do not indicate trust/signing.

The initial entries are [Flux](https://github.com/srelens/extension-flux) and
[Argo CD](https://github.com/srelens/extension-argocd). Search by name, ID or description.

The backend caches validated metadata for 24 hours alongside the inventory in
`*.extensions.catalog.json`. **Refresh catalog** checks immediately. A failed
refresh retains the cache and displays the failure and original timestamp.
Catalog browsing does not connect clusters or install extensions.

**Review installation** downloads a size-bounded manifest over HTTPS, checks its
SHA-256 against the selected catalog release, verifies ID/version/API identity,
and validates the app's read-only capability contract. The exact verified bytes
then go through the existing permission review and install action. A catalog
change invalidates the selected checksum and requires a new review. Replacing
an installed ID is explicit and preserves its settings. There are no automatic
updates or downgrade decisions.

API-incompatible releases stay visible but cannot be installed. Preview labels
come from catalog metadata. `testedHost.revision` records test provenance, not an
exact-build restriction. These manifests remain unsigned and require developer
mode; a checksum is not a publisher signature.

`extensions.catalog` and `extensions.catalogManifest` are read-only capabilities.
Both are refused on the web host. Downloads accept only the fixed catalog URL,
GitHub release assets and GitHub's release-asset redirects, with bounded sizes,
timeouts and redirect counts. The frontend never fetches catalog URLs or writes
catalog caches to browser storage.

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
  examples/extensions/flux.json --grant=k8s.listCustomResource --grant=k8s.listEvents

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

The current PR includes the broker and local declarative app lifecycle. #163 stays open until the native platform delivery milestones are complete.

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
4. **GitOps workflows and distribution:** native ArgoCD/Flux pages and detail
   panels, confirm-gated Sync/Refresh/Reconcile, reference Trivy integration,
   signed update verification, permission-diff consent, marketplace and revocation.

The declarative broker is implemented before executable runtimes. Executable
extensions remain future work until sandboxing and trust verification exist.

## Upgrading from the retired compatibility prototype

Existing archive installations are excluded when the backend reads the inventory;
native installations, permissions, revisions and settings remain intact. The next
successful inventory change removes retired entries from the saved file. Archive
installation and the compatibility broker are no longer available. Install the
native Flux or Argo CD JSON example through Settings → Extensions instead.

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

Reconcile/suspend writes and arbitrary custom renderer code remain future native
platform work.
