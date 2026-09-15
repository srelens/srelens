# Capabilities

What the host exposes for apps, through the shared capability registry and MCP.
Consent rules are in [permissions.md](permissions.md).

## `extensions.*`

| Capability | Kind | Purpose |
|---|---|---|
| `extensions.list` | Read-only | The installed apps, with revision, grants, settings, source, install time, the last three replaced versions and any quarantine reason. |
| `extensions.read` | Read-only | Run one of an installed app's declared readers, given its ID, revision, operation and context. |
| `extensions.resource` | Read-only | Inspect one resource of an enabled app, with its events and supported actions. |
| `extensions.catalog` | Read-only | Browse the catalog, from a 24-hour cache. Reports the host's supported API versions as `hostApiVersions`; the deprecated `hostApiVersion` still gives the newest. |
| `extensions.catalogManifest` | Read-only | Download and verify one catalog release for review. Does not install it. |
| `extensions.validate` | Read-only | Check a manifest, with its grants and optional signature, exactly as installing it would, and return every problem as `{code, path, message}` (see [Validation errors](specification.md#validation-errors)). Does not install it. |
| `extensions.configure` | Mutating | Install, enable, remove or configure an app. An install that fails validation is refused with the same problems. |
| `extensions.action` | Mutating | Request a host GitOps action on an app resource. |

The app facade refuses a host reader with stronger consent annotations than the
declarative contract allows. App-installed operations go through `extensions.read`;
per-app `plugin/...` tool discovery is a developer-harness feature (see
[testing.md](testing.md#developer-harness)).

## Host GitOps actions

The host derives the API group, kind, plural, version and scope from the enabled app's
declared reader. An app cannot rebind a reader to a write, and installation does not
give the app patch access: these are host operations.

Actions are offered only for the API versions whose schema carries the fields they
write:

- **Flux Kustomization, GitRepository, HelmRepository, HelmChart, Bucket,
  ImageRepository and ImageUpdateAutomation** (`v1`, `v1beta2`, `v1beta1`) and
  **OCIRepository** (`v1`, `v1beta2`): Suspend, Resume, Reconcile.
- **Flux HelmRelease** `v2` and `v2beta2`: Suspend, Resume, Reconcile, Force reconcile
  and Reset retries. On `v2beta1` only Suspend, Resume and Reconcile, because force and
  reset arrived with `v2beta2`.
- **Argo CD Application** (`v1alpha1`): Refresh status, Hard refresh, Sync. Sync does
  not enable pruning; configured sync options and hooks still apply.
- **Other resources and API versions** remain inspectable without invented or
  unsupported actions.

The backend fetches the resource again, checks the reviewed UID and resourceVersion,
and includes both in a conditional PATCH, rejecting stale or replaced resources. It
also rejects:

- a second Argo CD sync while an operation is already present
- reconciliation while suspended
- a Suspend of a suspended resource, or a Resume of one that is not suspended
- writes to a resource being deleted

API failures remain errors with no success message. Kubernetes RBAC still governs the
GET, event list and PATCH. The implementation follows
[Flux reconciliation and Helm actions](https://fluxcd.io/flux/components/helm/helmreleases/)
and [Argo CD operations through Kubernetes](https://argo-cd.readthedocs.io/en/stable/user-guide/sync-kubectl/).
Declared, app-defined actions will replace this built-in list
([#518](https://github.com/srelens/srelens/issues/518)).

## Web host

- Every `extensions.*` capability is refused on the multi-user web host until app
  state is kept per user ([#515](https://github.com/srelens/srelens/issues/515)).
- `k8s.gitOpsAction` is refused as well. On the web no installed app scopes it to a
  resource, and there is no consent prompt.
- `k8s.getCustomResource` stays available: it is a read under the user's own
  kubeconfig and RBAC, like every other custom-resource read.
