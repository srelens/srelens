# Capabilities

What the host exposes for apps, through the shared capability registry and MCP.
Consent rules are in [permissions.md](permissions.md).

## `extensions.*`

| Capability | Kind | Purpose |
|---|---|---|
| `extensions.list` | Read-only | The installed apps, with revision, grants, settings, source, install time, up to three replaced versions and any quarantine reason. |
| `extensions.read` | Read-only | Run one of an installed app's declared readers, given its ID, revision, operation and context. Refused with "App is not enabled for this cluster" on a cluster the app is not enabled for; so are `extensions.resource` and `extensions.action`. All three also refuse a custom-resource reader unless a CustomResourceDefinition named `{plural}.{group}` serves its bound version on the cluster. |
| `extensions.resource` | Read-only | Inspect one resource of an enabled app, with its events and supported actions. |
| `extensions.catalog` | Read-only | Browse the catalog, from a 24-hour cache. Reports the host's supported API versions as `hostApiVersions`; the deprecated `hostApiVersion` still gives the newest. |
| `extensions.catalogManifest` | Read-only | Download and verify one catalog release for review. Does not install it. |
| `extensions.validate` | Read-only | Check a manifest, with its grants and optional signature, exactly as installing it would, and return every problem as `{code, path, message}` (see [Validation errors](specification.md#validation-errors)). Does not install it. A `signature` other than 64 bytes or a `manifest` over 256 KiB is refused as invalid input, not reported as a problem. |
| `extensions.configure` | Mutating | Install, enable, remove or configure an app. An install that fails validation is refused with the same problems. `clusters` limits an app to chosen kubeconfig contexts by context key (`{file}#{name}` with `#` and `%` encoded in each part, as `k8s.listContexts` reports under `key`; a stable ID can be shared by two contexts, #623), or with `null` allows every cluster. As with `extensions.validate`, a `signature` must be 64 bytes and a `manifest` at most 256 KiB, and `settings` must be at most 64 KiB as compact JSON. |
| `extensions.action` | Mutating | Request a host GitOps action on an app resource. |

The app facade refuses a host reader with stronger consent annotations than the
declarative contract allows. App-installed operations go through `extensions.read`;
per-app `plugin/...` tool discovery is a developer-harness feature (see
[testing.md](testing.md#developer-harness)).

## Host-defined capability metadata

Every capability carries six host-authored facts, written in
`crates/capability/src/annotations.rs` beside the handler they describe and
projected into the committed
[`capability-catalog.json`](../../packages/core/src/lib/capability-catalog.json):

| Field | Meaning |
|---|---|
| `readOnly` | Changes nothing. |
| `destructive` | Destroys or disrupts something, as opposed to merely changing it. |
| `requiresConfirm` | Execution stops at a consent step. The flag that actually gates a call, and not derivable from the others. |
| `sensitive` | Reads or reveals secret material. A redaction flag for the audit log, not a safety class. |
| `impact` | `low`, `medium` or `high` — how much a successful call disturbs. |
| `confirm` | The host's confirmation wording, as a template, or `null`. |

**Nothing outside the host supplies any of it.** A manifest declares no
annotations, and a binding inherits the target capability's row through
`Annotations::for_binding`, which can raise every field and lower none — so an
app cannot turn a destructive host operation into a read-only-looking tool, drop
its level, or put its own words in the dialog that authorizes it. The same
function fails closed over the host row itself: a capability that mutates,
destroys or returns secrets is gated even if its own annotation forgot to say so.

### Impact

`requiresConfirm` is true for a Secret read and for a node drain alike, so on its
own it cannot tell a reader which of two prompts deserves a pause. `impact`
answers the other question:

- **`low`** — a read, or a write whose only effect is to make a controller look
  again (an Argo CD status refresh).
- **`medium`** — changes cluster or host state, leaving workloads running: a
  scale, a suspend, a tool install, a Secret handed to a caller.
- **`high`** — destroys, disrupts or replaces something running: a delete, a
  drain, a sync that applies manifests and runs hooks.

The level and the gate cannot disagree: anything `destructive` is `high`,
anything gated is at least `medium`, and an ungated read is `low`.
`assert_impact_matches_the_gate` (`crates/mcp/src/completeness.rs`) fails the
build over the whole registry otherwise.

A capability that accepts several named operations publishes **the highest level
any of them reaches**, because `tools/list` and the catalog carry one row per
capability and a row that understated the worst case would mislead every reader
of it. `k8s.gitOpsAction` is `high` for that reason — one of its eight actions is
an Argo CD sync — and the per-action level travels with the resource instead, as
`actionMeta` on `extensions.resource`'s reply. See
[Host GitOps actions](#host-gitops-actions).

### Confirmation templates

`confirm` is a template, not a finished sentence: it is rendered against one
call's arguments. The scheme is small and deliberately closed.

- `{field}` is replaced by that field's value. `field` is one of `action`,
  `cluster`, `kind`, `name`, `namespace`, `resource` — a fixed vocabulary, so a
  capability cannot paste a token, a manifest or a Secret value into a dialog
  title. `{resource}` is derived from the others (`kind namespace/name`,
  collapsed to whatever is known) and is never read from the arguments.
- `[ … ]` is an **optional segment**: kept only when every `{field}` inside it
  has a value, dropped whole otherwise. Segments do not nest.
- A `{field}` **outside** a segment with no value makes the render fail. The
  confirming surface then shows the capability summary rather than a sentence
  with a hole in it. Every committed template must render with no fields at all,
  which `assert_confirm_templates_are_renderable` enforces.

So `k8s.scale` carries:

```
Change the replica count[ of {resource}][ in cluster {cluster}]?
```

which reads as *Change the replica count of Deployment team/api in cluster prod?*
for a call that names both, and *Change the replica count?* for one that names
neither.

Both the host (`srelens_capability::render_confirm`) and the frontend
(`renderConfirmTemplate` in `@srelens/core`) implement the same scheme, so the
sentence in an MCP denial and the sentence in a dialog are one string, written
once. A host-owned confirmation UI built on this is
[#552](https://github.com/srelens/srelens/issues/552).

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

`extensions.resource` returns an `actionMeta` entry for every action it offers,
carrying the host's level and confirmation wording for that action. The levels
are not uniform, which is the reason they are per action:

| Action | Impact | Because |
|---|---|---|
| Refresh status | `low` | Argo CD re-reads the application's status. No manifest is applied. |
| Hard refresh | `medium` | Also drops Argo CD's manifest cache. |
| Sync | `high` | Applies the application's desired resources and runs its sync hooks. |
| Suspend, Resume, Reconcile, Reset retries | `medium` | Change what a controller does next; workloads already running are not stopped. |
| Force reconcile | `high` | Re-runs the Helm install or upgrade even when chart and values are unchanged. |

The `k8s.gitOpsAction` capability itself is published as `high` — the ceiling of
that table — and was deliberately **not** split into one capability per action.
The host, not the app, decides which actions a resource offers, so separate IDs
would not narrow what an installed app can reach; and
[#549](https://github.com/srelens/srelens/issues/549) replaces this table with
action primitives that derive the same two fields from the primitive and its
target fields.

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
