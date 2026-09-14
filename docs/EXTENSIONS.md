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

Both desktop designs load declarative manifests through **Settings → Apps**, from
the catalog or as a pasted local manifest. The backend owns installation, grants,
enable/disable, updates, removal and per-extension JSON settings. Installation
requires explicit review and grants for the requested capabilities. There is no
developer mode. Legacy inventories retain settings and revisions, and previously
disabled extensions remain disabled.

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
and fixed resource scope, plus explicitly granted `k8s.listEvents` readers. Only
`context` and `namespace` are forwarded from the host view. Core-group resources,
caller-supplied resource selectors, executable entry points and operations
requiring consent are rejected. Reads remain subject to the selected cluster's
RBAC; the extension receives no kubeconfig or token.

The inventory lives next to the desktop settings file, using the settings path
with its extension replaced by `extensions.json`. Saves use a private temporary
file, sync and atomic replacement under a cross-process lock. Updating an ID
preserves its settings and assigns a new revision. Every read checks the durable
inventory and revision, so disabled/removed/replaced installations cannot be
invoked through an old registry instance. Already admitted reads may finish.
No preference or installation is persisted in browser storage. Stored extension
settings are JSON data; this declarative version does not interpolate settings
into capability arguments.

Every load re-verifies each installed app: its manifest against this host, and a
signed app's stored proof against the trusted publisher table. An app that fails
is **quarantined on its own**. It loads disabled, its reason is shown in Settings →
Apps, and it cannot be re-enabled until it is reinstalled or removed. Every other
app keeps working. This covers a rotated or withdrawn signing key, a modified
proof or manifest, and an API version this host no longer supports. The reason is
recomputed on each load and never written to the inventory. A corrupt file or
duplicate app IDs still fail the whole inventory, because no single entry can be
trusted then.

The application exposes these capabilities through the shared registry and MCP:

- `extensions.list`, `extensions.read`, `extensions.resource`,
  `extensions.catalog` and `extensions.catalogManifest`: read-only.
- `extensions.configure` (install, enable, remove, settings) and
  `extensions.action` (host GitOps actions): mutating, behind the normal MCP
  consent gate.

