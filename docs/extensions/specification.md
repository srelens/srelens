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

## Terms

- **Extension API version**: the version of this contract, in SemVer form
  (`0.1.0`). It is independent of the srelens app version and of each app's own
  `version`.
- **Supported set**: the API versions a host implements, listed oldest first in
  `SUPPORTED_API_VERSIONS` (`crates/plugin-host/src/manifest.rs`). The catalog
  reports it as `hostApiVersions`.
- **API range**: a manifest's `srelensApiVersion`, a SemVer requirement in Cargo
  syntax, such as `^0.1` or `>=0.1, <0.3`.
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
   `^0.1` before 1.0, `^1.2` after. Under SemVer caret rules a `0.x` range pins its
   minor version, so `^0.1` does not match `0.2.0`. That is deliberate: each `0.MINOR`
   is its own compatibility line.
4. **New API versions.** Before 1.0, any manifest-visible change (a new field, a new
   contribution type, a new allowed capability target, a changed meaning) is a new
   `0.MINOR` version. A `0.MINOR.PATCH` bump is for clarifications that do not change
   which manifests validate. From 1.0: MAJOR for breaking changes, MINOR for
   additive ones, PATCH for fixes.
5. **Keeping old versions.** Adding a version to the supported set never removes an
   older one. An API version stays supported for **at least two srelens minor
   releases** after the release that ships its successor. For example, if API 0.2
   ships in srelens 0.10, API 0.1 remains supported through at least srelens 0.12.
   Retiring a version is announced in the [API changelog](#api-changelog) at least
   one srelens release before it happens.
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

The host enforces this for fields. `API_FIELDS` in `crates/plugin-host/src/manifest.rs`
lists every manifest field added or removed after API 0.1, with the API versions it is
available in. A rename is a removal plus an addition.

A manifest may use a field only if the field is available in every supported API
version its range admits, not just the one it negotiates to. Otherwise it is rejected,
even by a host that knows the field. That covers a field a later line added, one a later
line removed or renamed, and a range that spans several lines: `>=0.1, <0.3` claims 0.1
hosts, so it may not use a 0.2-only field. Otherwise the manifest would install on some
hosts and fail on others that still match its range.

The check runs at installation and again every time the inventory is loaded. An
installed app that uses a field a newer host's supported versions no longer admit is
quarantined rather than left enabled. A field that is null or an empty list or object
does not count as used.

A change that narrows the values a field accepts, rather than adding or removing the
field, must add a check keyed on the negotiated API version in the same change.

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
- **Names inside a manifest.** Capability `name`s are unique. Contribution `id`s are
  unique across `pages`, `detailTabs` and `detailLinks`. Both use `A–Z`, `a–z`, `0–9`
  and `-`, up to 64 characters. Titles are 1–120 characters with no control
  characters.
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

### 0.1.0

The only supported API version. Apart from one pre-release rename (#537), the manifest
contract has not changed since it was introduced; the other entries are host additions
that existing `^0.1` manifests receive without an update.

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
  - GitOps actions are offered only for the API versions listed in [capabilities.md](capabilities.md#host-gitops-actions).
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
  - An installed app may be limited to chosen kubeconfig contexts in `contexts`. Each is kept by stable ID (`{file}#{name}`: the declaring kubeconfig and the context's name in it), because a display name changes when another kubeconfig declares the same name (#265). Without the list the app is offered on every cluster, as before.
  - `extensions.configure` gains `clusters`. On a cluster the app is not enabled for, `extensions.read`, `extensions.resource` and `extensions.action` refuse with "App is not enabled for this cluster", and both desktop designs hide its pages, detail tabs and row actions.
  - The terminal UI is recorded as out of scope for extension API 1.0 (see [Scope](#scope)).
- **#537:** pre-release rename.
  - The `rowActions` contribution is now `detailLinks`, with the same shape. Each entry opens a read-only results panel from the resource detail view's **App links** menu; it was never a row menu or a cluster write.
  - A manifest that still uses `rowActions` is rejected with `EXTENSION_UNKNOWN_FIELD`. There is no alias and API 0.1 is not bumped: a rename is breaking under [Compatibility rules](#compatibility-rules), and this one is an exception made because extensions had not gone live.
  - `rowActions` is reserved for declared mutations ([#549](https://github.com/srelens/srelens/issues/549)).
  - The official releases moved to `detailLinks` as Argo CD 0.2.0 and Flux 0.3.0, signed with a rotated srelens publisher key ([#560](https://github.com/srelens/srelens/issues/560)) that the host now pins. Signatures under the previous key no longer verify; nothing had been released under it, so no transition is kept.
