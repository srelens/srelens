# Extension API specification

This is the normative contract between srelens and the apps (extensions) it hosts:
how the extension API is versioned, what may change within a version, and the rules
every manifest follows. The other pages in this directory describe how things work
today; where they and this page disagree, this page is the intent and the other page
is the bug.

Tracking: [#163](https://github.com/srelens/srelens/issues/163). Field reference:
[manifest.md](manifest.md).

## Scope

- **Hosts.** Apps run in the desktop app, whose extension capabilities are also exposed
  over MCP. The web host refuses every `extensions.*` capability.
- **The terminal UI is out of scope for extension API 1.0.** The TUI neither loads nor
  renders apps, and nothing in this specification applies to it.
- **Clusters.** An app is installed for the whole application. It may be limited to
  chosen kubeconfig contexts; on the others it is hidden and the host refuses its reads
  and actions.

## Unsigned-app policy

Settings → Apps stores **Allow unsigned apps to modify clusters and run code**
(`allowUnsignedApps`) in the extension inventory. It is **off by default**.

- Read-only declarative apps from any source, including unsigned local and catalog
  manifests, need their normal permission grants only.
- An app declaring any write action needs either a cryptographically verified
  publisher signature or this setting. Source labels, catalog membership, repository
  URLs, app IDs, and granted permission strings do not establish publisher trust.
  Signature proofs are checked again whenever installed apps are loaded.
- Install, update, enable, and rollback enforce the policy in the host. A permission
  review alone cannot authorize an unsigned writer. Validation reports the setting
  needed before installation.
- Turning the setting off disables affected apps without uninstalling them or
  removing their grants, settings, cluster scope, or version history. Apps shows the
  reason in `policyBlocked`, separate from a failed-signature `quarantined` reason.
  Turning the setting back on does not re-enable apps: enable each app explicitly.
- Every new read, resource inspection, or action through the broker checks the latest
  inventory, including calls through registrations created before the setting changed.
  Calls already admitted may finish. The generic `PluginHost` binding primitive is
  used by the trusted installer; production invocation passes through this broker.
- Existing inventories without `allowUnsignedApps` migrate to false. Legacy
  `developerMode: true` does not grant the new permission. Legacy disabled entries
  remain disabled. The next atomic save persists the policy; transient denial reasons
  are recomputed, never trusted from disk.
- All executable apps without a verified publisher require the same policy even if
  they declare no writes. **This host does not yet support executable apps**: its
  manifest parser rejects executable kinds regardless of this setting. A future
  runtime must extend the exhaustive kind classifier and enforce this gate before
  admitting code; enabling this setting does not enable an SDK or runtime today.

The configuration payload is
`{"action":"unsignedApps","allowUnsignedApps":true}` through the existing
confirm-gated `extensions.configure` capability. The setting never replaces normal
permission grants, action confirmation, cluster scoping, or manifest validation.

## Terms

- **Extension API version**: the version of this contract, in SemVer form
  (`0.3.0`). It is independent of the srelens app version and of each app's own
  `version`.
- **Supported set**: the API versions a host implements, listed oldest first in
  `SUPPORTED_API_VERSIONS` (`crates/plugin-host/src/manifest.rs`). The catalog
  reports it as `hostApiVersions`.
- **API range**: a manifest's `srelensApiVersion`, a SemVer requirement in Cargo
  syntax, such as `^0.3` or `>=0.3, <0.4`.
- **Negotiated version**: the highest supported version the range matches. The host
  treats the app as written for that version.

## Versioning

1. **Acceptance.** A host accepts a manifest when its API range matches at least one
   version in its supported set. Otherwise the manifest is rejected with a message
   naming the range it requires and the versions the host supports. The range is
   checked before the rest of the manifest, so a manifest written for a newer API
   is told which version it needs, not which field the host does not know.
2. **Installed apps.** An installed app whose range no longer matches any supported
   version is not removed. It is quarantined: disabled, with the reason shown in
   Settings → Apps, until it is updated or removed.
3. **Choosing a range.** Target the API line you tested against with a caret range:
   `^0.4` before 1.0, `^1.2` after. Under SemVer caret rules a `0.x` range pins its
   minor version, so `^0.1` does not match `0.2.0`. That is deliberate: each `0.MINOR`
   is its own compatibility line.
4. **New API versions.** Before 1.0, any manifest-visible change (a new field, a new
   contribution type, a new allowed capability target, a changed meaning) is a new
   `0.MINOR` version. A `0.MINOR.PATCH` bump is for clarifications that do not change
   which manifests validate. From 1.0: MAJOR for breaking changes, MINOR for
   additive ones, PATCH for fixes.
5. **Current supported lines.** This host implements **API 0.3 and API 0.4**. A
   `^0.3` manifest is served under 0.3 and may use only what 0.3 has; a `^0.4` one may
   also use what [0.4 added](#040). API 0.1 and API 0.2 are not supported. Existing
   installations targeting a retired line are quarantined until replaced by a
   compatible manifest. Official manifests must receive a new version and publisher
   signature; editing an installed signed manifest invalidates its proof.
6. **API 1.0.** The API is frozen as 1.0 when the cert-manager declarative milestone
   ([#582](https://github.com/srelens/srelens/issues/582)) passes. After that, the 1.x
   line only grows additively.

## Compatibility rules

A change is **additive** when every manifest the host accepted before is still
accepted and still means the same thing. Anything else is **breaking**.

| Change | Classification |
|---|---|
| New optional manifest field or new contribution type | Additive. Needs a new API minor; manifests that use it must require that minor. |
| New host capability allowed as a binding target | Additive. Needs a new API minor. |
| New host behaviour for existing manifests, with no manifest change (for example host-rendered resource inspection) | Additive. No API bump, but it must be recorded in the changelog and must not reject or reinterpret any accepted manifest. |
| New optional field in a capability's output | Additive. |
| Removing or renaming a field, making an optional field required, or narrowing accepted values | Breaking. |
| Changing what an existing field means | Breaking. |
| Removing a capability target, or tightening validation so a previously valid manifest fails | Breaking. |
| Removing or renaming a field in a capability's input or output | Breaking. |

Breaking changes go into a new API line (a new `0.MINOR` before 1.0, a new MAJOR
after). They never change a version that is already supported.

Why a new field needs a new minor even though it is optional: manifests are strict
(see below), so a host that predates the field would reject it. Requiring the minor
turns that into a clear "requires API 0.x" message.

There is no pre-1.0 exception to this. There was one: the fields listed under
[0.4.0](#040) were first added to API 0.3 in place while the extension platform was
being built ([#517](https://github.com/srelens/srelens/issues/517)), and srelens builds
implementing 0.3 with none or only some of them had already been published. A signed
release using them under `^0.3` would have been offered by those hosts and then failed
to parse on an unknown field. They moved to API 0.4 before any signed release used them
([#709](https://github.com/srelens/srelens/issues/709)), and the exception is retired.

The host enforces this. `API_FIELDS` in `crates/plugin-host/src/manifest.rs` lists every
field whose availability differs across supported API lines, and every *form* of value
that a later line admits in a field all of them have — API 0.4's predicate path filter
in `actions[].preconditions` and `actions[].availableWhen`, which API 0.3 already had
without it. A rename is a removal plus an addition. The schema file of each older
supported line is kept as it was when the next line was cut
(`schemas/extension-manifest.v0.3.json`), and CI fails when the host's contract has a
field that file lacks and `API_FIELDS` does not list.

A manifest may use a field only if the field is available in every supported API
version its range admits, not just the one it negotiates to. Otherwise it is rejected
with `EXTENSION_API_INCOMPATIBLE`, even by a host that knows the field, for example
"`contributions.commands` requires API 0.4.0, but this manifest's srelensApiVersion
admits API 0.3.0". That covers a field a later line added, one a later line removed or
renamed, and a range that spans several lines: `>=0.3, <0.5` claims 0.3 hosts, so it
may not use a 0.4 field. Otherwise the manifest would install on some hosts and fail on
others that still match its range.

The check runs at installation and again every time the inventory is loaded. An
installed app that uses a field a newer host's supported versions no longer admit is
quarantined rather than left enabled. So is an app installed by a build that accepted a
0.4 field under `^0.3`: it is quarantined with that message until it is updated to a
release that requires `^0.4`. A field that is null or an empty list or object does not
count as used.

A change that narrows the values a field accepts, rather than adding or removing the
field, must add a check keyed on the negotiated API version in the same change. A change
that widens them is a new API minor with an `API_FIELDS` form entry, as the path filter
was.

**Published 0.3 releases stay installable.** The signed releases published on the 0.3
line, Argo CD 0.3.0 and Flux 0.4.0, use none of the 0.4 fields, so this host installs
and reverifies them unchanged; the registry's tests pin their exact bytes and
signatures. The condition filter Flux 0.4.0 writes in `arguments.printerColumns` is not
the 0.4 predicate filter: printer columns evaluated that form before API 0.4. Their
successors require `^0.4`, so a host that implements only 0.3 lists them as incompatible
instead of offering them.

## Deprecation

- A deprecated field or contribution is listed in the changelog with its replacement
  and the earliest API version that may remove it.
- It keeps working unchanged in every API version that supports it. Once structured
  validation errors exist ([#533](https://github.com/srelens/srelens/issues/533)),
  using it produces a warning.
- It is removed only in a new API line, and only after the retirement window in
  [Versioning](#versioning): at least two srelens minor releases, and never before its
  replacement has shipped.

Deprecated or planned:

- `hostApiVersion` in the `extensions.catalog` output is deprecated in favour of
  `hostApiVersions`. It stays, set to the newest supported version, until a new API
  line removes it.
- `rowActions` is a reserved contribution name. It was renamed to `detailLinks` before
  extensions went live ([#537](https://github.com/srelens/srelens/issues/537)) and will
  name declared mutations ([#549](https://github.com/srelens/srelens/issues/549)). Until
  then a manifest that uses it is rejected as an unknown field.
- `pages[].statusColumns` is deprecated in favour of `contributions.statusResolvers`
  ([#541](https://github.com/srelens/srelens/issues/541)), which API 0.4 adds. It keeps
  working unchanged on the 0.3 and 0.4 lines — its indices are validated and dashboards
  still count by it — and the earliest version that may remove it is the next API line,
  0.5.

## Unknown fields

- **Manifests are strict.** An unknown field at any level is an error. Silently
  ignoring one would let an app look installed on a host that does not implement
  what it declares. The one exception is a top-level `$schema` string naming the
  manifest's JSON Schema, which is editor metadata that the host ignores.
- **Catalog metadata is tolerant.** Hosts ignore catalog fields they do not
  recognize and still validate the fields they do. A breaking catalog change bumps
  the catalog's `schemaVersion` (currently `1`), which older hosts refuse.
- **Capability inputs are strict.** A payload with unknown fields is rejected. The
  wire names are camelCase.
- **The inventory is host-owned.** `settings.extensions.json` is not an authoring
  surface. Its format changes are listed in [migration.md](migration.md).

## Validation errors

A rejected manifest reports every problem the host finds. Each is
`{code, path, message}`:

- **`code`** is one of the codes below. A published code never changes meaning; new
  codes may be added.
- **`path`** runs from the manifest root, with `.` between fields and `[i]` for array
  elements, for example `contributions.pages[2].capability`. It is empty when the
  whole manifest is at fault.
- **`message`** is for the author and may change between releases.

A schema problem (an unknown field, a missing field or a wrong JSON type) stops
decoding, so those are reported one at a time. Once a manifest matches the schema,
every rule violation is reported together, including the desktop app's rules in
[manifest.md](manifest.md#rules-the-desktop-app-adds). `extensions.validate` returns
the list without installing, `extensions.configure` refuses an install with the same
list, and the [developer harness](testing.md#developer-harness) prints one per line.

| Code | Meaning |
|---|---|
| `EXTENSION_INVALID_JSON` | The manifest is not JSON. |
| `EXTENSION_TOO_LARGE` | The manifest exceeds 256 KiB. |
| `EXTENSION_UNKNOWN_FIELD` | A field the schema does not define. See [Unknown fields](#unknown-fields). |
| `EXTENSION_INVALID_FIELD` | A required field is missing, or a field has the wrong JSON type. |
| `EXTENSION_INVALID_ID` | The app `id` is not a reverse-domain identifier. See [Identifiers](#identifiers). |
| `EXTENSION_RESERVED_ID` | The `id` is in a namespace reserved for a signed publisher, and the manifest is unsigned. |
| `EXTENSION_INVALID_SIGNATURE` | The publisher signature does not verify against these exact bytes. |
| `EXTENSION_INVALID_VERSION` | `version` is not SemVer, or `srelensApiVersion` is not a SemVer range. |
| `EXTENSION_API_INCOMPATIBLE` | The host supports no API version the range admits, or the manifest uses a field missing from one it admits. |
| `EXTENSION_INVALID_VALUE` | A value breaks a documented limit: the syntax of a name, title or group, a count, or a column index. |
| `EXTENSION_DUPLICATE_IDENTIFIER` | A capability name, contribution ID, permission or other list entry repeats. |
| `EXTENSION_PERMISSION_MISMATCH` | `permissions` does not name exactly the bound host capabilities, or they were not all granted. |
| `EXTENSION_UNSUPPORTED_TARGET` | A binding or contribution uses a host capability that is not allowed there. |
| `EXTENSION_INVALID_BINDING` | A binding's arguments or inputs break its target's rules. |
| `EXTENSION_UNRESOLVED_CAPABILITY` | A contribution or dashboard names a capability the manifest does not declare. |
| `EXTENSION_UNRESOLVED_PAGE` | A dashboard names a page the manifest does not declare. |
| `EXTENSION_INVALID_KIND` | The manifest `kind` is not `declarative`, or a `forKinds` entry is not a qualified Kubernetes kind. |

## Identifiers

- **App `id`.** Reverse-domain: at least two dot-separated segments, each 1–64
  characters from `A–Z`, `a–z`, `0–9` and `-`, and at most 128 characters in total.
  Catalog entries additionally require lowercase, so use lowercase everywhere. Use a
  domain you control, for example `io.example.cert-manager`.
- **Reserved namespaces.** IDs under `org.srelens.` install only with the srelens
  publisher signature. A local, unsigned manifest cannot use one, and cannot replace
  a signed installation. See [distribution.md](distribution.md). Namespaces for other
  verified publishers are planned in
  [#559](https://github.com/srelens/srelens/issues/559).
- **Collisions between apps.** One installed app per ID. Installing an ID that is
  already installed is an explicit replacement after permission review. It keeps the
  app's settings and assigns a new revision.
- **Names inside a manifest.** Capability `name`s are unique, and an action's `name`
  shares that space, since both become `plugin/<id>/<name>`. Contribution `id`s are
  unique across `pages`, `detailTabs` and `detailLinks`. Both use `A–Z`, `a–z`, `0–9`
  and `-`, up to 64 characters.
- **Names, titles and groups.** The app `name`, every `title` and a page `group` are
  1–120 characters. They may not contain control characters (Unicode category Cc) or
  format characters (category Cf), which change how text displays without being seen:
  bidirectional marks, embeddings, overrides and isolates (U+200E–U+200F,
  U+202A–U+202E, U+2066–U+2069), zero-width spaces and joiners (U+200B–U+200D, U+2060),
  the byte order mark (U+FEFF), the soft hyphen (U+00AD) and tags. Letters, marks and
  symbols in any script are allowed, but an emoji sequence joined with U+200D is not.
  A catalog entry's `name` and `description` are held to the same rule, and refuse both
  control and format characters.
- **Derived names.** Operations are addressed as `plugin/<id>/<name>`. App routes
  carry the cluster, app ID, page and, where relevant, namespace and resource name.
- **Kinds.** `forKinds` entries are group-qualified: `argoproj.io/Application`, and
  `/Pod` for the core group.

## Versions of an app

- `version` must be SemVer 2.0.0.
- A catalog release's `version` and `srelensApiVersion` must equal the manifest's.
  Its SHA-256 pins the exact bytes, so changing a published manifest means publishing
  a new version.
- The app version, the extension API version and the srelens version are unrelated.
  Update and downgrade handling is planned in
  [#563](https://github.com/srelens/srelens/issues/563).

## API changelog

### 0.4.0

Everything below was first added to API 0.3 in place, and moved to this line before
any signed release used it (#709). A manifest that uses any of it requires `^0.4`.
Under a range that admits 0.3 it is refused with `EXTENSION_API_INCOMPATIBLE` at
`srelensApiVersion`, naming the field and the version it needs, rather than as an
unknown field. A 0.4 manifest may use everything 0.3 has, with the same meaning.

- Table columns on native resource lists may declare `joins` over a granted
  custom-resource reader and `tableColumns` with a row or joined `jsonPath`.
  `extensions.resolveColumns` batches up to 1,000 rows per request and reports
  failed reads explicitly (#538). See [Manifest reference](manifest.md#table-columns-and-joins).
- Apps may declare `detailPanels` with fields and conditions for matching
  resources. `extensions.resolvePanels` rechecks the installed app's revision,
  grants and cluster scope (#539). See the
  [manifest reference](manifest.md#detailpanels).
- Apps may declare `statusResolvers` for their custom-resource kinds and `badges` on
  built-in kinds, as first-hit rules resolving to six normalized statuses with a
  required word (#541). A badge reads its row's metadata or a declared join. The
  predicate path grammar gains one filter form, `[?(@.key=="text")]`, selecting the
  first matching element, which an action's `preconditions` and `availableWhen` may
  also use. `statusColumns` is deprecated. See the
  [manifest reference](manifest.md#status-resolvers-and-badges).
- The cluster overview draws `dashboardCards` (`count`, `countByStatus`, `metric`,
  `list`) over a granted custom-resource reader, with `equals`, `absent`, and date
  `within` / `before` predicates. `extensions.resolveCards` answers one app's cards per
  request, each with a figure or the reason it has none, and `extensions.read` takes a
  `card` to show a card's target page narrowed to what it counted (#540). See
  [Manifest reference](manifest.md#dashboard-cards).
- Apps may declare typed `settings` (`string`, `number`, `boolean`, `select`,
  `multi-select`, `url`, `namespace-selector`, `cluster-selector`, `secret-reference`),
  drawn as a host form in Settings → Apps and checked by the host on every save. A
  setting fills a binding argument as `"${settings.<id>}"` only where the host
  capability marks the argument settable (`k8s.annotate`'s `value`,
  `k8s.setStatusCondition`'s `message`), and is checked at install, on save and on
  every request. A `secret-reference` value never enters the inventory (#542): it is
  kept, write-only, in srelens's encrypted secrets vault through
  `extension.secretStore`, which an app declaring one must request as a permission
  (#543). An installed app's settings are held to its manifest's declarations, so
  values saved as free-form JSON by an earlier host that the manifest does not declare
  are refused on the next save. See [Manifest reference](manifest.md#settings) and
  [Secret settings](manifest.md#secret-settings).
- Apps may declare `commands` for the new design's command palette: open a declared
  page, or open the host confirmation for a declared action on a resource of the
  action's kind (#544). See the [manifest reference](manifest.md#commands).
- A custom-resource reader may list `versions` in preference order instead of one
  `arguments.version`, with optional per-version `jsonPathOverrides`. Each cluster reads
  the first listed version its CRD serves, through that version's paths, for every read,
  action and contribution, and a cluster serving none is refused (#547). A binding that
  fixes `arguments.version` means what it did before. See
  [Several served versions](manifest.md#several-served-versions).
- Apps may declare `resourceLinks` from one qualified kind to another their
  readers list, with a relation (`ownedBy`, `managedBy`, `exposedBy`,
  `references`) and a join-style match read on the linked-from resource (#545).
  `extensions.resolveLinks` rechecks the installed app's revision, grants and
  cluster scope; the Inspector shows the result as a Related section. See the
  [manifest reference](manifest.md#resourcelinks).
- The host supports API 0.3 and 0.4, and `extensions.catalog` reports both in
  `hostApiVersions` (#709). The signed 0.3 releases keep installing; see
  [Compatibility rules](#compatibility-rules).
- The manifest JSON Schema for this line is `schemas/extension-manifest.v0.4.json`.
  `schemas/extension-manifest.v0.3.json` is the 0.3 contract, without these fields.
- Current examples are Flux 0.5.0 and Argo CD 0.4.0, requiring `^0.4` and naming
  `schemas/extension-manifest.v0.4.json`. Publishing them requires fresh signed
  external releases and a catalog update; existing signed release bytes stay unchanged.
  The Flux example reads HelmReleases at `v2` or `v2beta2` and OCIRepositories at `v1`
  or `v1beta2` (#547).

### 0.3.0

- Unsigned apps declaring write actions require the default-off inventory policy
  described above (#558). Turning it off disables affected installations without
  removing them; normal read-only declarative permission grants are unchanged.
- API 0.1 and 0.2 are retired: their manifests are incompatible.
- Flux and Argo CD actions are declared by manifests, with their target reader,
  primitive write binding, confirmation metadata and availability predicates.
  The host no longer supplies a controller-specific action menu.
- The signed releases on this line are Argo CD 0.3.0 and Flux 0.4.0, requiring `^0.3`
  and naming `schemas/extension-manifest.v0.3.json`.
- App pages, dashboard card targets, palette page commands and Related links route by
  context key (`/extension-contexts/<key>/…`) and ask the host by the context's pinned
  ID, which `k8s.listContexts` now reports as `pinnedId`. So two contexts that share a
  stable ID open two tabs, each reading its own cluster. A context listed without a
  pinned ID (its kubeconfig path cannot be made absolute) says its apps cannot be opened.
  A route opened before (`/extension-clusters/<stableId>/…`) still opens while one context
  carries that ID, and says so when two do. Every app receives this; no manifest changes
  (#695).

### 0.1.0

The only supported API version. Apart from one pre-release rename (#537) and one
security fix (#603), the manifest contract has not changed since it was introduced; the
other entries are host additions that existing `^0.1` manifests receive without an
update.

- **#508:** API 0.1 manifests.
  - Contributions: `pages` (with `group`, `statusColumns`, `dashboard`), `detailTabs`, `rowActions` (renamed `detailLinks` in #537).
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
  - GitOps actions are offered only for the API versions listed in [capabilities.md](capabilities.md#declared-gitops-actions).
  - A Suspend of a suspended resource, or a Resume of one that is not suspended, is refused.
  - Resource events are newest first across every page read (up to 10 pages), ranking a recurring series by its latest occurrence, and capped at 100. `eventsTruncated`, `eventsPartial` and `eventsRead` say what was left out and how much was read.
  - An accepted action refreshes every open list, dashboard and detail view of that resource.
  - `k8s.gitOpsAction` is refused on the web host.
- **#530:**
  - The host declares a set of supported API versions and serves each manifest under the highest version its range matches.
  - A manifest whose range the host does not support is rejected with the versions it needs and the host supports, before strict schema checks.
  - A manifest may use only fields available in every supported API version its range admits (`API_FIELDS`, empty while 0.1 is the only version).
  - `extensions.catalog` also reports `hostApiVersions`, the full list. `hostApiVersion` stays, set to the newest supported version, and is deprecated (see [Deprecation](#deprecation)).
- **#531:**
  - The manifest JSON Schema is committed at `schemas/extension-manifest.v0.1.json`, and CI fails when it drifts from the host.
  - A manifest may name that schema in a top-level `$schema` key, which the host ignores.
- **#533:**
  - A rejected manifest reports every problem found, each with a stable `code`, the `path` of the value at fault and a `message` (see [Validation errors](#validation-errors)), instead of only the first problem as text.
  - `extensions.validate` returns those problems without installing, and the install review in Settings → Apps lists them before offering to install.
- **#534:**
  - Each installed app records its `source` (`local` or `catalog`), `installedAt`, and in `history` up to three versions it replaced, fewer when they would take the inventory past 1 MiB.
  - `extensions.configure` gains `rollback`, which restores a kept version with explicit grants, keeps settings and assigns a new revision.
  - Settings → Apps shows an app's manifest, grants with their annotations, source and install time; it exports settings as JSON, resets them to defaults, and rolls back.
  - The capability catalog carries a `sensitive` flag.
- **#535:**
  - An installed app may be limited to chosen kubeconfig contexts in `contexts`. Each is kept by context key (`{file}#{name}` with `#` and `%` encoded in each part, as `k8s.listContexts` reports under `key`), because a display name changes when another kubeconfig declares the same name (#265), and a stable ID can be shared by two contexts (#623). Without the list the app is offered on every cluster, as before.
  - `extensions.configure` gains `clusters`. On a cluster the app is not enabled for, `extensions.read`, `extensions.resource` and `extensions.action` refuse with "App is not enabled for this cluster", and both desktop designs hide its pages, detail tabs and detail links.
  - Those three send the request on under the checked context's pinned ID (the reserved `srelens-context:` prefix followed by the absolute kubeconfig path and encoded context name), and a context lookup accepts either ID as well as a name, so a kubeconfig change mid-request cannot move it to another cluster. Pinned requests never fall back to merged kubeconfig entries, and managed authentication resolves the pinned context's original name. Stable IDs themselves are unchanged, since settings persist them. A pinned ID names exactly one context (`#` and `%` in the path are percent-encoded); a stable ID that two contexts share (`a` + `b#c` and `a#b` + `c`) is refused for both, since it does not say which cluster was chosen.
  - A limited app's page waits while the contexts are listed, and shows the failure with a retry if the listing fails. Resource views do the same for a limited app's tabs and actions. A failed refresh keeps the contexts already known.
  - On a context the host cannot resolve, a limited app is refused with the reason (no kubeconfig declares it, or which kubeconfig could not be read), not with "App is not enabled for this cluster".
  - The terminal UI is recorded as out of scope for extension API 1.0 (see [Scope](#scope)).
- **#623:**
  - `k8s.listContexts` reports a `key` next to `stableId`: the stable ID with `#` and `%` percent-encoded in the file and the name, so no two contexts share it (`a` + `b#c` and `a#b` + `c` share a stable ID). The stable ID itself is unchanged.
  - An app's `contexts` list holds keys, and the broker checks a request's context by key. A context added later under a stable ID that a removed, allowed context carried cannot inherit the app.
- **#537:** pre-release rename.
  - The `rowActions` contribution is now `detailLinks`, with the same shape. Each entry opens a read-only results panel from the resource detail view's **App links** menu; it was never a row menu or a cluster write.
  - A manifest that still uses `rowActions` is rejected with `EXTENSION_UNKNOWN_FIELD`. There is no alias and API 0.1 is not bumped: a rename is breaking under [Compatibility rules](#compatibility-rules), and this one is an exception made because extensions had not gone live.
  - `rowActions` is reserved for declared mutations ([#549](https://github.com/srelens/srelens/issues/549)).
  - The official releases moved to `detailLinks` as Argo CD 0.2.0 and Flux 0.3.0, signed with a rotated srelens publisher key ([#560](https://github.com/srelens/srelens/issues/560)) that the host now pins. Signatures under the previous key no longer verify; nothing had been released under it, so no transition is kept.
- **#603:** security fix.
  - The app `name`, every `title` and a page `group` refuse Unicode format characters (category Cf), such as right-to-left overrides and zero-width spaces, with `EXTENSION_INVALID_VALUE` at the field's path. See [Identifiers](#identifiers).
  - A catalog entry whose `name` or `description` holds a control or format character is refused, and the catalog with it, as for any other invalid entry.
  - The install review shows a manifest's own name only once the host has accepted it; until then, and for one it refuses, it says "This manifest".
  - This narrows accepted values within API 0.1, which [Compatibility rules](#compatibility-rules) classify as breaking. The exception is made because such a label can display as a different app's name. An installed app whose label holds one fails re-verification and is quarantined.
- **#604:** security fix, in catalog validation rather than the manifest contract.
  - A catalog entry's `repository` is matched against the trusted-publisher table case-insensitively, as GitHub resolves owner and repository names. An entry whose repository is `https://github.com/SRELENS/…` must carry the srelens signature, exactly as the lowercase form must.
  - A lookalike owner such as `srelensx` is a different repository, and its entries stay ordinary unsigned third-party apps.
  - The release asset URL must still equal the pinned repository's `v<version>/manifest.json`, so an official entry writes it in the pinned form.
- **#601:** security fix.
  - A `k8s.listCustomResource` binding's `group` must be shaped like a CustomResourceDefinition group. A group without a dot (`apps`, `batch`, `policy`) or with an empty label is refused with `EXTENSION_INVALID_BINDING` at `capabilities[i].arguments.group`, by `extensions.validate`, install and rollback. Dotted groups under `k8s.io` are accepted here, since some, such as `gateway.networking.k8s.io`, are CRD groups.
  - An installed app whose binding breaks the rule fails re-verification and is quarantined.
  - `extensions.read`, `extensions.resource` and `extensions.action` confirm, before dispatching, that a CustomResourceDefinition named `{plural}.{group}` declares that group and plural and serves the bound `version` on the cluster, and refuse the call when none does. That covers dotted built-in groups such as `networking.k8s.io`, aggregated APIs, and a version of a CRD's group and plural that the CRD does not serve. A lookup that fails refuses the call with the reason, not as a missing CRD. `k8s.listCustomResource` itself is unchanged.
  - This narrows accepted values within API 0.1, which [Compatibility rules](#compatibility-rules) classify as breaking. The exception is made because such a binding exposed built-in objects, such as a Deployment's environment, under a permission that reads as custom resources only. The Flux and Argo CD releases are unaffected.
- **#549:** declared actions.
  - A manifest may declare up to 32 `actions`, each naming a host action primitive (`k8s.annotate`, `k8s.setFields`, `k8s.setStatusCondition`, `k8s.mergePatch`) and the reader binding whose kind it acts on. See [Declared actions](manifest.md#declared-actions). An existing manifest that declares none is unaffected, and the field is left out of the stored form when empty, so a signed manifest still verifies.
  - The host fills in the kind from that reader binding and fixes the inputs to `context`, `namespace`, `name`, `uid` and `resourceVersion`. An action that binds either is refused with `EXTENSION_INVALID_BINDING`, one that names an undeclared reader with `EXTENSION_UNRESOLVED_CAPABILITY`, and one that binds anything other than a primitive with `EXTENSION_UNSUPPORTED_TARGET`. An action's `name` shares the capability name space, so a repeat is `EXTENSION_DUPLICATE_IDENTIFIER`.
  - Each primitive re-reads the object, refuses a UID or `resourceVersion` that has moved and an object being deleted, pins its patch to both, and reports the request as accepted rather than as done. `$now` and `$uuid` are the only substitutions; any other `$` value is refused. `k8s.mergePatch` also enforces a host deny-list (`metadata.finalizers`, `ownerReferences`, `managedFields`, `uid`, `resourceVersion`, `status`, a Secret's values, and RBAC kinds).
  - Every rule above is applied when a manifest is validated, installed and reverified, and again on the way to the cluster.
  - All four primitives are refused on the web host, as `k8s.gitOpsAction` is, because nothing there scopes them to a kind and there is no confirmation.
  - Migrating the core Flux and Argo CD actions into manifests ([#551](https://github.com/srelens/srelens/issues/551)) and the host-owned confirmation ([#552](https://github.com/srelens/srelens/issues/552)) are not in this release. `extensions.action` and the host's own `supported_actions` table are unchanged.
- **#550:** declarative preconditions and availability.
  - A declared action may carry `preconditions` and `availableWhen`, each up to 8 predicates over the object, with the operators `equals`, `notEquals`, `present` and `absent` against a literal, addressed by a bounded JSONPath subset. See [Preconditions and availability](manifest.md#preconditions-and-availability). An action that declares neither behaves as before, and both are left out of the stored form when empty, so a signed manifest still verifies.
  - `preconditions` are evaluated by the host against the fresh read each primitive already performs, after the host's own guards and before the patch. One that does not hold refuses the request with the declared `reason`, escaped, framed by the host's own words. A malformed path, an unknown or repeated operator, a non-literal comparand and a missing `reason` are refused with `EXTENSION_INVALID_BINDING` when the manifest is validated, installed and reverified, and again on the way to the cluster.
  - `availableWhen` decides whether a control is offered and is not a guard: the host enforces nothing from it. Declaring a condition there and not in `preconditions` means the write is still accepted when the surface is out of date.
  - The host's guards are unchanged and unreachable from a manifest: an object being deleted, and a `uid` or `resourceVersion` that has moved, are refused before any declared predicate is read. A predicate can only add a refusal.
  - Binding `preconditions` or `availableWhen` inside an action's `arguments` is refused with `EXTENSION_INVALID_BINDING`; they are declared in the action's own fields.
