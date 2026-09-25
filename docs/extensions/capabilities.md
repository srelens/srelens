# Capabilities

What the host exposes for apps, through the shared capability registry and MCP.
Consent rules are in [permissions.md](permissions.md).

## `extensions.*`

| Capability | Kind | Purpose |
|---|---|---|
| `extensions.list` | Read-only | The installed apps, with revision, grants, settings, source, install time, up to three replaced versions and any quarantine reason. |
| `extensions.read` | Read-only | Run one of an installed app's declared readers, given its ID, revision, operation and context. Refused with "App is not enabled for this cluster" on a cluster the app is not enabled for; so are `extensions.resource` and `extensions.action`. All three also refuse a custom-resource reader unless a CustomResourceDefinition named `{plural}.{group}` serves its bound version on the cluster. |
| `extensions.resolveColumns` | Read-only | Resolve an app's native table columns and badges for up to 1,000 row summaries in one `uids[]` batch. Direct badges read the rows' metadata only, never Secrets. The host rechecks the installed revision, grants, cluster scope and joined CRD before listing; failed reads stay explicit. Desktop-only until web app isolation exists. |
| `extensions.resolveCards` | Read-only | Answer every dashboard card an app declares for one cluster and a `namespaces[]` selection (at most 256): a figure, or on that card alone why it has none. The host rechecks the installed revision, grants, cluster scope and each source's CRD, and reads each source once through the shared five-second snapshot. `extensions.read` accepts a `card` id to return only the rows that card counted, with the card's `namespaces[]` when it counted in several. Desktop-only until web app isolation exists. |
| `extensions.resolvePanels` | Read-only | Resolve installed declarative detail panels for a selected resource. The host rechecks the revision, grants and cluster scope, and uses only declared join readers. Desktop-only until web app isolation exists. |
| `extensions.resolveLinks` | Read-only | Resolve an app's `resourceLinks` for a selected resource: `{ from, links: [{ id, relation, to, capability, targets: [{ namespace, name, exists, unverified? }], error? }] }`. A target the host did not look up (a bare Argo CD name with no `defaultNamespace`) carries `unverified` with why, and is never `exists`. Targets are looked up only in the granted reader for `to`, and only when the resource names one. A failed read is an `error` on that link, never an empty `targets`. Desktop-only until web app isolation exists. |
| `extensions.resource` | Read-only | Inspect one resource of an enabled app, with its events and supported actions. |
| `extensions.streams` | Read-only | The open app streams in this process and what each app has sent: open, opened, messages, payload bytes, streams stopped for the rate and opens refused for the cap, with the limits. For the Inspector ([#575](https://github.com/srelens/srelens/issues/575)); the streams themselves are opened by host commands, not capabilities. See [streams.md](streams.md). Desktop-only until web app isolation exists. |
| `extensions.catalog` | Read-only | Browse the catalog, from a 24-hour cache. Reports the host's supported API versions as `hostApiVersions`; the deprecated `hostApiVersion` still gives the newest. |
| `extensions.catalogManifest` | Read-only | Download and verify one catalog release for review. Does not install it. |
| `extensions.validate` | Read-only | Check a manifest, with its grants and optional signature, exactly as installing it would, and return every problem as `{code, path, message}` (see [Validation errors](specification.md#validation-errors)). Does not install it. A `signature` other than 64 bytes or a `manifest` over 256 KiB is refused as invalid input, not reported as a problem. |
| `extensions.configure` | Mutating | Install, enable, remove or configure an app. An install that fails validation is refused with the same problems. `clusters` limits an app to chosen kubeconfig contexts by context key (`{file}#{name}` with `#` and `%` encoded in each part, as `k8s.listContexts` reports under `key`; a stable ID can be shared by two contexts, #623), or with `null` allows every cluster. As with `extensions.validate`, a `signature` must be 64 bytes and a `manifest` at most 256 KiB, and `settings` must be at most 64 KiB as compact JSON. `settings` is held to the typed settings the manifest declares and refused with each problem at `settings.<id>` (see [Settings](manifest.md#settings)); a value for a `secret-reference` is always refused. |
| `extensions.action` | Mutating | Run an app’s declared action against a resolved resource through its bound host primitive. |
| `extension.secretStore` | Mutating, sensitive | Set or clear an app's `secret-reference` setting in the host's secret store: `{"action":"set","id","setting","secret"}` or `{"action":"clear","id","setting"?}` (no `setting` clears every secret the app keeps). Write-only: answers `{"set": bool}` and never returns a value. A set needs the app's `extension.secretStore` grant and an available store, and `secret` is 1–16384 bytes of text with no NUL; every refusal leaves the value out. Also the permission an app requests to keep secrets, so its metadata is what the install review shows. `extensions.list` reports whether the store is available as `secretStore: {available, reason?}`. See [Secret settings](manifest.md#secret-settings). |

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
of it. `extensions.action` is `high` for that reason — a declared action can be
an Argo CD sync — and the per-action level travels with the resource instead, as
`actionMeta` on `extensions.resource`'s reply. See
[Declared GitOps actions](#declared-gitops-actions).

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

## Declared GitOps actions

Flux and Argo CD declare their actions in the API 0.3 manifests under
`examples/extensions`. Reading a kind grants no write access. Installation or
update must explicitly grant every action primitive in `permissions`.

`extensions.action` resolves the installed app revision, cluster scope, reader,
and action declaration before dispatching its bound primitive. The caller supplies
only the selection, action ID, reviewed UID and `resourceVersion`; it cannot
replace the kind, patch or preconditions. Every call rechecks app lifecycle and grants.

The Flux manifest declares Suspend, Resume and Reconcile for its nine supported
controller kinds, plus Force reconcile and Reset retries for HelmRelease. ImagePolicy,
notification resources and all other readers have no implicit actions. Argo CD declares
Refresh status, Hard refresh and Sync; Sync preserves `prune: false` and hook strategy.
The reader binding fixes the exact API version for each action.

`extensions.resource` returns action IDs and `actionMeta`, whose `title` and
`availableWhen` predicates come from the installed declaration. The host primitive
supplies `impact` and `confirm`; an app cannot soften those. Inspector and bulk
controls render these fields without a GitOps label or availability table.

Declared preconditions reject reconciliation while suspended, redundant Suspend or
Resume requests, and Sync while an Argo CD operation exists. The primitives enforce
these against a fresh GET after their unconditional UID/resourceVersion and deletion
checks, then pin the PATCH. Display predicates only explain availability and never
authorize a write. Force/reset write both Flux annotations in one patch, with the
same request timestamp. API failures remain errors; accepted requests do not claim
that controller work has completed.

`k8s.gitOpsAction` has been removed. There is no compatibility endpoint or inherited
write permission. API 0.1 releases must be replaced by signed API 0.3 releases and
reviewed with their new grants before they can be enabled.

## Host action primitives

The four capabilities a manifest binds as `actions`
([#549](https://github.com/srelens/srelens/issues/549), written up in
[manifest.md](manifest.md#declared-actions)). They execute a write the *app* declares, against a kind it already holds a granted reader for,
with every rule enforced by the host.

| Primitive | Impact | Because |
|---|---|---|
| `k8s.annotate` | `medium` | Sets one annotation key. It changes no spec and stops nothing; what the controller does next is the controller's. |
| `k8s.setFields` | `medium` | Sets fixed fields under `spec`. Workloads already running are not stopped. |
| `k8s.setStatusCondition` | `medium` | Writes one condition through the status subresource, which a controller then acts on. |
| `k8s.mergePatch` | `high` | The one that can express an Argo CD sync: applying manifests and running hooks. |

The level is the ceiling of what the *primitive's shape* can do, not of what a
controller may do afterwards — an app is free to bind Flux's `forceAt` key through
`k8s.annotate`, and a host that called every annotation `high` for that reason would
be telling every Argo CD refresh the same thing. A binding never comes out below its
primitive's row ([`Annotations::for_binding`](#host-defined-capability-metadata) only
raises), so #550 and #551 can publish a specific action above it without moving these.

Each primitive re-reads the object, refuses a review that no longer matches and an
object being deleted, pins its patch to the reviewed UID and `resourceVersion`, and
reports the request as accepted rather than as complete.

## Web host

- Every `extensions.*` capability is refused on the multi-user web host until app
  state is kept per user ([#515](https://github.com/srelens/srelens/issues/515)).
  So are the app stream commands (`extension_stream_open`, `extension_stream_cancel`,
  `extension_stream_close_view`); see [streams.md](streams.md#hosts).
- `extension.secretStore` is refused before dispatch too: the web host keeps no app
  secrets until per-user storage exists ([#522](https://github.com/srelens/srelens/issues/522)),
  and `@srelens/core` refuses a set on the web before the value leaves the page.
- The four host action primitives are refused for the same reason: what bounds one is
  an installed manifest fixing the kind and the template, which the web host has none
  of, so a caller would be naming both itself.
- `k8s.getCustomResource` stays available: it is a read under the user's own
  kubeconfig and RBAC, like every other custom-resource read.