App-installed operations use the `extensions.read` facade with installation ID,
revision, operation and context; individual `plugin/...` tool discovery remains a
developer-harness feature. The app facade refuses a host reader with stronger
consent annotations. All `extensions.*` capabilities are unavailable on the
multi-user web host until per-user extension state is implemented
([#515](https://github.com/srelens/srelens/issues/515)).

The host GitOps write `k8s.gitOpsAction` is refused on the web as well. There, no
installed app scopes it to a resource and there is no consent prompt.
`k8s.getCustomResource` stays available on the web, because it is a read under the
user's own kubeconfig and RBAC, like every other custom-resource read.

No third-party code, subprocess, iframe, npm install or lifecycle script is
executed. Catalog downloads contain JSON data only. Official catalog releases are
signature-verified (below). Third-party publisher signing, key rotation,
revocation, executable runtimes and sandboxing are future work
([#519](https://github.com/srelens/srelens/issues/519),
[#521](https://github.com/srelens/srelens/issues/521)); declarative support does
not claim those protections.

## Browse the native catalog

In either desktop design, open **Settings → Apps → Catalog**.
The **Apps** tab lists installed extensions. A collapsed
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

Catalog metadata is additive. Hosts ignore catalog fields they don't recognize,
so the catalog can gain fields such as publishers, categories or revocations
without released hosts rejecting it. Known fields are still validated strictly,
and a breaking change bumps `schemaVersion`, which older hosts refuse. Manifests
stay strict: an unknown manifest field is an error.

**Review installation** downloads a size-bounded manifest over HTTPS, checks its
SHA-256 against the selected catalog release, verifies ID/version/API identity,
and validates the app's read-only capability contract. The exact verified bytes
then go through the existing permission review and install action. A catalog
change invalidates the selected checksum and requires a new review. Replacing
an installed ID is explicit and preserves its settings. There are no automatic
updates or downgrade decisions.

API-incompatible releases stay visible but cannot be installed. Preview labels
come from catalog metadata. `testedHost.revision` records test provenance, not an
exact-build restriction.

### Signed official releases

Official Flux and Argo CD releases carry a detached Ed25519 signature
(`manifest.json.sig`) over the exact manifest bytes. The host pins a
**trusted-publisher table** in `crates/registry/src/extensions/signing.rs`. For
each publisher it holds:

- the public key
- the app ID namespace reserved for it (`org.srelens.` for srelens)
- the only repository each of its apps may be released from

Catalog metadata cannot supply a trusted key.

A catalog entry that names a reserved ID *or* a trusted publisher's repository
must be signed. Missing signatures, modified bytes and repository substitution are
rejected before review. Installation re-verifies the proof, persists it, and
checks it against the installed manifest on every load; see quarantine above.
Permission review is still required. A checksum alone is not a publisher signature.

IDs in a reserved namespace install **only** with that publisher's signature:

- A pasted manifest cannot use an `org.srelens.` ID.
- A pasted manifest cannot replace a signed installation.

This stops a local manifest from taking an official app's ID and logo while
differing from it only by a label. Apps with IDs outside reserved namespaces
install unsigned and are labelled **Unsigned local**. Unsigned `org.srelens.`
apps installed before this rule keep working, still labelled unsigned. Reinstall
them from the catalog to get the signed release.

Release workflows in both app repositories require `APP_SIGNING_PRIVATE_KEY`
(PKCS#8 Ed25519 PEM) in GitHub Actions secrets, check it against
`signing-public.pem`, and publish a 64-byte binary signature. The private key must
never be committed. The current public key is stored as raw 32 bytes in
`crates/registry/src/extensions/srelens-apps.pub`. Key rotation still requires a
host update that trusts the new key before new release signatures are published.
A host that stops trusting a stored signature quarantines only that app.

`extensions.catalog` and `extensions.catalogManifest` are read-only capabilities.
Both are refused on the web host. Downloads accept only the fixed catalog URL,
GitHub release assets and GitHub's release-asset redirects, with bounded sizes,
timeouts and redirect counts. The frontend never fetches catalog URLs or writes
catalog caches to browser storage.

## Try a native extension

In either desktop design, the quickest path is **Settings → Apps → Catalog** →
Flux or Argo CD → **Review installation**. To exercise the local installer
instead:

1. Copy `examples/extensions/argocd.json` or `flux.json` and change its `id` to
   one outside the reserved namespace, for example `org.example.argocd`.
   (`org.srelens.*` IDs install only as signed releases.)
2. Open **Settings → Apps → Install a local manifest**, paste it, review the
   manifest, then install and grant `k8s.listCustomResource` (Flux also requests
   `k8s.listEvents` for its dashboard).
3. Open pages beneath **Apps → app name** in the connected cluster’s
   sidebar in either desktop design. Classic opens separate app tabs; both
   designs pin the cluster. Settings has no cluster selector or page launcher; it
   manages app-wide installation only.
4. Open a Namespace's resource overview. Its **Apps** section contains the
   declared detail view and an **App actions** menu, scoped to that namespace.
5. Disable/remove the extension to remove its contributions, or install the same
   ID again to update it. Open views refresh against the new revision. JSON
   settings are preserved across updates and restarts, and deleted on removal.

Installation and inventory discovery do not contact clusters. Page reads happen
when opened; namespace detail contributions read only the selected resource's
cluster and namespace. Refresh explicitly repeats a read. Readers remain
declarative and read-only. The host supplies resource inspection and explicitly
confirmed GitOps actions, described below; arbitrary custom renderers and
extension-defined write forwarding remain unsupported.

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

The developer harness does not install anything, so it accepts the example IDs
unchanged.

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

- `id`: reverse-domain identifier. IDs under `org.srelens.` are reserved for
  signed srelens releases. `version`: SemVer. `srelensApiVersion`: a compatible
  range for the extension API, versioned independently of the app.
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

## Extension API changelog

The manifest contract is still API **0.1**. The entries below are host behaviour
added without a manifest change, so existing `^0.1` manifests keep installing.
The versioning policy for future API bumps is tracked in
[#530](https://github.com/srelens/srelens/issues/530).

- **#508:** API 0.1 manifests.
  - Contributions: `pages` (with `group`, `statusColumns`, `dashboard`), `detailTabs`, `rowActions`.
  - Readers: `k8s.listCustomResource` and `k8s.listEvents`.
  - Backend-owned inventory.
- **#511:**
  - Catalog installation, with signature verification for official releases.
  - Resource inspection through `extensions.resource`.
  - Host-owned, confirmed Flux and Argo CD actions for matching kinds through `extensions.action`.
  - App resource tables prefer the CRD's `additionalPrinterColumns` for the served version over manifest `printerColumns`, which remain the fallback when discovery fails. Dashboard status columns still index manifest `printerColumns`.
  - Developer mode removed.
- **#528:**
  - IDs under `org.srelens.` reserved for signed releases.
  - An app that fails re-verification is quarantined individually.
  - Catalog metadata tolerates unknown fields.
- **#529:**
  - GitOps actions are offered only for the API versions listed under
    [host actions](#resource-inspection-and-host-actions).
  - A Suspend of a suspended resource, or a Resume of one that is not suspended, is refused.
  - Resource events are newest first across every page read (up to 5,000), ranking a recurring series by its latest occurrence, and capped at 100. `eventsTruncated` says when older ones were left out, and `eventsPartial` says when pages remained unread.
  - An accepted action refreshes every open list, dashboard and detail view of that resource.
  - `k8s.gitOpsAction` is refused on the web host.

**Downgrading.** Inventory changes are one-way. A host older than #511 cannot read an
inventory written by #511 or later, because `developerMode` was removed and
`signatureProof` added, and older hosts reject both. After such a downgrade,
Settings → Apps reports the inventory as unreadable. Upgrade again, or move
`settings.extensions.json` aside to start with no apps.

## Delivery plan

The platform plan, its decisions and milestones are tracked in
[#163](https://github.com/srelens/srelens/issues/163) and its child epics. In
brief:

1. **Shipped:** the native contract and broker, the local declarative app
   lifecycle in both designs, the signed official catalog, resource inspection and
   host-owned GitOps actions.
2. **Foundation (API 0.2):** specification and versioning policy, committed JSON
   Schema, structured validation errors, lifecycle gaps
   ([#516](https://github.com/srelens/srelens/issues/516)).
3. **Declarative UI contributions:** table columns, detail panels, dashboard cards,
   status resolvers and badges, typed settings, commands
   ([#517](https://github.com/srelens/srelens/issues/517)).
4. **Declared, host-enforced actions**, replacing the built-in Flux/Argo CD action
   list ([#518](https://github.com/srelens/srelens/issues/518)), alongside
   third-party signing, rotation and revocation
   ([#519](https://github.com/srelens/srelens/issues/519)).
5. **Streaming and providers**, then a supervised, sandboxed **executable SDK**
   that refuses to run where no OS sandbox backend exists
   ([#520](https://github.com/srelens/srelens/issues/520),
   [#521](https://github.com/srelens/srelens/issues/521)).

Contributions render with host components only; no iframe or third-party renderer
code is planned.

## Upgrading from the retired compatibility prototype

Existing archive installations are excluded when the backend reads the inventory;
native installations, permissions, revisions and settings remain intact. The next
successful inventory change removes retired entries from the saved file. Archive
installation and the compatibility broker are no longer available. Install Flux or
Argo CD from **Settings → Apps → Catalog** instead.

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

Install the updated Flux release again to upgrade an existing installation and
review its new event-read grant. The application does not silently replace an
installed manifest or expand its grants. Flux controllers/CRDs must already exist
on the selected cluster. API versions are declared by the manifest; unsupported
versions produce an explicit error, not a claim that the cluster has no Flux.

Arbitrary custom renderer code is not supported. Write actions are host-owned and
described in the next section.

## Resource inspection and host actions

User-facing extension management is named **Apps**. Internal `extensions.*`
capability IDs, manifest IDs and existing routes remain stable.

Click a resource row to open its overview: metadata, spec, conditions, status,
labels/annotations, a read-only YAML manifest in the shared CodeEditor, and the
resource-UID-filtered events. Events are newest first by when they were last seen,
including the latest occurrence of a recurring event series. At most 100 are shown,
and the panel says when older ones were left out.

To choose them, the host reads up to 5,000 events (10 pages of 500). For a resource
with more than that, the panel says it shows the newest of the events it read,
rather than claiming they are the latest. Event RBAC failures are shown separately
and preserve the overview. The list stays visible beside the shared Inspector; Open
tab promotes details to an independent resource tab. Existing native manifests
need no update.

The host derives the API group, kind, plural, version and scope from the enabled
app's declared reader. `extensions.resource` and `extensions.action` reload the
backend inventory and verify its revision and grants on every request. An app
cannot rebind a reader to a write capability. These are host UI operations;
installation does not grant extension code arbitrary patch access.

Supported controls are resource-specific. They are offered only for the API versions
whose schema carries the fields they write:

- **Flux Kustomization, GitRepository, HelmRepository, HelmChart, Bucket,
  ImageRepository and ImageUpdateAutomation** (`v1`, `v1beta2`, `v1beta1`) and
  **OCIRepository** (`v1`, `v1beta2`): Suspend, Resume, Reconcile.
- **Flux HelmRelease** `v2` and `v2beta2`: Suspend, Resume, Reconcile, Force
  reconcile and Reset retries. On `v2beta1` only Suspend, Resume and Reconcile,
  because force and reset arrived with `v2beta2`.
- **Argo CD Application** (`v1alpha1`): Refresh status, Hard refresh, Sync. Sync
  does not enable pruning; configured sync options and hooks still apply.
- **Other resources and API versions** remain inspectable, without invented or
  unsupported actions.

Each write requires a review naming the pinned cluster and namespace/resource.
The backend checks UID and resourceVersion and includes both in its conditional
PATCH, rejecting stale/replaced resources. It also rejects:

- a second Argo CD sync while an operation is already present
- reconciliation while suspended
- a Suspend of a suspended resource, or a Resume of one that is not suspended
- writes to a resource being deleted

Acknowledgement says **Request accepted**, not that reconciliation completed.
Every open list, dashboard and detail view of that resource then refreshes. API
failures remain errors with no success message. Kubernetes RBAC still governs GET,
events and PATCH.

The implementation follows [Flux reconciliation and Helm actions](https://fluxcd.io/flux/components/helm/helmreleases/)
and [Argo CD operations through Kubernetes](https://argo-cd.readthedocs.io/en/stable/user-guide/sync-kubectl/).

App resource lists request `useCrdColumns: true` through `extensions.read`.
The host keeps the installed binding's API target and reads that CRD's
`additionalPrinterColumns` for the exact served version. Columns use the same
JSONPath renderer, priority filtering and Age deduplication as Custom Resources.
The response includes column definitions alongside the rendered values so headers
cannot drift from their data. If definition discovery fails, app-defined columns
remain available with an explicit error notice. Dashboard reads retain their
declared status-column mapping; signed manifests are not modified on disk.
