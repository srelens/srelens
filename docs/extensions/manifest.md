# Manifest reference

Every app is one JSON manifest. The rules for versioning, identifiers and unknown
fields are normative and live in [specification.md](specification.md); this page is
the field reference. Complete examples: [argocd.json](../../examples/extensions/argocd.json)
and [flux.json](../../examples/extensions/flux.json).

Generate the JSON Schema for your editor or validator with:

```sh
cargo run -p srelens-plugin-host --example extension_host -- --schema
```

## Top-level fields

| Field | Required | Meaning |
|---|---|---|
| `id` | Yes | Reverse-domain identifier. See [Identifiers](specification.md#identifiers). |
| `name` | Yes | Display name, 1–120 characters. |
| `version` | Yes | The app's own SemVer version. |
| `srelensApiVersion` | Yes | A SemVer range of extension API versions, for example `^0.1`. See [Versioning](specification.md#versioning). |
| `kind` | Yes | `declarative`. No other kind is accepted. |
| `permissions` | Yes | The exact host capability IDs the bindings use. |
| `capabilities` | Yes | 1–32 bindings, below. |
| `contributions` | Yes | `pages`, `detailTabs` and `rowActions`, below. |

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

### `detailTabs` and `rowActions`

| Field | Meaning |
|---|---|
| `id`, `title`, `capability` | Identity, label and binding. |
| `forKinds` | 1–32 group-qualified kinds: `argoproj.io/Application`, or `/Pod` for the core group. |

`rowActions` open read panels; they are not cluster writes. The name is planned to
change to `detailLinks` ([#537](https://github.com/srelens/srelens/issues/537)).

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
- Every page, detail tab and row action references a `k8s.listCustomResource` binding.
- `statusColumns` indices point at declared `printerColumns`.

The examples bind `argoproj.io/v1alpha1` Applications and Flux's
`kustomize.toolkit.fluxcd.io/v1` Kustomizations and `helm.toolkit.fluxcd.io/v2`
HelmReleases. The cluster must serve those versions; see
[requirement checks](ui-contributions.md#requirement-checks).
