# Manifest reference

Every app is one JSON manifest. The rules for versioning, identifiers and unknown
fields are normative and live in [specification.md](specification.md); this page is
the field reference. Complete examples: [argocd.json](../../examples/extensions/argocd.json)
and [flux.json](../../examples/extensions/flux.json).

## JSON Schema

The schema for API 0.1 is committed at
[`schemas/extension-manifest.v0.1.json`](../../schemas/extension-manifest.v0.1.json).
Point your editor at it by naming it in the manifest:

```json
{
  "$schema": "https://raw.githubusercontent.com/srelens/srelens/main/schemas/extension-manifest.v0.1.json",
  "id": "io.example.cert-manager"
}
```

The file is generated from the host's `Manifest` type, and `cargo test` fails when the
two differ. After changing a manifest field, regenerate it with:

```sh
UPDATE_CATALOG=1 cargo test -p srelens-plugin-host --test schema
```

The schema checks shape only. The host also enforces the rules on this page that a
schema cannot express, such as identifier syntax, unique names and permission
coverage, so validate with the [developer harness](testing.md#developer-harness)
before publishing.

## Top-level fields

| Field | Required | Meaning |
|---|---|---|
| `$schema` | No | The JSON Schema URL, for editors. The host ignores it. |
| `id` | Yes | Reverse-domain identifier. See [Identifiers](specification.md#identifiers). |
| `name` | Yes | Display name, 1–120 characters, with no control characters and no bidirectional or invisible format characters. See [Identifiers](specification.md#identifiers). |
| `version` | Yes | The app's own SemVer version. |
| `srelensApiVersion` | Yes | A SemVer range of extension API versions, for example `^0.1`. See [Versioning](specification.md#versioning). |
| `kind` | Yes | `declarative`. No other kind is accepted. |
| `permissions` | Yes | The exact host capability IDs the bindings use. |
| `capabilities` | Yes | 1–32 bindings, below. |
| `actions` | No | Up to 32 declared mutations, below. |
| `contributions` | Yes | `pages`, `detailTabs` and `detailLinks`, below. |

Unknown fields are errors at every level. A manifest is at most 256 KiB.

## Capability bindings

Each entry in `capabilities` binds a local operation to a trusted host capability:

| Field | Meaning |
|---|---|
| `name` | Local operation name, unique within the manifest. Addressed as `plugin/<id>/<name>`. |
| `title` | Display title, held to the same rules as `name`. |
| `target` | The host capability ID. It cannot start with `plugin/`; apps cannot call other apps. |
| `arguments` | Fixed arguments, merged into every call. Callers cannot override them. A `k8s.listCustomResource` binding may declare at most 32 `printerColumns`. |
| `inputs` | The argument names a caller may supply. They cannot overlap with `arguments`. |

Every required argument of the target must come from `arguments` or `inputs`, and the
target's own handler validates the values. `permissions` must name exactly the set of
targets used.

The author cannot supply a handler, JavaScript, a schema or safety annotations.
Annotations come from the host: mutations, destructive operations and sensitive reads
cannot lose their confirmation requirement, and neither the impact level nor the
confirmation wording can be lowered or replaced — see
[Host-defined capability metadata](capabilities.md#host-defined-capability-metadata).
Fixed arguments are excluded from the public input schema.

## Declared actions

An app never sends a Kubernetes request. Each entry in `actions` names one **host
action primitive** and the reader binding whose kind it acts on, and the host builds
the request:

| Field | Meaning |
|---|---|
| `name` | Local action name, unique across `capabilities` and `actions`. Addressed as `plugin/<id>/<name>`. |
| `title` | Display title, held to the same rules as a binding's. |
| `target` | A host action primitive: `k8s.annotate`, `k8s.setFields`, `k8s.setStatusCondition`, `k8s.mergePatch`, `k8s.requestRolloutRestart` or `k8s.requestCordonNode`. |
| `resource` | The `name` of a reader binding in `capabilities`. The action acts on that binding's kind and on no other. |
| `arguments` | What the action writes, fixed here. |

```json
"actions": [{
  "name": "reconcile",
  "title": "Reconcile",
  "target": "k8s.annotate",
  "resource": "helmreleases",
  "arguments": { "key": "reconcile.fluxcd.io/requestedAt", "value": "$now" }
}]
```

The host fills in `group`, `version`, `plural`, `kind` and `namespaced` from the reader
binding `resource` names, and fixes the inputs to `context`, `namespace`, `name`, `uid`
and `resourceVersion` — the object the operator reviewed. An action that binds any of
those itself is rejected, and so is one whose reader does not fix its kind: there is no
field in which an app can name a kind it holds no granted reader for. `permissions`
names the primitive like any other host capability, and the user grants it.

Every primitive re-reads the object, refuses a UID or `resourceVersion` that has moved
on and an object that is being deleted, pins its patch to both, and reports
`{"requested": true}` — the API server accepted the request, which is not a claim that
the controller has done anything.

| Primitive | Arguments | Writes |
|---|---|---|
| `k8s.annotate` | `key`, `value` | One annotation. |
| `k8s.setFields` | `fields` | RFC 6901 pointers under `/spec` (at most 16, at most 8 segments deep, none inside another), each set to a fixed value. |
| `k8s.setStatusCondition` | `conditionType`, `conditionStatus`, `reason`, `message?` | One condition, through the **status subresource**, carrying over the conditions it does not own. `conditionStatus` is `True`, `False` or `Unknown`, and `lastTransitionTime` moves only when the status changes. |
| `k8s.mergePatch` | `patch` | A fixed JSON merge patch, past the deny-list below. |

`k8s.setFields` writes **object fields**, and writes a list by naming the list
(`"/spec/ignore": ["a", "b"]`). A pointer that reaches *through* a list —
`/spec/containers/0/image` — is refused against the object the host just read,
because a merge patch replaces a value of a different shape rather than merging into
it, and on a field with no schema that would rewrite the whole list as an object. A
numeric or `-` segment is still a legal object key and is accepted as one; only the
live object decides.

A string value is a **literal**, except for the two tokens the host substitutes per
request: `$now` (RFC 3339, nanoseconds, UTC) and `$uuid`. Any other `$`-prefixed string
is rejected rather than written through, because an app that asked for `$timestamp`
meant a timestamp. A bound template is at most 8 KiB.

`k8s.mergePatch` may not write `metadata.finalizers`, `metadata.ownerReferences`,
`metadata.managedFields`, `metadata.uid`, `metadata.resourceVersion` or `status`, may
not write a Secret's `data` or `stringData`, and may not target an RBAC kind (`Role`,
`ClusterRole`, `RoleBinding`, `ClusterRoleBinding`, or anything in
`rbac.authorization.k8s.io`). The host applies the same rules when the manifest is
installed, when a stored app is reverified, and on the way to the cluster.

### Preconditions and availability

An action may declare what must be true of the object, as predicates the host
evaluates rather than words in a description:

| Field | Meaning |
|---|---|
| `preconditions` | Checked by the host against its own fresh read of the object, before the patch. One that does not hold refuses the request, and the operator is told its `reason`. |
| `availableWhen` | Checked by the surface, against the object it is already showing, to decide whether the control is offered. The `reason` is the disabled control's tooltip. |

Each list holds at most 8 predicates, and each predicate is one question about one
value:

```json
"preconditions": [
  { "jsonPath": ".spec.suspend", "notEquals": true,
    "reason": "Resume this resource before requesting reconciliation" }
]
```

| Field | Meaning |
|---|---|
| `jsonPath` | The value to ask about. An optional leading `$`, then `.key`, `['key']`, `["key"]` and `[0]`, at most 8 segments and 256 characters. No wildcard, filter, recursive descent or function — each addresses a *set* of values, and "does this hold" over a set is a different question. |
| `equals` / `notEquals` | The value must (not) be this string, number or boolean **literal**. An object or a list is not a comparand. A field nobody set is not equal to anything, so `notEquals` holds when it is absent. |
| `present` / `absent` | Written `true`. The value must be set, or unset. `null` counts as unset, which is also why `null` is not a comparand: write `absent: true`. |
| `reason` | Required, at most 200 characters. Shown to the operator, so it says what to do next. |

Exactly one operator per predicate. Both lists are checked when the app is installed
and each time a stored app is reverified, and `preconditions` are checked again on the
way to the cluster — one statement of each rule, in
`crates/capability/src/predicate.rs`, run from both ends.

Two things these cannot do:

- **They cannot remove a host guard.** An object that is being deleted, and a `uid` or
  `resourceVersion` that has moved on since the operator reviewed it, are refused
  before any declared predicate is evaluated. A predicate can only add a refusal.
- **`availableWhen` does not enforce.** A surface can be seconds out of date; the
  host's own read cannot. A condition that must hold when the write lands belongs in
  `preconditions`, and an app that means both writes both.

The `reason` is an app's text shown in the host's UI, so the host escapes it before
drawing it and frames its own refusal around it.

The host-owned confirmation dialog
([#552](https://github.com/srelens/srelens/issues/552)) is not part of this API version
yet, and the Flux and Argo CD actions in core still come from the host's own table
until [#551](https://github.com/srelens/srelens/issues/551) moves them into manifests.

## Built-in operational action bindings

Four existing summary readers can scope operational actions. Their `arguments`
must be empty: the host derives the exact API group, version, plural, kind and
scope from the reader target.

| Reader | Inputs | Allowed action | Fixed action arguments | Impact |
|---|---|---|---|---|
| `k8s.listDeployments` | `context`, `namespace` | `k8s.requestRolloutRestart` | `{}` | High |
| `k8s.listStatefulSets` | `context`, `namespace` | `k8s.requestRolloutRestart` | `{}` | High |
| `k8s.listDaemonSets` | `context`, `namespace` | `k8s.requestRolloutRestart` | `{}` | High |
| `k8s.listNodes` | `context` | `k8s.requestCordonNode` | `{"unschedulable": true}` to cordon, `false` to uncordon | Medium |

Grant both the reader and action target. For example:

```json
"capabilities": [{
  "name": "deployments", "title": "Deployments", "target": "k8s.listDeployments",
  "arguments": {}, "inputs": ["context", "namespace"]
}],
"actions": [{
  "name": "restart", "title": "Restart", "target": "k8s.requestRolloutRestart",
  "resource": "deployments", "arguments": {}
}]
```

The reviewing host supplies `context`, `namespace`, `name`, `uid` and
`resourceVersion`; use an empty namespace for Nodes. Each request freshly reads
the object, refuses a changed or deleting object and unmet `preconditions`, and
pins its patch to the reviewed UID and resource version. Success means
`{"requested": true}`, not that rollout has finished. Host confirmation and
impact annotations are inherited by the registered app action.

These adapters share patch construction with `k8s.rolloutRestart` and
`k8s.cordonNode`. They have separate capability IDs because those interactive
operations do not accept a reviewed object identity. Apps cannot bind the
interactive operations. General annotation, field, status and merge-patch writes
remain unavailable on built-in reader bindings. Drain remains deferred: cordon
never evicts pods.

Built-in readers return their existing summary formats. They do not provide
whole objects, and cannot back the custom-resource table or detail contributions.
This contract adds action bindings, not a built-in resource screen. Custom-resource
bindings still require a real CRD; an app cannot disguise a built-in kind as one.

## Contributions

Contribution `id`s are unique across all three lists (at most 64 in total), and each
contribution names a declared capability.

### `pages`

| Field | Meaning |
|---|---|
| `id`, `title`, `capability` | Identity, navigation label, and the binding that lists the page's resources. |
| `group` | Optional navigation group label, held to the same rules as `name`. |
| `statusColumns` | Optional `{ ready, suspended?, progressing? }`: zero-based indices into the binding's `printerColumns`, each below 64. |
| `dashboard` | Optional `{ pages, events? }`. `pages` references 1–12 resource pages that have `statusColumns` and are not dashboards. `events` is `{ capability, apiGroups }`, where `capability` binds `k8s.listEvents` and `apiGroups` lists 1–32 dotted groups. |

### `detailTabs` and `detailLinks`

| Field | Meaning |
|---|---|
| `id`, `title`, `capability` | Identity, label and binding. |
| `forKinds` | 1–32 group-qualified kinds: `argoproj.io/Application`, or `/Pod` for the core group. |

A detail tab adds a tab to a matching resource's detail view. A detail link adds an
entry to that view's **App links** menu, which opens a read-only results panel. Neither
writes to the cluster.

`rowActions` was this contribution's name before release
([#537](https://github.com/srelens/srelens/issues/537)). It is reserved for declared
mutations ([#549](https://github.com/srelens/srelens/issues/549)), and a manifest that
uses it now is rejected as an unknown field.

## Rules the desktop app adds

The desktop app accepts a narrower surface than the developer broker:

- Targets are `k8s.listCustomResource` or `k8s.listEvents`, and the target must be
  read-only with no confirmation, sensitive or destructive annotation.
- Inputs are only `context` and `namespace`.
- A `k8s.listCustomResource` binding fixes a non-empty `group`, `version`, `plural`
  and `kind` (letters, digits, `.` and `-`) and a boolean `namespaced`. It must accept
  `context`, may not fix `context` or `namespace`, and a namespaced binding must accept
  `namespace`.
- That `group` must be shaped like a CustomResourceDefinition group: dot-separated labels
  such as `argoproj.io` or `gateway.networking.k8s.io`, so built-in groups such as `apps`
  and `batch` are refused. The problem is reported at `capabilities[i].arguments.group`,
  and an installed app that breaks the rule is quarantined when the inventory loads.
  Every read, inspection and action also checks that a CustomResourceDefinition named
  `{plural}.{group}` serves the bound `version` on the cluster, and is refused when none
  does. That refuses dotted built-in groups such as `networking.k8s.io` and aggregated
  APIs.
- A `k8s.listEvents` binding has no fixed arguments and accepts both `context` and
  `namespace`.
- Every page, detail tab and detail link references a `k8s.listCustomResource` binding.
- `statusColumns` indices point at declared `printerColumns`.

Settings → Apps checks these rules together with the manifest's own before it offers
to install, and lists every problem with its path. See
[Validation errors](specification.md#validation-errors).

The examples bind `argoproj.io/v1alpha1` Applications and Flux's
`kustomize.toolkit.fluxcd.io/v1` Kustomizations and `helm.toolkit.fluxcd.io/v2`
HelmReleases. The cluster must serve those versions; see
[requirement checks](ui-contributions.md#requirement-checks).
