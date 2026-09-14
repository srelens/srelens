# Extension API specification

This is the normative contract between srelens and the apps (extensions) it hosts:
how the extension API is versioned, what may change within a version, and the rules
every manifest follows. The other pages in this directory describe how things work
today; where they and this page disagree, this page is the intent and the other page
is the bug.

Tracking: [#163](https://github.com/srelens/srelens/issues/163). Field reference:
[manifest.md](manifest.md).

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

A manifest that uses a field outside the version its range negotiates to is rejected,
even by a host that knows the field. That covers a field a later line added, and one a
later line removed or renamed. Otherwise the manifest would install on some hosts and
fail on others that still match its range.

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
- `rowActions` becomes `detailLinks`
  ([#537](https://github.com/srelens/srelens/issues/537)).

## Unknown fields

- **Manifests are strict.** An unknown field at any level is an error. Silently
  ignoring one would let an app look installed on a host that does not implement
  what it declares.
- **Catalog metadata is tolerant.** Hosts ignore catalog fields they do not
  recognize and still validate the fields they do. A breaking catalog change bumps
  the catalog's `schemaVersion` (currently `1`), which older hosts refuse.
- **Capability inputs are strict.** A payload with unknown fields is rejected. The
  wire names are camelCase.
- **The inventory is host-owned.** `settings.extensions.json` is not an authoring
  surface. Its format changes are listed in [migration.md](migration.md).

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
  unique across `pages`, `detailTabs` and `rowActions`. Both use `A–Z`, `a–z`, `0–9`
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

The only supported API version. The manifest contract has not changed since it was
introduced; the entries below are host additions that existing `^0.1` manifests
receive without an update.

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
  - GitOps actions are offered only for the API versions listed in [capabilities.md](capabilities.md#host-gitops-actions).
  - A Suspend of a suspended resource, or a Resume of one that is not suspended, is refused.
  - Resource events are newest first across every page read (up to 10 pages), ranking a recurring series by its latest occurrence, and capped at 100. `eventsTruncated`, `eventsPartial` and `eventsRead` say what was left out and how much was read.
  - An accepted action refreshes every open list, dashboard and detail view of that resource.
  - `k8s.gitOpsAction` is refused on the web host.
- **#530:**
  - The host declares a set of supported API versions and serves each manifest under the highest version its range matches.
  - A manifest whose range the host does not support is rejected with the versions it needs and the host supports, before strict schema checks.
  - A manifest may use only the fields of the API version its range negotiates to (`API_FIELDS`, empty while 0.1 is the only version).
  - `extensions.catalog` also reports `hostApiVersions`, the full list. `hostApiVersion` stays, set to the newest supported version, and is deprecated (see [Deprecation](#deprecation)).
