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
| `name` | Yes | Display name, 1–120 characters. |
| `version` | Yes | The app's own SemVer version. |
| `srelensApiVersion` | Yes | A SemVer range of extension API versions, for example `^0.1`. See [Versioning](specification.md#versioning). |
| `kind` | Yes | `declarative`. No other kind is accepted. |
| `permissions` | Yes | The exact host capability IDs the bindings use. |
| `capabilities` | Yes | 1–32 bindings, below. |
| `contributions` | Yes | `pages`, `detailTabs` and `detailLinks`, below. |

Unknown fields are errors at every level. A manifest is at most 256 KiB.

## Capability bindings

Each entry in `capabilities` binds a local operation to a trusted host capability:

| Field | Meaning |
|---|---|
| `name` | Local operation name, unique within the manifest. Addressed as `plugin/<id>/<name>`. |
| `title` | Display title. |
| `target` | The host capability ID. It cannot start with `plugin/`; apps cannot call other apps. |
| `arguments` | Fixed arguments, merged into every call. Callers cannot override them. |
| `inputs` | The argument names a caller may supply. They cannot overlap with `arguments`. |

Every required argument of the target must come from `arguments` or `inputs`, and the
target's own handler validates the values. `permissions` must name exactly the set of
targets used.

The author cannot supply a handler, JavaScript, a schema or safety annotations.
Annotations come from the host: mutations, destructive operations and sensitive reads
cannot lose their confirmation requirement. Fixed arguments are excluded from the
public input schema.

## Contributions

Contribution `id`s are unique across all three lists (at most 64 in total), and each
contribution names a declared capability.

### `pages`

| Field | Meaning |
|---|---|
| `id`, `title`, `capability` | Identity, navigation label, and the binding that lists the page's resources. |
| `group` | Optional navigation group label. |
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
