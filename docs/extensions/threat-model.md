# Threat model

What the extension platform defends against, how, and what is left. [security.md](security.md)
is the short version; this page is the reasoning behind it, with each mitigation tied to
the code that enforces it. It covers the platform as it is on `dev` (extension API 0.1,
declarative apps only) and feeds the pre-1.0 security review
([#39](https://github.com/srelens/srelens/issues/39)).

Tracking: [#579](https://github.com/srelens/srelens/issues/579), part of
[#523](https://github.com/srelens/srelens/issues/523).

## How to read this page

Every mitigation carries a status:

- **Shipped:** on `dev`, enforced by the code named beside it.
- **Pending:** implemented in an open pull request, not on `dev`.
- **Planned:** tracked by the linked issue, not enforced today.
- **Gap:** a weakness with no issue yet.

Each threat is tagged with its STRIDE classes: **S**poofing, **T**ampering,
**R**epudiation, **I**nformation disclosure, **D**enial of service and **E**levation of
privilege.

Code paths are relative to the repository root and name the function or constant to look
for. Line numbers are left out because they drift.

## Scope

In scope:

- The desktop app, in both designs, managing apps through Settings → Apps.
- Every MCP surface that carries `extensions.*`: the desktop's in-app loopback server and
  native agent, and the headless `--mcp-stdio` and `--mcp-http` modes
  (`apps/desktop/src-tauri/src/main.rs`). Each builds its registry with the desktop
  settings path (`build_registry_with` in `crates/registry/src/lib.rs`).
- The catalog, catalog downloads and signed official releases.
- The inventory and the catalog cache on disk.
- The multi-user web host, which refuses apps.

Out of scope, and assumed:

- **The user's account is not compromised.** Anything that can write the user's files can
  already edit their kubeconfig. The inventory checks below limit what a tampered file can
  do; they are not a defence against local malware.
- **srelens itself is trusted:** its build, release and update chain, the Tauri shell and
  the WebView code. The renderer runs only srelens code, which is what makes a
  host-rendered review meaningful. The WebView calls capabilities without a backend
  consent gate (`invoke_capability` in `apps/desktop/src-tauri/src/bridge.rs`), and the
  Tauri CSP is `null` (`apps/desktop/src-tauri/tauri.conf.json`), so a renderer injection
  would bypass every review. Both belong to [#39].
- **Kubernetes enforces RBAC** for the credentials the user chose. srelens adds no
  authorization of its own; it only avoids handing those credentials to apps.
- **TLS and GitHub** authenticate `raw.githubusercontent.com`, `github.com` and
  `release-assets.githubusercontent.com`.

## Assets

| Asset | Where it lives | Why it matters |
|---|---|---|
| Cluster credentials | Kubeconfig files, exec plugins and tokens, resolved by `ClientCache` (`crates/kube/src/client_cache.rs`) | Whoever holds them acts as the user on the cluster. |
| Cluster state | The clusters | A write changes what runs. |
| Cluster data | The clusters, read under RBAC | Includes Secrets, and whatever workloads carry in their specs. |
| App inventory | `settings.extensions.json`, next to the desktop settings file | Which apps are installed and enabled, their grants, settings, signature proofs and kept versions. |
| Catalog cache | `*.extensions.catalog.json`, next to the inventory | What **Review installation** offers, and whether an install is recorded as `catalog`. |
| Trust anchors | `PUBLISHERS` and `srelens-apps.pub` in `crates/registry/src/extensions/` | Which apps are official. The private key is a GitHub Actions secret in the app repositories ([distribution.md](distribution.md#releasing-an-official-app)). |
| Consent | Install and action reviews in the UI; the MCP consent gate | The user's decision is the only thing that grants an app access or starts a write. |
| Host UI integrity | The WebView | The user trusts that a srelens dialog means what it says. |

## Actors

| Actor | Trust | Supplies |
|---|---|---|
| User | Trusted | Decisions in reviews and dialogs; pasted manifests. |
| App author | Untrusted | A manifest, pasted locally or listed in the catalog. |
| srelens publisher | Trusted; its key may be stolen | Signed official releases. |
| Catalog maintainers | Trusted to list apps, not to vouch for them | `catalog.json` on the `main` branch of [srelens/extensions](https://github.com/srelens/extensions). |
| Network attacker | Untrusted | Anything on the wire: responses, DNS answers, redirects. |
| MCP client or agent | Authenticated, but may act on injected instructions | Any tool call. |
| Other web user | Untrusted with respect to you | Requests to the same web host. |

## Trust boundaries

1. **Manifest bytes to host.** Untrusted JSON becomes a `Manifest` only through
   `Manifest::decode` and `Manifest::validate` (`crates/plugin-host/src/manifest.rs`), and
   an installable app only through `validate_app` and `check_install`
   (`crates/registry/src/extensions.rs`).
2. **App to host capability.** A binding reaches a host capability only through the broker
   (`PluginHost::register` in `crates/plugin-host/src/lib.rs`), with fixed arguments and
   the host's own schema and annotations.
3. **Host to cluster.** Every call runs with the user's credentials for the context it
   names, under that context's RBAC.
4. **Network to host.** Catalog, manifest and signature bytes arrive only through
   `download` in `crates/registry/src/extensions/catalog.rs`.
5. **Disk to host.** The inventory and the catalog cache are parsed and checked again on
   every load (`read` in `crates/registry/src/extensions.rs`, `load_with` in
   `crates/registry/src/extensions/catalog.rs`).
6. **Caller to registry.** The WebView calls capabilities through `invoke_capability`
   (`apps/desktop/src-tauri/src/bridge.rs`), MCP clients through the consent-gated
   `handle_request` (`crates/mcp/src/stdio.rs`), and web users through
   `invoke_capability` (`crates/server/src/api.rs`).

## Entry points

| Entry point | Untrusted input | Mutating | Code |
|---|---|---|---|
| `extensions.validate` | Manifest, grants, signature | No | `check_install` in `crates/registry/src/extensions.rs` |
| `extensions.configure` | Manifest, grants, signature; app ID; settings JSON; rollback revision | Yes | `mutate` in `crates/registry/src/extensions.rs` |
| `extensions.list` | None | No | `read` in `crates/registry/src/extensions.rs` |
| `extensions.read` | App ID, revision, operation, context, namespace | No | `register` in `crates/registry/src/extensions.rs` |
| `extensions.resource` | App ID, revision, operation, context, namespace, name | No | `resolve` in `crates/registry/src/extensions/resource.rs` |
| `extensions.action` | As `extensions.resource`, plus action, UID and resourceVersion | Yes | `resolve` in `crates/registry/src/extensions/resource.rs`, `request` in `crates/kube/src/action_primitives.rs` |
| `extensions.catalog` | `catalog.json` from the network | No | `load_with` and `parse_catalog` in `crates/registry/src/extensions/catalog.rs` |
| `extensions.catalogManifest` | App ID and SHA-256; manifest and signature from the network | No | `verify_release` in `crates/registry/src/extensions/catalog.rs` |
| Inventory load | `settings.extensions.json` | — | `read` and `reverify` in `crates/registry/src/extensions.rs` |
| Catalog cache load | `*.extensions.catalog.json` | — | `load_with` in `crates/registry/src/extensions/catalog.rs` |

`extensions.action` dispatches only the action declared for the selected reader,
through a granted host primitive. The generic primitives are also registry capabilities
and require consent; their direct invocation does not inherit an app's identity.

## Threats and mitigations

### Malicious app

An author who wants an app to do more than show custom resources.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| APP-1 | Read kubeconfig, tokens or local files | I | No app code is loaded or run. `ManifestKind` has one variant, `declarative` (`crates/plugin-host/src/manifest.rs`), and the broker (`crates/plugin-host/src/lib.rs`) only forwards JSON arguments to host handlers. A manifest has no field that names a file, URL or command, and unknown fields are refused (`deny_unknown_fields`). The host resolves credentials from the context name; the app never sees them. | Shipped |
| APP-2 | Reach arbitrary network endpoints or run commands | I, E | As APP-1. Desktop bindings may target only `k8s.listCustomResource` and `k8s.listEvents` (`validate_app` in `crates/registry/src/extensions.rs`), and no binding may target another app's `plugin/` capability (`Manifest::validate`). | Shipped. Brokered network access planned in [#568] |
| APP-3 | Read Secrets or other core resources through the reader | I | `validate_app` (`crates/registry/src/extensions.rs`) requires a fixed, non-empty `group`, `version`, `plural` and `kind` of letters, digits, `.` and `-`, so the core group (Secrets, ConfigMaps, Pods) cannot be bound. The group must also be shaped like a CustomResourceDefinition group, with a dot and no empty label, so built-in groups such as `apps` and `batch` are refused (`group_problems` in `crates/registry/src/extensions/crd.rs`). The same rule runs when the inventory loads, so a stored app that breaks it is quarantined. Before dispatching, `extensions.read`, `extensions.resource` and `extensions.action` confirm that a CustomResourceDefinition named `{plural}.{group}` declares that group and plural and serves the bound version on the cluster (`require` in the same file, `custom_resource_serves` in `crates/kube/src/crds.rs`). That refuses dotted built-in groups such as `networking.k8s.io`, aggregated APIs, and a version the CRD does not serve; a failed lookup refuses the call and says so. `k8s.listCustomResource` returns names, namespaces, ages and the declared printer columns (`list_custom_resource_capability` in `crates/kube/src/crds.rs`). | Shipped. Built-in groups refused in [#601]; bindings shown in the install review by [#608]. See residual risk |
| APP-4 | Widen a read by overriding bound arguments | T, E | The broker refuses any input not listed in the binding's `inputs` and any missing required one, and the schema it exposes sets `additionalProperties: false` (`PluginHost::register`). Fixed `arguments` are merged into every call, and inputs may not overlap them, so a caller cannot override one. `extensions.read` forwards only `context` and `namespace` and checks the namespace's syntax; `validate_app` refuses a binding that fixes either. | Shipped |
| APP-5 | Write to the cluster, or dispatch an operation that needs consent | E | `validate_app` refuses a target that is not read-only or carries `requires_confirm`, `sensitive` or `destructive`. A manifest cannot supply annotations: the broker copies the host's and forces `requires_confirm` on anything not read-only, sensitive or destructive. A manifest declares `actions` ([#549]), which bind one of four host action primitives (`PRIMITIVES` in `crates/kube/src/action_primitives.rs`) and nothing else (`validate_app`). The host copies the kind from a reader binding in the same manifest and fixes the inputs to the reviewed object (`Manifest::action_binding`, `ACTION_INPUTS`), so an action reaches no kind the app lacks a granted reader for; what it writes is fixed at install and checked by the primitive itself, at install, at reverification and again on the way to the cluster (`check_bound_arguments`, run from `PluginHost::problems_at` and from each handler). `k8s.mergePatch` refuses `metadata.finalizers`, `ownerReferences`, `managedFields`, the pin fields, `status`, a Secret's values and RBAC kinds. Every primitive re-reads the object, refuses a moved UID or resourceVersion and one being deleted (`guard_reviewed`), and pins its PATCH to both. | Shipped ([#549]). An opt-in for unsigned apps that write in [#558] |
| APP-6 | Keep acting after being disabled, removed, updated or quarantined | E | `extensions.read`, `extensions.resource` and `extensions.action` read the inventory on every call, and require the app to be enabled, at the caller's revision, and to pass `validate_app` with its stored grants. `Registration::unregister` revokes the handlers older registry snapshots still hold. Calls already admitted may finish. | Shipped |
| APP-7 | Use a permission it was not granted | E | `permissions` must name exactly the bound targets (`EXTENSION_PERMISSION_MISMATCH` in `Manifest::validate`), and each one must be among the grants the caller supplied (`validate_app`, `PluginHost::register`). | Shipped |
| APP-8 | Escalate through an update or rollback | E | An update is a new `extensions.configure` install, which is mutating and carries its own grants; nothing updates automatically. A rollback verifies the kept version's signature again and runs `validate_app` with the grants given now (`Configure::Rollback` in `crates/registry/src/extensions.rs`). The review shows the full permission list again, not what changed. | Shipped. Permission diff planned in [#554]; update checks and downgrade protection in [#563] |
| APP-9 | Pose as an official app | S | IDs under `org.srelens.` install only with the srelens signature (`check_install` in `crates/registry/src/extensions.rs`, `reserved` in `crates/registry/src/extensions/signing.rs`), so an unsigned install cannot take an official ID or replace a signed app. An unsigned entry already stored under one, such as an app installed before [#528] reserved the namespace, is quarantined when the inventory loads, so it cannot be enabled, and no unsigned kept version under one is restored (`unsigned_reserved`, `reverify` and the `rollback` action in `crates/registry/src/extensions.rs`; shipped by [#602]). Bundled logos are chosen by ID (`packages/ui-next/src/extensions/ExtensionLogo.tsx`). Settings → Apps labels each app **Unsigned local**, **Signed by srelens** or **Signature not verified** (`packages/ui-next/src/extensions/Extensions.tsx`). | Shipped. See residual risk |
| APP-10 | Spoof host UI or dialogs | S | Apps contribute data, never markup: pages, detail tabs and row actions render with host components, and the frontend renders no text as raw HTML. Names, titles and groups are 1–120 characters with no control characters and no format characters (category Cf), such as right-to-left overrides and zero-width spaces, so none displays differently from what it holds (`label` and `is_format_character` in `crates/plugin-host/src/manifest.rs`, [#603]). Catalog names and descriptions refuse the same control and format characters (`parse_catalog` in `crates/registry/src/extensions/catalog.rs`). The install review renders a manifest's own name only once the host has accepted it, and says "This manifest" for one still being checked or refused (`ExtensionManager` in `packages/ui-next/src/extensions/Extensions.tsx`). Values the host does not restrict, such as printer column names and JSON paths, are drawn in the review's binding summary with format, control and line-separator characters written as escapes, and the review's manifest view escapes format characters as Details does (`plainText` and `escapeFormatCharacters` in `packages/ui-next/src/extensions/displayText.ts`, [#608]). Install and action reviews are host-owned (`packages/ui-next/src/extensions/Extensions.tsx`, `packages/ui-next/src/extensions/ExtensionResourceDetails.tsx`). | Shipped. See residual risk |
| APP-11 | Read clusters the user did not intend the app for | I | Every read names an explicit context and runs under that context's RBAC. Installation is app-wide, so by default an enabled app can read any cluster the user opens it on. An optional per-app cluster allow-list narrows that: the `clusters` action of `extensions.configure` limits an app to 1–256 named contexts, or with an explicit `null` allows every cluster — leaving the list out is refused rather than read as "every cluster" (`Configure::Clusters` in `crates/registry/src/extensions.rs`). It is enforced on `extensions.read`, `extensions.resource` and `extensions.action` by one check (`Installed::check_scope`, called from `extensions.rs` and `crates/registry/src/extensions/resource.rs`), which refuses a limited app on any other context and refuses it equally when the host could not resolve the context at all, saying so rather than guessing. The list is held by each context's `key` (`ResolvedContext::key`), so a same-named context in another kubeconfig cannot inherit the access and no two contexts share an entry, and an update keeps the list as it keeps settings. | Shipped ([#597], [#535]) |
| APP-12 | Exhaust the host | D | What an app declares is bounded. A manifest is at most 256 KiB (`MAX_MANIFEST_BYTES` in `crates/plugin-host/src/manifest.rs`), with 1–32 capabilities, at most 32 printer columns per binding (`MAX_PRINTER_COLUMNS`), at most 64 contributions, 1–32 kinds per detail tab or row action, and 1–12 pages per dashboard. The inventory is at most 1 MiB and keeps at most three replaced versions per app. Single-resource inspection reads at most 10 pages of 500 events (`list_events` in `crates/kube/src/gitops.rs`). App pages and dashboards list through `list_capped` (`crates/kube/src/list_cap.rs`): at most 2,000 rows from `k8s.listCustomResource` and `k8s.listEvents`, with a `truncated` marker when more remain. | Manifest, inventory and app-read limits shipped ([#609]). Performance budgets in [#581] |

Residual risk:

- **Any custom resource is readable in full.** Since [#601] a binding reaches only a
  version a CustomResourceDefinition serves, never a built-in or aggregated API, but it may name any CRD
  on the cluster, not just the ones the app is about. Its objects are exposed in two
  ways:
  - **Lists:** `k8s.listCustomResource` (`list_custom_resource_capability` in
    `crates/kube/src/crds.rs`) renders the binding's `printerColumns` JSON paths. These
    can surface any scalar field of the custom resource.
  - **Whole objects:** `extensions.resource` resolves the same binding (`resolve` in
    `crates/registry/src/extensions/resource.rs`) and calls `k8s.getCustomResource`. That
    returns the complete object, with only `managedFields` removed (`inspect` in
    `crates/kube/src/gitops.rs`). The Manifest tab shows all of it
    (`packages/ui-next/src/extensions/ExtensionResourceDetails.tsx`), and an MCP client
    can request it without consent. A custom resource can hold values as sensitive as a
    built-in one, for example a CRD whose spec embeds credentials.

  RBAC still applies, and the install review names the custom resources and columns each
  binding reads (see the next point).
- **Grants are per host capability, not per binding.** Granting `k8s.listCustomResource`
  grants whatever the manifest binds, so the install review shows what that is
  ([#608]). The review (`ExtensionManager` in
  `packages/ui-next/src/extensions/Extensions.tsx`, which the classic design reuses
  through `apps/desktop/src/components/Extensions.tsx`) shows the app's name, a signature
  label, the requested capability IDs and any validation problems.
  - **Bindings:** once the host has accepted the manifest, the review lists each
    custom-resource reader's group, version, kind, plural, scope and printer columns
    with their JSON paths; each event reader's API groups, per dashboard that shows its
    events; and any other binding's fixed arguments (`ExtensionBindings` in
    `packages/ui-next/src/extensions/ExtensionBindings.tsx`).
  - **Manifest:** **View manifest** opens the full manifest before anything is
    installed, for catalog and pasted installs alike. A catalog install's manifest is
    otherwise never shown before it is installed; the catalog lists only the app's name,
    description, ID, versions and license
    (`packages/ui-next/src/extensions/ExtensionCatalog.tsx`). An unsigned catalog app is
    labelled **Unsigned local manifest** in the review.
  - **After install:** the full manifest can be read under Details
    (`packages/ui-next/src/extensions/ExtensionDetails.tsx`).

  The review says what is bound, not whether it matters: it names the CRDs an app reads
  but cannot tell whether their objects hold anything sensitive, and nothing makes the
  user read it. [#554] adds a diff for updates.
- **App reads are capped.** An app page lists with `k8s.listCustomResource`
  (`list_custom_resource_capability` in `crates/kube/src/crds.rs`), and a dashboard reads
  events with `k8s.listEvents` (`list_events_capability` in `crates/kube/src/events.rs`).
  Both page with `limit`/`continue` and stop at 2,000 rows (`list_capped` in
  `crates/kube/src/list_cap.rs`), returning `truncated` when more remain. A binding may
  declare at most 32 printer columns (`MAX_PRINTER_COLUMNS` in
  `crates/plugin-host/src/manifest.rs`). The UI pages the returned rows and says how many
  are not shown (`ExtensionResults.tsx`, `ExtensionWorkspace.tsx`). `request_timeout` still
  bounds only wait time, not the cost of a full capped page that arrives in time — see
  [#581].
- **Unsigned apps choose their own names.** A local app outside the reserved namespace can
  call itself "Argo CD". It gets an initials mark and the **Unsigned local** label, not the
  bundled logo.
- **Labels can still look alike.** Format characters are refused, but letters are not
  compared across scripts. A name that spells "Argo CD" with a Cyrillic "А", or holds an
  invisible letter such as the Hangul filler (U+3164), still validates.

### Compromised publisher or key

An attacker who holds the srelens app signing key, or controls a release repository.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| PUB-1 | Forge an official release without the key | S, T | Official releases carry a detached Ed25519 signature over the exact manifest bytes, checked against a key compiled into the host (`PUBLISHERS` and `verify_key` in `crates/registry/src/extensions/signing.rs`). Catalog metadata cannot supply a key. Each official app ID is pinned to one repository, and its manifest must be that repository's `v<version>/manifest.json` release asset (`signature_url` in `crates/registry/src/extensions/catalog.rs`). The repository is compared case-insensitively, as GitHub resolves it ([#604]); the asset URL must match exactly. A catalog entry that names a reserved ID or a srelens repository must be signed. | Shipped |
| PUB-2 | Alter a signed installation after install | T | The verified bytes and signature are stored as `signatureProof`. Every inventory load verifies them again and checks that they parse to the installed manifest (`reverify`, `verify_proof` in `crates/registry/src/extensions.rs`); an app that fails is quarantined on its own. | Shipped |
| PUB-3 | Use a stolen key | S, E | A manifest signed with a stolen key installs as official on every host that trusts the key. It is still a declarative app, held to the rules above and to the user's permission review. A host release that removes the key quarantines every app it signed and leaves the rest working. | Quarantine shipped. Rotation planned in [#560]; revocation and a kill switch in [#561] |
| PUB-4 | Reinstall an older, vulnerable signed release | T | A validly signed older manifest still installs through `extensions.configure`, and a rollback restores a kept one. Both need the user's review, and nothing warns about the older version. | Planned: revocation in [#561], downgrade protection in [#563] |
| PUB-5 | Sign apps as an unknown publisher | S | A signature on an app ID with no trusted publisher is refused (`verify_for`). Third-party apps install unsigned and are labelled so. | Shipped. Publisher delegation planned in [#559] |

Residual risk:

- One key signs every official app, and replacing it requires a host release ([#560]).
- The private key is protected by GitHub Actions secrets in the app repositories. Its
  handling is outside this repository.

### Vulnerable app

An honest app with a flaw, or a host bug that an app's input can reach.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| VULN-1 | A flawed app is hijacked to run code, open sockets or read files | E | Not possible in API 0.1: there is no app code to hijack, and the manifest type admits no executable kind. Executable apps are to ship only with an OS sandbox, and unsigned ones only behind an explicit setting. The sandbox spike ([#571]) found that AppContainer plus a Job Object on Windows, and Landlock plus seccomp plus cgroup v2 on Linux, each enforced all seven of its checks on ordinary operations. macOS has a candidate Seatbelt backend in the spike, which is not yet verified: on its first run (macOS 27.0 arm64) the profile never let the sidecar start. That run also showed that `setrlimit` refuses a memory limit there, and the research found no CPU rate limit. The findings, the support matrix and the proposed backends are in [Sandbox backends for executable extensions](../design/plugin-architecture.md#sandbox-backends-for-executable-extensions-proposed), marked Proposed. None of this is enforced today, and none of it has been reviewed against deliberate escapes. | Planned: supervisor, and its escape-hardening review, in [#572] (epic [#521]); untrusted-source policy in [#558] |
| VULN-2 | A malformed or oversized manifest, catalog, signature or inventory crashes or exhausts the host | T, D | Parsers are Rust and `serde`. Downloads and the catalog cache are read only up to their limits, so an oversized one is refused before it fills memory: catalogs 1 MiB and downloaded signatures 64 bytes (`download` and `load_with` in `crates/registry/src/extensions/catalog.rs`, then `parse_catalog`). The inventory is read only to one byte past 1 MiB, so an oversized, corrupt or tampered file is refused before it is loaded whole or parsed (`read` in `crates/registry/src/extensions.rs`); the same limit bounds every inventory the host writes. A manifest string is checked against 256 KiB before it is decoded (`Manifest::decode` in `crates/plugin-host/src/manifest.rs`). Caller-supplied capability inputs are bounded twice. First the transport: an MCP request is at most 4 MiB (`MAX_REQUEST_BYTES` in `crates/mcp/src/lib.rs`). The HTTP router sets that as its body limit explicitly, refusing a larger body with 413 (`router_inner_with_push` in `crates/mcp/src/http.rs`), and stdio reads each request line through a bounded reader that drops a longer line as it arrives, never holding it, and answers with a JSON-RPC error naming the limit (`BoundedLines` in `crates/mcp/src/stdio.rs`, fed from stdin by `run_mcp_stdio` in `apps/desktop/src-tauri/src/main.rs`). Then the fields, while the arguments are decoded (`crates/registry/src/extensions/limits.rs`): on `extensions.validate` and `extensions.configure`, a `signature` is refused at its 65th byte or when shorter than 64, a `manifest` over 256 KiB is refused before it is decoded, and a `settings` object over 64 KiB as compact JSON is refused before it is saved. Each refusal is an invalid-input error naming the field and its limit. The desktop bridge sets no transport limit (`invoke_capability` in `apps/desktop/src-tauri/src/bridge.rs`): its only caller is the app's own WebView, whose request is already in the process's memory when the command runs, and the field limits apply to it as to MCP. Every problem found in a manifest is reported with a stable code and path (`crates/plugin-host/src/validation.rs`). | Limits on downloads, the catalog cache, the inventory read and caller-supplied inputs shipped ([#610]); fuzzing planned in [#580] |
| VULN-3 | An app's settings leak a credential | I | Settings are typed by the manifest (#542), and a credential belongs in a `secret-reference` setting, whose value never enters the inventory. Every save is held to the declarations (`Manifest::check_setting_values` in `crates/plugin-host/src/manifest/settings.rs`, called from `checked_settings` in `crates/registry/src/extensions/app_settings.rs`), and any value for a `secret-reference`, even its reference, is refused. A `url` setting refuses a user name or password. The inventory writer refuses to save a secret setting that holds anything but its host-minted reference (`saved_form` in `crates/registry/src/extensions.rs`), whichever path put it there, and loading drops one that does. An update or rollback keeps only the values the new manifest accepts, so a string setting that becomes a secret loses its plaintext (`Manifest::retain_settings`). No refusal repeats the value it refused. A setting reaches a capability only through a binding argument the capability marks settable, and never a secret (`PluginHost::interpolate`). Other settings are still plain text in two places. The inventory stores them, and `extensions.list` returns them. An `extensions.configure` call is also recorded in the local audit log, from MCP or from Settings → Apps (#555) (`audit.jsonl`, created with mode 0600 on Unix), whether consent is granted or denied. The capability is not sensitive-annotated, so `redact` (`crates/capability/src/audit.rs`) removes values by key name, which does not catch a setting named `credential` or `certificate`; the `settings` map is therefore redacted as a whole, keeping the action, the app ID and the setting names and blanking every value at any depth, and the recorded error is scrubbed of the same values, since a refused argument tends to be echoed by the refusal ([#605]). Other `extensions.configure` actions are recorded as before. Settings saved from Settings → Apps go through the same capability and so through the same redaction: the app ID and the setting names are recorded, never a value. | Audit-log redaction of `settings` shipped ([#605]). Typed settings, with secret values refused from the inventory, shipped ([#542]). Keychain storage for `secret-reference` values planned in [#543]; until then a secret setting cannot be set |
| VULN-4 | An app is slow on a large cluster | D | Each cluster request is bounded in time by `request_timeout` (`crates/kube/src/connect.rs`), 8 seconds by default and configurable from 1 to 120. App-reader lists are also bounded in size by `APP_LIST_CAP` (`crates/kube/src/list_cap.rs`). | App-read row caps shipped ([#609]). Performance budgets planned in [#581] |

### Malicious catalog or network position

An attacker who can alter traffic, or who controls `catalog.json`.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| NET-1 | Intercept or redirect a download | T, S | Every download is HTTPS with no credentials, fragment or custom port (`https_url`). Only the fixed `CATALOG_URL`, `github.com/<owner>/<repo>/releases/download/…` and `release-assets.githubusercontent.com` are fetched, and every redirect is checked against the same list, at most four of them (`allowed_download`, `download` in `crates/registry/src/extensions/catalog.rs`). Requests time out after 20 seconds, 10 to connect, and a body is read only to one byte past its limit. | Shipped |
| NET-2 | Swap the manifest between listing and review | T | The catalog pins each release's SHA-256. The downloaded bytes must match it, and their ID, version and API range must equal the entry's (`verify_manifest`). Review names the release by ID and checksum, so a changed catalog needs a new review. | Shipped |
| NET-3 | List a malicious or look-alike app from a compromised catalog | S, T | The catalog is not signed; its integrity rests on TLS and on control of the `srelens/extensions` repository. It cannot make an app official, because reserved IDs and srelens repositories need the pinned signature (PUB-1), whatever the case of the repository URL ([#604]). Every entry is validated (`parse_catalog`), and every manifest passes the same rules and permission review as a pasted one. | Signed catalog planned in [#559] |
| NET-4 | Freeze or roll back the catalog | T, D | A failed refresh keeps the cached catalog, marks it stale and shows the error with the original time (`load_with`). A `schemaVersion` other than 1 is refused. Nothing detects an older catalog that is still well formed. | Planned: signed catalog in [#559], revocation in [#561] |
| NET-5 | Alter the catalog cache on disk | T | A cached catalog is validated again on every load, and one that fails is fetched again. `source: catalog` only records that the installed bytes match a cached release; it grants nothing. | Shipped |

Residual risk:

- A compromised catalog can list an unsigned app with any name, description and
  repository link, served from any GitHub release. It is labelled unsigned and still needs
  the user's review.

### MCP client abuse

An agent that is connected and authenticated, but acting on bad instructions.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| MCP-1 | Connect without authorization | S | `/mcp` always requires a bearer token, compared in constant time, and no production constructor serves without one (`router_with_auth` and `token_guard` in `crates/mcp/src/http.rs`, `crates/mcp/src/auth.rs`). The desktop's in-app server binds `127.0.0.1` only (`start_server` in `apps/desktop/src-tauri/src/mcp.rs`). Headless `--mcp-http` defaults to `127.0.0.1:8765` and refuses a non-loopback address with an error naming it, before the vault is opened or a token minted, unless the process was started with `--mcp-expose-http`; the listener `serve_http` accepts can only come from that check (`check_bind_addr` and `HttpListener::bind` in `crates/mcp/src/http.rs`, `run_mcp_http` in `apps/desktop/src-tauri/src/main.rs`). The startup message reports the address actually bound and says when it is exposed. Every route rejects a `Host` header that is not a loopback IP or `localhost` (`host_guard`), the same loopback test the bind uses; an exposed listener also accepts any IP-literal `Host`, never a hostname. That stops DNS rebinding from a browser, but it does not restrict where a request comes from, because any client can send `Host: localhost`; the bind is the network boundary. See [MCP.md](../MCP.md#security-model). | Shipped ([#607]). See residual risk |
| MCP-2 | Install or enable an app, change its grants or settings, or roll it back | E | `extensions.configure` is mutating, so `handle_request` (`crates/mcp/src/stdio.rs`) asks the consent policy first (`consent_kind` in `crates/mcp/src/lib.rs`). In the desktop app that is a dialog (`PromptUser` in `apps/desktop/src-tauri/src/mcp_confirm.rs`). Headless, it needs both `--mcp-allow-destructive` and `"_confirm": true` (`FlagGated` in `crates/mcp/src/policy.rs`). With no policy, it is denied (`AlwaysDeny`). | Shipped |
| MCP-3 | Start a GitOps write | E | `extensions.action` and all action primitives are mutating and consent-gated (`action_dispatch_uses_bound_api_and_mcp_cannot_bypass_confirmation` in `crates/registry/src/extensions/resource.rs`). The write fetches the resource again and refuses a changed UID or resourceVersion, a sync while an Argo CD operation is present, a Suspend of a suspended resource or a Resume of one that is not, reconciliation while suspended, and a resource being deleted. It then sends the UID and resourceVersion as PATCH preconditions (declared predicates and `request` in `crates/kube/src/action_primitives.rs`). A sync never enables pruning. | Shipped |
| MCP-4 | Call a removed app through a stale tool list | E | Broker handlers check the flag `Registration::unregister` clears, and `extensions.*` read the inventory on every call. | Shipped |
| MCP-5 | Deny having made a call | R | Capability calls are recorded, best-effort, in a local JSONL audit log with the source, the consent decision, the outcome and redacted arguments (`JsonlAuditLog` and `redact` in `crates/capability/src/audit.rs`). The record is written by the registry itself (`Registry::invoke_audited`), which is where the two calling surfaces meet, so both are covered: MCP through `handle_request` (`crates/mcp/src/stdio.rs`), and the desktop UI through `invoke_capability` (`apps/desktop/src-tauri/src/bridge.rs`), which used to reach the registry directly and leave nothing behind. MCP records every call it handles; from the UI, mutating and sensitive capabilities are recorded and plain reads are not (`is_audited_from_ui`). Each record names the app and revision it went through, the cluster and the object. Recording fails open by design, so that a lost log line never breaks a cluster operation. A failed rotation, write or permission change is swallowed, a failed open is only reported on stderr, and the call goes ahead either way. An executed call is recorded after it returns. | Shipped ([#555]) |

Residual risk:

- **Headless `--mcp-http` with `--mcp-expose-http` is reachable from the network over
  plain HTTP.** The flag is the operator's explicit choice, the startup message says the
  listener is exposed, and [MCP.md](../MCP.md#security-model) states the risk, but nothing
  in srelens reduces it: there is no TLS, so anyone who can observe the traffic can read
  the bearer token and every tool result, including cluster data, and replay the token;
  anyone who can reach the address and holds the token can call every tool the process
  allows, and `/healthz` answers them without one. Gated tools still need the process
  flags and `_confirm`. The documented alternative is to keep the loopback bind behind an
  SSH tunnel or a TLS-terminating reverse proxy.
- With `--mcp-allow-destructive`, a headless agent that sends `_confirm` can install any
  unsigned read-only app with the grants it asks for. [#558] covers apps that write or run
  code, not read-only ones.
- Read-only `extensions.*` calls are not gated. An agent can read through any enabled app,
  read every app's settings through `extensions.list`, and make the host fetch the catalog
  and release manifests from GitHub (`extensions.catalog` with `refresh`,
  `extensions.catalogManifest`). The fetches are bounded as in NET-1, and the reads carry
  the RBAC of the host's own readers.
- A host action primitive called directly is not scoped by any app. It still needs consent and
  still names an allowlisted kind.
- **The audit log is a record, not evidence.** If the log path is unwritable or the disk
  is full, a call succeeds and leaves no record, so its caller can deny making it. A call
  that brings the process down before it returns is never recorded. The log is an
  ordinary file the user's account can edit or delete, and each rotation replaces the
  previous `audit.jsonl.1`. [#555] extended auditing to UI calls; it did not make
  recording fail closed or tamper-evident.
- **A plain read from the UI leaves no record.** Only mutating and sensitive
  capabilities are recorded on that path ([#555]), so the trail cannot answer which
  objects a person looked at in the app — only what they changed, and what secret
  material they revealed. MCP records reads as well, so the two sources are not
  comparable row for row.

### Web multi-user host

Another user of a shared `srelens-server`.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| WEB-1 | Read or change another user's apps, grants or settings | I, T | The inventory is one process-wide file, so the web host offers no apps. Web registries are built without a settings path, so `extensions.*` are never registered (`build_registry_with_paths` in `crates/registry/src/lib.rs`, used by `crates/server/src/bin/srelens-server.rs` and `--serve`), and all eight `extensions.*` IDs are refused before dispatch anyway (`WEB_DENIED_CAPABILITIES` in `crates/server/src/api.rs`). | Shipped. Per-user apps planned in [#515] (epic [#522]) |
| WEB-2 | Start a GitOps write with no consent prompt | E | The four action primitives are in `WEB_DENIED_CAPABILITIES`: the web host has no consent prompt, and no app scopes the call — what bounds one is an installed manifest fixing the kind and the template, which the web host has none of (`host_action_primitives_are_denied_on_web` in `crates/server/src/api.rs`). | Shipped |

### Tampered local state

Out of scope as an attacker (see [Scope](#scope)), but the host still checks what it reads.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| LOCAL-1 | Edit the inventory to add, enable or widen an app | T, E | The inventory is parsed strictly: unknown fields, a `schemaVersion` other than 1, a file over 1 MiB and duplicate IDs are all fatal. An oversized file is refused after reading one byte past the limit, never loaded whole (see VULN-2). Each app's manifest and signature proof are checked again, and a failing app is quarantined; the reason is recomputed on every load and never saved (`read`, `reverify`, `saved_form` in `crates/registry/src/extensions.rs`). An unsigned entry under a reserved ID is quarantined too (see APP-9, [#602]). A hand-added unsigned entry under any other ID is still confined to declarative readers, because every call runs `validate_app`. | Shipped |
| LOCAL-2 | Corrupt or lose the inventory through concurrent writes or a crash | T | Saves are serialized by a cross-process lock (`write_lock` in `crates/registry/src/settings.rs`). Each save goes through one helper (`replace` in `crates/registry/src/durable.rs`, called by `write` in `crates/registry/src/extensions.rs`): it writes a private temporary file beside the inventory, syncs it, and renames it over the inventory, so a concurrent writer or a crashed process leaves the old file or the new one, never a torn one. The rename is then made durable, so a power loss or kernel crash just after a save cannot bring back the previous inventory, and an app that was just disabled or removed cannot reappear enabled. On Unix the parent directory is opened before the rename and synced after it, and a directory created for the first save (`create_dir_all` in the same file) is synced into its parent. A save that fails returns an error with the old inventory intact; if only the sync after a completed rename fails, the save still succeeds and a warning says it may not survive a power loss, so the UI never reports a change as failed when it has been applied. On Windows the rename is `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`; Windows cannot flush a directory without administrator rights, so neither the rename's directory entry nor newly created parent directories from a first save are flushed here — both rest on the NTFS metadata journal. The catalog cache (`load_with` in `crates/registry/src/extensions/catalog.rs`) and settings (`write_document` in `crates/registry/src/settings.rs`) save through the same helper. | Shipped ([#611]) |

## Open work

| Issue | Addresses |
|---|---|
| [#543] | VULN-3: secret settings |
| [#551] | APP-5: the built-in Flux and Argo CD action list moved into manifests, with explicit write grants |
| [#554] | APP-8: permission diff on update |
| [#558] | APP-5, VULN-1: opt-in for unsigned apps that write or run code |
| [#559] | PUB-5, NET-3, NET-4: signed catalog and publisher delegation |
| [#560] | PUB-3: key rotation |
| [#561] | PUB-3, PUB-4, NET-4: revocation and a kill switch |
| [#563] | APP-8, PUB-4: update checks and downgrade protection |
| [#568] | APP-2: brokered network access |
| [#572] ([#521]) | VULN-1: sandboxed executable apps: the supervisor, its sandbox backends and their escape-hardening review, building on the [#571] findings |
| [#580] | VULN-2: parser fuzzing |
| [#581] | APP-12, VULN-4: performance budgets |
| [#607] | MCP-1: refuse non-loopback addresses for headless HTTP unless explicitly exposed |
| [#605] | VULN-3: redact extension settings in the MCP audit log |
| [#515] ([#522]) | WEB-1: per-user apps on the web host |
| [#39] | Scope: CSP, update chain and the rest of the host |
[#39]: https://github.com/srelens/srelens/issues/39
[#515]: https://github.com/srelens/srelens/issues/515
[#521]: https://github.com/srelens/srelens/issues/521
[#522]: https://github.com/srelens/srelens/issues/522
[#528]: https://github.com/srelens/srelens/issues/528
[#535]: https://github.com/srelens/srelens/issues/535
[#542]: https://github.com/srelens/srelens/issues/542
[#543]: https://github.com/srelens/srelens/issues/543
[#549]: https://github.com/srelens/srelens/issues/549
[#551]: https://github.com/srelens/srelens/issues/551
[#554]: https://github.com/srelens/srelens/issues/554
[#555]: https://github.com/srelens/srelens/issues/555
[#558]: https://github.com/srelens/srelens/issues/558
[#559]: https://github.com/srelens/srelens/issues/559
[#560]: https://github.com/srelens/srelens/issues/560
[#561]: https://github.com/srelens/srelens/issues/561
[#563]: https://github.com/srelens/srelens/issues/563
[#568]: https://github.com/srelens/srelens/issues/568
[#571]: https://github.com/srelens/srelens/issues/571
[#572]: https://github.com/srelens/srelens/issues/572
[#580]: https://github.com/srelens/srelens/issues/580
[#581]: https://github.com/srelens/srelens/issues/581
[#601]: https://github.com/srelens/srelens/issues/601
[#602]: https://github.com/srelens/srelens/issues/602
[#603]: https://github.com/srelens/srelens/issues/603
[#604]: https://github.com/srelens/srelens/issues/604
[#605]: https://github.com/srelens/srelens/issues/605
[#607]: https://github.com/srelens/srelens/issues/607
[#608]: https://github.com/srelens/srelens/issues/608
[#609]: https://github.com/srelens/srelens/issues/609
[#610]: https://github.com/srelens/srelens/issues/610
[#611]: https://github.com/srelens/srelens/issues/611
[#597]: https://github.com/srelens/srelens/pull/597
