# Threat model

What the extension platform defends against, how, and what is left. [security.md](security.md)
is the short version; this page is the reasoning behind it, with each mitigation tied to
the code that enforces it. It covers the platform as it is on `dev` (extension API 0.3
to 0.6, as `SUPPORTED_API_VERSIONS` in `crates/plugin-host/src/manifest.rs` lists them;
declarative apps, and executable apps whose sidecars run in the OS sandbox) and feeds
the pre-1.0 security review
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
- The signed catalog, the root and publisher keys it chains to, catalog downloads and
  signed releases ([trust.md](trust.md)).
- `.srelens-extension` packages: their reading, signature, and the directories they are
  unpacked into ([packages.md](packages.md)).
- The inventory and the catalog cache on disk.
- The multi-user web host, where each signed-in user has their own apps.

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
| App secrets | `extension_secrets` in srelens's encrypted secrets vault, `secrets.enc` under the MCP config dir, encrypted under the vault's one master key (held by the OS keychain, or derived from the master password) (`apps/desktop/src-tauri/src/vault.rs`) | Credentials an app's `secret-reference` settings hold for other services (Datadog, GitHub, Grafana Cloud). |
| Catalog cache | `*.extensions.catalog.json`, next to the inventory | What **Review installation** offers, and whether an install is recorded as `catalog`. |
| Unpacked packages | `*.extensions.packages/<app ID>/<SHA-256 of digests.json>/`, next to the inventory, owner-only | An installed package's files: its logo, and an executable app's binaries ([#574]), the only files unpacked runnable. |
| Sidecar data | `*.extensions.data/<directory named for the app ID>/` (`sidecar::data`, [#573]), next to the inventory, owner-only | The one directory an executable app's sidecar may write, and its working directory. Removed with the app. |
| Trust anchors | The pinned root and shipped publisher delegations, `trust/root.json` and `trust/publishers.json` in `crates/registry/src/extensions/` | Which catalog is trusted, which publisher may sign which app ID namespace, and so which apps are signed. The root keys are offline; the catalog key is a GitHub Actions secret in srelens/extensions; each publisher holds its own key ([trust.md](trust.md#key-ceremony)). |
| Signed proofs | `signatureProof` of each installed app: manifest bytes, signature, and the delegation that vouched for it | What lets a signed app be verified again on every load with no catalog. |
| Consent | Install and action reviews in the UI; the MCP consent gate | The user's decision is the only thing that grants an app access or starts a write. |
| Host UI integrity | The WebView | The user trusts that a srelens dialog means what it says. |

## Actors

| Actor | Trust | Supplies |
|---|---|---|
| User | Trusted | Decisions in reviews and dialogs; pasted manifests. |
| App author | Untrusted | A manifest, pasted locally or listed in the catalog. |
| Publisher (srelens or a third party) | Trusted only for the namespaces delegated to it; its key may be stolen | Releases signed with its key. |
| Catalog key holders | Trusted to list apps and delegate namespaces to publishers | `catalog.signed.json` on the `main` branch of [srelens/extensions](https://github.com/srelens/extensions), signed in its CI. |
| Root key holders | Trusted to choose the catalog key; keys kept offline | The root document the build pins. |
| Network attacker | Untrusted | Anything on the wire: responses, DNS answers, redirects. |
| MCP client or agent | Authenticated, but may act on injected instructions | Any tool call. |
| Other web user | Untrusted with respect to you | Requests to the same web host. |
| Web server operator | Trusted | The server's deployment config, including the extension policy every user's apps are held to (`SRELENS_EXTENSION_POLICY`). |

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
4. **Network to host.** Catalog, manifest, signature and package bytes arrive only through
   `download` in `crates/registry/src/extensions/catalog.rs`. A catalog and the publisher
   delegations in it become trusted only once their signatures verify under the pinned
   root (`TrustRoot` in `crates/registry/src/extensions/trust.rs`, `accept` and
   `verify_catalog` in `catalog.rs`).
5. **Package bytes to host.** An archive becomes a package only through `read` in
   `crates/registry/src/extensions/package.rs`, whole or not at all, with its signature
   checked under the same delegations as a manifest's, and reaches disk only through
   `unpack`, after it has verified.
6. **Disk to host.** The inventory and the catalog cache are parsed and checked again on
   every load (`read` in `crates/registry/src/extensions.rs`, `load_with` in
   `crates/registry/src/extensions/catalog.rs`). A package's logo is checked against the
   digest list its version was unpacked with each time it is read (`installed_icon` in
   `crates/registry/src/extensions/package.rs`).
7. **Caller to registry.** The WebView calls capabilities through `invoke_capability`
   (`apps/desktop/src-tauri/src/bridge.rs`), MCP clients through the consent-gated
   `handle_request` (`crates/mcp/src/stdio.rs`), and web users through
   `invoke_capability` (`crates/server/src/api.rs`).

## Entry points

| Entry point | Untrusted input | Mutating | Code |
|---|---|---|---|
| `extensions.validate` | Manifest, grants, signature, key ID, a package's digest list | No | `check_install` in `crates/registry/src/extensions.rs` |
| `extensions.configure` | Manifest, grants, signature, key ID; a package file as base64; a catalog release and package checksum; app ID; settings JSON; rollback revision | Yes | `configure`, `install_package` in `crates/registry/src/extensions.rs` |
| `extensions.packageManifest` | A package file as base64 | No | `read` in `crates/registry/src/extensions/package.rs` |
| `extension.secretStore` | App ID, setting, secret value | Yes | `change` in `crates/registry/src/extensions/secret_store.rs`, `VaultSecretStore` in `apps/desktop/src-tauri/src/extension_secrets.rs` |
| `extensions.list` | None | No | `read` in `crates/registry/src/extensions.rs` |
| `extensions.read` | App ID, revision, operation, context, namespace | No | `register` in `crates/registry/src/extensions.rs` |
| `extensions.resource` | App ID, revision, operation, context, namespace, name | No | `resolve` in `crates/registry/src/extensions/resource.rs` |
| `extensions.action` | As `extensions.resource`, plus action, UID and resourceVersion | Yes | `resolve` in `crates/registry/src/extensions/resource.rs`, `request` in `crates/kube/src/action_primitives.rs` |
| `extensions.catalog` | `catalog.signed.json` from the network | No | `load_with`, `accept`, `verify_catalog` and `parse_catalog` in `crates/registry/src/extensions/catalog.rs`; `TrustRoot` in `trust.rs` |
| `extensions.catalogManifest` | App ID and SHA-256; manifest and signature, or a package, from the network | No | `verify_release`, `verify_package` in `crates/registry/src/extensions/catalog.rs`; `parse_release_signature` and `verify_for` in `signing.rs` |
| Inventory load | `settings.extensions.json` | — | `read` and `reverify` in `crates/registry/src/extensions.rs` |
| Catalog cache load | `*.extensions.catalog.json` | — | `load_with` and `read_cache` in `crates/registry/src/extensions/catalog.rs`, which verify the cached signed catalog again |

`extensions.action` dispatches only the action declared for the selected reader,
through a granted host primitive. The generic primitives are also registry capabilities
and require consent; their direct invocation does not inherit an app's identity.

## Threats and mitigations

### Malicious app

An author who wants an app to do more than show custom resources.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| APP-1 | Read kubeconfig, tokens or local files | I | No app code is loaded into srelens. A declarative app is data; an executable app's code runs only as a sidecar in the OS sandbox, with no kubeconfig, no network and no path but its own data directory (VULN-1). The broker (`crates/plugin-host/src/lib.rs`) only forwards JSON arguments to host handlers. A manifest has no field that names a file, and unknown fields are refused (`deny_unknown_fields`); the one URL it may name is a `network.http` request's, which the host sends (APP-2), and the one command a `k8s.exec` binding's, which runs in a pod in the cluster, never on this computer (APP-14). The host resolves credentials from the context name; the app never sees them. | Shipped |
| APP-2 | Reach arbitrary network endpoints or run commands | I, E | As APP-1. Desktop bindings may target only `k8s.listCustomResource`, `k8s.listEvents`, the narrow summary readers, `network.http`, and the pod capabilities `k8s.streamLogs`, `k8s.exec` and `k8s.portForward` held to their own scope (APP-13, APP-14, APP-15) (`validate_app` in `crates/registry/src/extensions.rs`), and no binding may target another app's `plugin/` capability (`Manifest::validate`). `network.http` ([#568]) is broker-only: registered beside the CRD check and never in the registry the catalog and MCP are built from (`broker_only` in `crates/registry/src/lib.rs`), so nothing reaches it but an installed app's binding through `extensions.read`. A binding is one fixed GET with no inputs; a provider's query adds only the parameters the host sets for it (APP-16). Its URL and every redirect are held to the hosts the manifest lists, resolved with the app's saved `url` settings on every request (`Manifest::network_allowlist` in `crates/plugin-host/src/manifest/network.rs`): HTTPS only, plain HTTP only to loopback and only behind a per-app switch a person turns on, no credentials or fragment, at most four redirects, 10 s to connect, 20 s and 4 MiB in all (`Policy`, `send` in `crates/registry/src/extensions/network.rs`, over the rules shared with the catalog in `crates/registry/src/extensions/http_policy.rs`). A wildcard is one leftmost label with two labels after it. Hosts need API 0.4 (`API_FIELDS`), and the access review lists them, so an update that adds one is changed access. | Shipped. Brokered network access with host allowlists shipped ([#568]) |
| APP-3 | Read Secrets or other core resources through the reader | I | `validate_app` (`crates/registry/src/extensions.rs`) requires a fixed, non-empty `group`, `version`, `plural` and `kind` of letters, digits, `.` and `-`, so the core group (Secrets, ConfigMaps, Pods) cannot be bound. The group must also be shaped like a CustomResourceDefinition group, with a dot and no empty label, so built-in groups such as `apps` and `batch` are refused (`group_problems` in `crates/registry/src/extensions/crd.rs`). The same rule runs when the inventory loads, so a stored app that breaks it is quarantined. Before dispatching, `extensions.read`, `extensions.resource` and `extensions.action` confirm that a CustomResourceDefinition named `{plural}.{group}` declares that group and plural and serves the bound version on the cluster (`require` in the same file, `custom_resource_serves` in `crates/kube/src/crds.rs`). That refuses dotted built-in groups such as `networking.k8s.io`, aggregated APIs, and a version the CRD does not serve; a failed lookup refuses the call and says so. `k8s.listCustomResource` returns names, namespaces, ages and the declared printer columns (`list_custom_resource_capability` in `crates/kube/src/crds.rs`). | Shipped. Built-in groups refused in [#601]; bindings shown in the install review by [#608]. See residual risk |
| APP-4 | Widen a read by overriding bound arguments | T, E | The broker refuses any input not listed in the binding's `inputs` and any missing required one, and the schema it exposes sets `additionalProperties: false` (`PluginHost::register`). Fixed `arguments` are merged into every call, and inputs may not overlap them, so a caller cannot override one. `extensions.read` forwards only `context` and `namespace` and checks the namespace's syntax; `validate_app` refuses a binding that fixes either. | Shipped |
| APP-5 | Write to the cluster, or dispatch an operation that needs consent | E | `validate_app` refuses a target that is not read-only or carries `requires_confirm`, `sensitive` or `destructive`. A manifest cannot supply annotations: the broker copies the host's and forces `requires_confirm` on anything not read-only, sensitive or destructive. A manifest declares `actions` ([#549]), which bind one of six host action primitives (`PRIMITIVES` in `crates/kube/src/action_primitives.rs`) and nothing else (`validate_app`). The host copies the kind from a reader binding in the same manifest and fixes the inputs to the reviewed object (`Manifest::action_binding`, `ACTION_INPUTS`), so an action reaches no kind the app lacks a granted reader for; what it writes is fixed at install and checked by the primitive itself, at install, at reverification and again on the way to the cluster (`check_bound_arguments`, run from `PluginHost::problems_at` and from each handler). `k8s.mergePatch` refuses `metadata.finalizers`, `ownerReferences`, `managedFields`, the pin fields, `status`, a Secret's values and RBAC kinds. Every primitive re-reads the object, refuses a moved UID or resourceVersion and one being deleted (`guard_reviewed`), and pins its PATCH to both. The Flux and Argo CD actions are declared in those apps' manifests like any other app's: the host registers no GitOps action of its own, and neither manifest validates with only its reader grant (`gitops_examples_declare_writes_and_core_has_no_implicit_action` in `crates/registry/src/extensions/resource.rs`). An app that declares any action needs a verified publisher signature, or **Allow unsigned apps to modify clusters and run code** turned on in Settings → Apps, which is off by default (`allowUnsignedApps`, set by the `unsignedApps` action of `extensions.configure`). Validation reports it, and install, update, enable and rollback refuse without it (`check_unsigned_policy` in `crates/registry/src/extensions.rs`). Every inventory load applies it again, so turning it off disables the affected apps, and they stay disabled when it is turned back on (`apply_unsigned_policy`); readers and actions refuse a blocked app on every call (`policy_blocked`). | Shipped ([#549], [#551], [#558]) |
| APP-6 | Keep acting after being disabled, removed, updated or quarantined | E | `extensions.read`, `extensions.resource` and `extensions.action` read the inventory on every call, and require the app to be enabled, at the caller's revision, and to pass `validate_app` with its stored grants. `Registration::unregister` revokes the handlers older registry snapshots still hold. An app's MCP tools ([#574]) route through those same paths, pinned to the revision they were listed for, and are withdrawn from every snapshot when the app changes (MCP-4). An executable app's sidecar is stopped then too. Calls already admitted may finish. | Shipped |
| APP-7 | Use a permission it was not granted | E | `permissions` must name exactly the bound targets (`EXTENSION_PERMISSION_MISMATCH` in `Manifest::validate`), and each one must be among the grants the caller supplied (`validate_app`, `PluginHost::register`). | Shipped |
| APP-8 | Escalate through an update or rollback | E | An update is a new `extensions.configure` install, which is mutating and carries its own grants; nothing updates automatically. Before an update, `extensions.validate` compares the incoming manifest's access with the installed revision's: the grants, what each reader binds, which settings it keeps secrets for, and each action's primitive, resource, arguments and preconditions. It returns what is added, removed and unchanged (`permission_diff` and `access_items` in `crates/registry/src/extensions.rs`). The review lists the additions and removals first and collapses the unchanged access (`ExtensionManager` in `packages/ui-next/src/extensions/Extensions.tsx`), and an MCP install's consent prompt names the added and removed access (`handle_request` in `crates/mcp/src/stdio.rs`). An update must name the revision it was reviewed against (`reviewedRevision`), so an app that changed after the review is refused and has to be reviewed again. The diff is a review aid and refuses nothing: an update that widens access installs once it is approved. A rollback verifies the kept version's signature again and runs `validate_app` with the grants given now (`Configure::Rollback` in `crates/registry/src/extensions.rs`). Its review compares capability IDs: when they differ from the grants held now, it lists those the kept version requests and those it no longer uses. For `network.http` ([#568]) it also compares the hosts, with the declaration of each url setting a host is read from, and each request's arguments with the settings they interpolate (`networkReach`, `networkRequests` in `packages/ui-next/src/extensions/networkText.tsx`): when either differs it says so and lists the kept version's hosts, with a setting host's default, and its requests (`ExtensionDetails` in `packages/ui-next/src/extensions/ExtensionDetails.tsx`). Reader and action bindings are still compared by capability ID only. | Shipped. The access diff and reviewed-revision check on update in [#554]. Detecting newer versions (an Updates view) and downgrade protection planned in [#563] |
| APP-9 | Pose as an official app, or as another publisher's | S | The catalog delegates app ID namespaces to publishers, `org.srelens` to srelens, and an ID in a delegated namespace installs only with its publisher's signature (`check_install` and `unsigned_reserved` in `crates/registry/src/extensions.rs`, `Delegations::owner` in `crates/registry/src/extensions/trust.rs`), so an unsigned install cannot take such an ID or replace a signed app. An ID is matched without regard to case, so `org.Srelens.` is reserved too. A build with no usable root cannot tell which IDs are reserved, and installs and restores nothing. An unsigned entry already stored under one of the namespaces this build ships a delegation for, such as an `org.srelens.` app installed before [#528], is quarantined when the inventory loads, so it cannot be enabled, and no unsigned kept version under a delegated namespace is restored (`reverify` and the `rollback` action; shipped by [#602]). No logo is chosen by ID, and none is bundled: an app's logo comes only from its installed package, and never implies trust (PKG-5, [#562]). Settings → Apps labels each app **Unsigned local**, **Signed by** the publisher the host verified on that read (`signedBy`), or **Signature not verified** (`packages/ui-next/src/extensions/Extensions.tsx`). | Shipped. See residual risk |
| APP-10 | Spoof host UI or dialogs | S | Apps contribute data, never markup: pages, detail tabs and row actions render with host components, and the frontend renders no text as raw HTML. Names, titles and groups are 1–120 characters with no control characters and no format characters (category Cf), such as right-to-left overrides and zero-width spaces, so none displays differently from what it holds (`label` and `is_format_character` in `crates/plugin-host/src/manifest.rs`, [#603]). Catalog names and descriptions refuse the same control and format characters (`parse_catalog` in `crates/registry/src/extensions/catalog.rs`). The install review renders a manifest's own name only once the host has accepted it, and says "This manifest" for one still being checked or refused (`ExtensionManager` in `packages/ui-next/src/extensions/Extensions.tsx`). Values the host does not restrict, such as printer column names and JSON paths, are drawn in the review's binding summary with format, control and line-separator characters written as escapes, and the review's manifest view escapes format characters as Details does (`plainText` and `escapeFormatCharacters` in `packages/ui-next/src/extensions/displayText.ts`, [#608]). Install and action reviews are host-owned (`packages/ui-next/src/extensions/Extensions.tsx`, `packages/ui-next/src/extensions/ExtensionResourceDetails.tsx`). | Shipped. See residual risk |
| APP-11 | Read clusters the user did not intend the app for | I | Every read names an explicit context and runs under that context's RBAC. Installation is app-wide, so by default an enabled app can read any cluster the user opens it on. An optional per-app cluster allow-list narrows that: the `clusters` action of `extensions.configure` limits an app to 1–256 named contexts, or with an explicit `null` allows every cluster — leaving the list out is refused rather than read as "every cluster" (`Configure::Clusters` in `crates/registry/src/extensions.rs`). It is enforced on `extensions.read`, `extensions.resource` and `extensions.action` by one check (`Installed::check_scope`, called from `extensions.rs` and `crates/registry/src/extensions/resource.rs`), which refuses a limited app on any other context and refuses it equally when the host could not resolve the context at all, saying so rather than guessing. The list is held by each context's `key` (`ResolvedContext::key`), so a same-named context in another kubeconfig cannot inherit the access and no two contexts share an entry, and an update keeps the list as it keeps settings. | Shipped ([#597], [#535]) |
| APP-13 | Reach pods it is not about: stream their logs, run in them, or forward to them ([#567]) | I, E | A pod binding never names a pod. Its scope is a reader binding in the same manifest whose objects select pods — a Deployment, StatefulSet or DaemonSet reader, or a namespaced custom-resource reader with the path its objects keep their selector at — or the 1–16 namespaces its permission grants (`pod_problems` in `crates/plugin-host/src/manifest/pods.rs`). On every open, and on `extensions.pods`, the host reads the object the view names and its own label selector with the user's credentials, refuses one with no selector or one that selects every pod, reads the pod the view names, and matches its namespace and labels itself rather than trusting a list the cluster returned (`scope`, `admitted_pod` and `Selector` in `crates/registry/src/extensions/pods.rs`). A Service is only a way to name a pod: a forward through one reaches a running pod it selects that the scope admits, or nothing (`service_target`). A log stream reads the scope again on every reconnect and ends when its pod leaves it; a forward checks its pod every 2 s. A read that gets no answer (a timeout, or a 408, 429 or 5xx status) is asked again, and nothing is followed or retargeted on the old scope until one comes (`ScopeError`). The app's authority is rechecked on every inventory write. Tests: `a_pod_outside_the_selector_or_the_grant_is_refused`, `the_pods_a_view_is_offered_are_the_ones_the_scope_admits` and `a_forward_through_a_service_reaches_only_a_pod_in_scope` in `crates/registry/src/extensions/pods_tests.rs`. Residual: a scope is as wide as the selectors of the objects of the kind the app reads, in any namespace the view opens, and a namespace grant reaches every pod in it; the review says so before install. | Shipped |
| APP-14 | Run a command the person did not approve, or a shell ([#567]) | E | `k8s.exec` binds one `command` fixed in the manifest: 1–32 literal arguments, no settings, run as arguments without a shell, and never a shell, nor a wrapper (`env`, `busybox`, `toybox`, `nice`, `nohup`, `timeout`, `setsid`, `stdbuf`, `xargs`) with a shell's name anywhere after it or `env -S` (`command_problems`, `runs_a_shell`). The stream request has no field for a command. `k8s.exec` is sensitive and `high` impact (`EXEC_ANNOTATIONS`), an unsigned app that binds it needs the unsigned-app setting (`needs_unsigned_policy`), and every session is refused unless the view sends back, field for field, the pod, container and command the host confirmation named (`open_exec` in `crates/registry/src/extensions/streams/pods.rs`); the confirmation draws every argument, escaped and never cut (`HostConfirmation`'s `command`). Stdin and a terminal are never attached; a session is bounded to 300 s and 1 MiB of output. Every session, run or refused, is audited. Tests: `exec_is_refused_without_the_host_confirmation`, `exec_is_refused_for_anything_but_the_confirmed_manifest_command`, `an_exec_command_is_one_fixed_program_and_never_a_shell`. Residual: the shell refusal is a guard against the obvious, not a sandbox — a fixed `python -c` is still a script — so the review, which shows the exact command, and the confirmation before each run are the control. Ending a session closes its connection; a command that ignores that may run on in the container until it exits. | Shipped |
| APP-15 | Open a lasting local listener, or one that outlives its view ([#567]) | I, E | The host binds a forward on `127.0.0.1` only, at a port it picks; the request has no field for one. The source owns the listener and every connection through it in a `JoinSet`, so when the stream ends — its view closed, its window closed or reloaded, the app disabled, updated or removed — the port stops listening and each connection closes (`Forward` in `crates/registry/src/extensions/streams/pods.rs`); a connection's forwarder task is aborted when it drops (`connect_port` in `crates/kube/src/app_pods.rs`). Every forward is audited with its local port. Test: `closing_the_view_releases_every_stream_and_forward_it_opened`. Residual: while a forward is open, any program on this computer may connect to its port. | Shipped |
| APP-16 | Turn a name the view binds into query syntax, or supply markup ([#569]) | T, I, S | A provider declares a PromQL, LogQL or TraceQL template, never a request: the host binds each variable. `QueryTemplate::parse` (`crates/plugin-host/src/manifest/providers.rs`) reads the template as each language reads its strings and admits `${cluster}`, `${namespace}`, `${workload}` and `${pod}` only inside a double-quoted string, where `render` escapes `\` and `"` — the two escapes the three languages document — and, in a regex matcher's string (after `=~`, `!~`, `\|~`, or a LogQL line filter's `or`), only as `${name:regex}`, which quotes RE2 metacharacters first. It refuses a name anywhere else (bare, in a raw or single-quoted string), comments (`#`, and the `/* */` and `//` LogQL's and TraceQL's lexers skip, quotes and all), control characters and line separators, unclosed strings, unknown variables and settings. Every value is held to what it can be: the names the view sends to Kubernetes name syntax, and `${cluster}`, the host's resolved context name, to letters, digits and `._:/@+-`, so no value can end a string, open a comment or start a LogQL `line_format` template. The host sets the query and time range as parameters the binding may not set, through `network::request`, with the policy the app was read under, so the allowlist, loopback switch, redirects, size limit, secrets by reference and, on the web, the operator's network ceiling (#578) hold as for any request (test: `a_provider_query_under_a_policy_reaches_only_hosts_its_ceiling_allows`). What comes back is read by the host (`read_matrix`, `read_streams`, `read_traces` in `crates/registry/src/extensions/providers.rs`) and drawn with host components as text; nothing a provider answers is rendered as HTML. A backend's own words the host repeats, from a refusal or a 2xx `status: "error"`, are scrubbed of the full URL and its host, its path segments and non-host query values (each as sent and decoded) of at least eight characters, and every secret header value the request carried (`Scrub` in `crates/registry/src/extensions/network.rs`); they are left out when they hold a secret header value too short to replace. Residual: a shorter URL component echoed by itself, and the query parameters the host adds (the provider's query and time range), are repeated as the server wrote them. The query's own input is bounded before anything is looked up, and no refusal repeats it. Tests: `a_cluster_name_cannot_close_its_string_or_add_to_the_query` and a property test over arbitrary names in `crates/plugin-host/tests/providers.rs`, `a_cluster_name_reaches_the_query_as_one_escaped_string` against a socket in `crates/registry/src/extensions/providers_tests.rs`. Residual: a query is as wide as the backend's own access for the credential the app is given; the template's labels decide what it matches, and the review shows the whole template. | Shipped |
| APP-17 | Call another system on a timer ([#569]) | D, I | Metric and trace panels ask when they open, when their range changes and on Refresh. A log provider the log view follows is the one timer: the `logProvider` source asks every 5 seconds — the host's interval, which the manifest cannot set — and a second later only after a full page of 1,000 lines, only while its view is open, and it stops when the view, its window or the app goes (`Follow` in `crates/registry/src/extensions/streams/providers.rs`). Each query is authorized again, counts against the app's 8 streams, and its frames against 50 a second. The review says a log provider polls while a log view is open. | Shipped |
| APP-12 | Exhaust the host | D | What an app declares is bounded. A manifest is at most 256 KiB (`MAX_MANIFEST_BYTES` in `crates/plugin-host/src/manifest.rs`), with 1–32 capabilities, at most 32 printer columns per binding (`MAX_PRINTER_COLUMNS`), at most 64 contributions, 1–32 kinds per detail tab or row action, and 1–12 pages per dashboard. The inventory is at most 1 MiB and keeps at most three replaced versions per app. Single-resource inspection reads at most 10 pages of 500 events (`list_events` in `crates/kube/src/gitops.rs`). App pages and dashboards list through `list_capped` (`crates/kube/src/list_cap.rs`): at most 2,000 rows from `k8s.listCustomResource` and `k8s.listEvents`, with a `truncated` marker when more remain. What the host does with what an app declares is measured: resolving a joined column over 1,000 rows lists each joined reader once, closing a view cancels every watch and stream it opened, and loading 50 apps, a call's own overhead and the navigation they contribute are timed against the roadmap's targets (`crates/registry/src/extensions/budget_tests.rs`, `packages/ui-next/src/extensions/extensionBudgets.test.tsx`). | Manifest, inventory and app-read limits shipped ([#609]). Performance budgets measured in CI ([#581]); a call's overhead grows with the number of installed apps (see below) |

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
  user read it. For an update, the review puts the access added and removed since the
  installed revision first ([#554]; see APP-8).
- **App reads are capped.** An app page lists with `k8s.listCustomResource`
  (`list_custom_resource_capability` in `crates/kube/src/crds.rs`), and a dashboard reads
  events with `k8s.listEvents` (`list_events_capability` in `crates/kube/src/events.rs`).
  Both page with `limit`/`continue` and stop at 2,000 rows (`list_capped` in
  `crates/kube/src/list_cap.rs`), returning `truncated` when more remain. A binding may
  declare at most 32 printer columns (`MAX_PRINTER_COLUMNS` in
  `crates/plugin-host/src/manifest.rs`). The UI pages the returned rows and says how many
  are not shown (`ExtensionResults.tsx`, `ExtensionWorkspace.tsx`). `request_timeout` still
  bounds only wait time, not the cost of a full capped page that arrives in time.
- **Every installed app slows every app's calls a little.** Each broker call reads and
  re-validates the whole inventory to authorize one app, so its own overhead grows with
  the number installed: well under a millisecond with one, around the 5 ms target with 50
  on a fast machine ([#581], [PERFORMANCE.md](../PERFORMANCE.md#extension-platform-budgets)).
  Nothing an app declares changes that; the inventory's 1 MiB limit bounds it.
- **An allowed host is trusted with what the app sends it and what it answers.** The
  allowlist says where a request may go, not what the server does with it: a
  `network.http` app can send the query it declares, and a secret header, to any host
  the person approved. A `${settings.<id>}` host goes wherever the person points the
  setting, and a wildcard covers every one-label subdomain, including ones created after
  install. HTTPS certificate verification is what ties a name to its server; a name that
  resolves to a private address still needs a certificate valid for that name. Plain
  HTTP to loopback, once allowed for an app, has no TLS at all, which is why it is off
  by default and per app.
- **Unsigned apps choose their own names.** A local app outside the reserved namespace can
  call itself "Argo CD". It gets an initials mark and the **Unsigned local** label, not the
  bundled logo.
- **Labels can still look alike.** Format characters are refused, but letters are not
  compared across scripts. A name that spells "Argo CD" with a Cyrillic "А", or holds an
  invisible letter such as the Hangul filler (U+3164), still validates.

### Compromised publisher or key

An attacker who holds a publisher's signing key, the catalog key or root keys, or controls
a release repository.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| PUB-1 | Forge a publisher's release without its key | S, T | A release in a delegated namespace carries a detached Ed25519 signature over the exact manifest bytes, or, for a package, over the exact bytes of its digest list, which names the manifest and every other file by SHA-256 (`verify_signed` in `crates/registry/src/extensions/package.rs`, [#562]), checked against the keys of the publisher delegated that namespace (`verify_for` in `crates/registry/src/extensions/signing.rs`). A delegation counts only when the catalog role of the pinned root signed it (`TrustRoot::publisher` in `trust.rs`), so neither a catalog entry nor a repository URL can supply a key or make a release signed: [#559] removed the pinned repository table and the repository-prefix rule. A signature that names a key is refused unless the key is that publisher's, before anything is verified with it. | Shipped |
| PUB-2 | Alter a signed installation after install | T | The verified bytes and signature are stored as `signatureProof`, with a package's digest list, which must be the one the installed version's directory was unpacked with, and with the signed delegation that vouched for them unless this build's shipped delegations already do. Every inventory load verifies the delegation under the pinned root, the signature under the delegation, and that the bytes parse to the installed manifest (`reverify`, `verify_proof` in `crates/registry/src/extensions.rs`); an app that fails is quarantined on its own. A delegation moved to an app outside its namespaces does not cover it, and the app is quarantined. A delegation the build ships for the same publisher at the same or a later version replaces the kept one, so a hand-edited proof cannot bring back a key a later delegation withdrew. Install applies the same rule to the catalog's delegations, so what installs as signed verifies on the next load (`a_catalog_rotates_a_shipped_publisher_only_with_a_later_delegation`). | Shipped |
| PUB-3 | Use a stolen publisher key | S, E | A manifest signed with a stolen key installs as that publisher's, but only under the namespaces delegated to it: a third party's key cannot sign an `org.srelens.` app, nor another publisher's. It is still a declarative app, held to the rules above and to the user's permission review. The catalog can replace the delegation's keys for new installs; installed apps keep the delegation that vouched for them until a build ships a later one for that publisher, which quarantines what the withdrawn key signed. | Namespace containment shipped ([#559]). Rotation planned in [#560]; revocation and a kill switch in [#561] |
| PUB-4 | Reinstall an older, vulnerable signed release | T | A validly signed older manifest still installs through `extensions.configure`, and a rollback restores a kept one. Both need the user's review, and nothing warns about the older version. The catalog itself cannot be rolled back to re-offer old releases as current (NET-4). | Planned: revocation in [#561], downgrade protection in [#563] |
| PUB-5 | Sign apps as an unknown publisher | S | A signature on an app ID in no delegated namespace is refused (`verify_for`). A third party signs only once the catalog delegates it a namespace; until then its apps install unsigned and are labelled so. | Shipped ([#559]) |
| PUB-6 | Use a stolen catalog key | S, T | The catalog key can sign a catalog that delegates any namespace, `org.srelens` included, to a key of the attacker's, and so publish apps as any publisher on every host that fetches it. The catalog role's threshold can require more than one key. A host refuses a catalog older than the one it holds, so the attacker must also out-version the real catalog, and its catalogs expire. Apps installed before keep the delegations that vouched for them. | Replacing the catalog key through root rotation is [#560]; revoking what it signed is [#561] |
| PUB-7 | Use stolen root keys | S, T | Whoever holds a threshold of root keys chooses the catalog key. The root keys are kept offline, at least two are required ([trust.md](trust.md#key-ceremony)), and the root is replaced only by a host release until [#560] adds rotation. | Offline custody is the key holders' ([trust.md](trust.md#key-ceremony)) |

Residual risk:

- Replacing the root, or retiring a publisher key from installed apps, requires a host
  release until [#560].
- The catalog key and the srelens release key are protected by GitHub Actions secrets in
  srelens/extensions and the app repositories, and the root keys by their offline holders.
  Their handling is outside this repository.
- A namespace delegated in the catalog, but not shipped with the build, is not checked
  when the inventory loads, which has no catalog at hand. An unsigned app installed under
  it before the delegation stays usable until it is replaced; installing one after is
  refused. Quarantining those on a catalog refresh belongs with [#561].

### Vulnerable app

An honest app with a flaw, or a host bug that an app's input can reach.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| VULN-1 | A flawed app is hijacked to run code, open sockets or read files | E | A declarative app has no code to hijack. An executable app (API 0.6, [#574]) does, and it runs only as a sidecar in an OS sandbox; an unsigned one only behind the setting unsigned apps that write already need (APP-5), whether it writes or not. The host decides which apps need it by an exhaustive match on the manifest kind, so a new kind cannot be added without deciding (`needs_unsigned_policy` in `crates/registry/src/extensions.rs`). The sandbox spike ([#571]) found that AppContainer plus a Job Object on Windows, and Landlock plus seccomp plus cgroup v2 on Linux, each enforced all seven of its checks on ordinary operations. On macOS 27.0 arm64, a Seatbelt profile under the deprecated `sandbox-exec` enforced the filesystem, network, DNS, process and stdio checks. The kernel provided no memory or CPU limit (observed: `setrlimit` refuses the memory limits, and CPU use went unthrottled). Intel Macs and older macOS versions are unverified. The maintainer decided in [#521] that macOS is included, with memory and CPU **host-enforced** by a supervisor watchdog ([#713]: built, not yet checked with Seatbelt on a macOS 27 Mac, so macOS still refuses sidecars). That guarantee is **weaker than kernel enforcement**: the watchdog bounds sustained use, but a burst between samples can exceed the limit. The findings, the support matrix and the proposed backends are in [Sandbox backends for executable extensions](../design/plugin-architecture.md#sandbox-backends-for-executable-extensions). The backends there are Proposed, and the macOS decision is Accepted. The supervisor ([#572]; `crates/plugin-host/src/sidecar/`, [sidecar-protocol.md](sidecar-protocol.md)) implements them. It refuses a sidecar on an OS with no backend, on macOS until its watchdog is checked with Seatbelt on a macOS 27 Mac ([#713]), and on Linux without Landlock or a delegated cgroup; it never starts one unconfined. A sidecar gets only the environment srelens names (never the host's, which may hold `KUBECONFIG` or tokens), no descriptor above stderr on Linux and macOS, and no socket at all under the Linux filter. A sidecar starts on its app's first operation call, from a binary checked against its package's digest list first, so one changed on disk since the install is refused (`installed_binary` in `crates/registry/src/extensions/package.rs`, `AppSidecars` in `crates/registry/src/extensions/sidecars.rs`). It gets one owner-only data directory and none of the host's environment, every call's input is held to the operation's declared inputs first (`Operation::check_input` in `crates/plugin-host/src/manifest/sidecar.rs`), and it stops when its app is disabled, updated or removed. It reaches the host only through the broker ([#573]): the facade its app's own screens use, with the app's identity from the supervisor, and a person's confirmation before any write (the host confirmation in the desktop app, which names the app; none headless, where every write is refused). None of this has been reviewed against deliberate escapes. | Supervisor and per-OS backends shipped in [#572]; the executable kind that starts them in [#574]. Planned: the escape-hardening review, which the ADR assigned to #572 and #572 did not do, in [#744]. Not done: checking macOS's host-enforced limits with Seatbelt on a macOS 27 Mac. [#713], which built the watchdog, is closed. The unsigned-app setting shipped in [#558] |
| VULN-2 | A malformed or oversized manifest, catalog, signature or inventory crashes or exhausts the host | T, D | Parsers are Rust and `serde`. Downloads and the catalog cache are read only up to their limits, so an oversized one is refused before it fills memory: signed catalogs 2 MiB, the catalog document they carry 1 MiB, and downloaded release signatures 512 bytes (`download` and `load_with` in `crates/registry/src/extensions/catalog.rs`, then `accept` and `parse_catalog`); root and publisher documents are bounded as their envelopes decode (`trust.rs`). The inventory is read only to one byte past 1 MiB, so an oversized, corrupt or tampered file is refused before it is loaded whole or parsed (`read` in `crates/registry/src/extensions.rs`); the same limit bounds every inventory the host writes. A manifest string is checked against 256 KiB before it is decoded (`Manifest::decode` in `crates/plugin-host/src/manifest.rs`). Caller-supplied capability inputs are bounded twice. First the transport: an MCP request is at most 4 MiB (`MAX_REQUEST_BYTES` in `crates/mcp/src/lib.rs`). The HTTP router sets that as its body limit explicitly, refusing a larger body with 413 (`router_inner_with_push` in `crates/mcp/src/http.rs`), and stdio reads each request line through a bounded reader that drops a longer line as it arrives, never holding it, and answers with a JSON-RPC error naming the limit (`BoundedLines` in `crates/mcp/src/stdio.rs`, fed from stdin by `run_mcp_stdio` in `apps/desktop/src-tauri/src/main.rs`). Then the fields, while the arguments are decoded (`crates/registry/src/extensions/limits.rs`): on `extensions.validate` and `extensions.configure`, a `signature` is refused at its 65th byte or when shorter than 64, a `keyId` that is not 64 lowercase hexadecimal characters is refused, a `manifest` over 256 KiB is refused before it is decoded, and a `settings` object over 64 KiB as compact JSON is refused before it is saved. Each refusal is an invalid-input error naming the field and its limit. The desktop bridge sets no transport limit (`invoke_capability` in `apps/desktop/src-tauri/src/bridge.rs`): its only caller is the app's own WebView, whose request is already in the process's memory when the command runs, and the field limits apply to it as to MCP. Every problem found in a manifest is reported with a stable code and path (`crates/plugin-host/src/validation.rs`). | Limits on downloads, the catalog cache, the inventory read and caller-supplied inputs shipped ([#610]); a package's limits are PKG-3. Parser fuzzing shipped ([#580]): cargo-fuzz targets for the manifest, catalog, signed-manifest, package and inventory readers (`fuzz/`), and since [#559] for the signed catalog and the trust metadata (root, publisher delegation and release signature), run by the Fuzz workflow (`.github/workflows/fuzz.yml`) for a minute on a pull request that touches the parsers and for fifteen minutes nightly. It is not a required check. `cargo test` holds the same properties over a fixed set of cases ([testing.md](testing.md#fuzzing)) |
| VULN-3 | An app's settings leak a credential (EXT-083) | I | Settings are typed by the manifest (#542), and a credential belongs in a `secret-reference` setting, whose value never enters the inventory. Every save is held to the declarations (`Manifest::check_setting_values` in `crates/plugin-host/src/manifest/settings.rs`, called from `checked_settings` in `crates/registry/src/extensions/app_settings.rs`), and any value for a `secret-reference`, even its reference, is refused. A `url` setting refuses a user name or password. The inventory writer refuses to save a secret setting that holds anything but its host-minted reference (`saved_form` in `crates/registry/src/extensions.rs`), whichever path put it there, and loading drops one that does. **Secret values ([#543]):** `extension.secretStore` keeps them in srelens's encrypted secrets vault, one more entry in `secrets.enc` under the vault's one master key (held by the OS keychain, or derived from the master password), beside the MCP token (`VaultSecretStore` in `apps/desktop/src-tauri/src/extension_secrets.rs`); no per-secret keychain item and no second store. The value goes to the vault first and the reference is written after it (`change` in `crates/registry/src/extensions/secret_store.rs`). It needs the app's `extension.secretStore` grant, which a manifest installed now must request exactly when it declares a secret setting (`Manifest::install_problems`, from `check_install`; `Manifest::validate` refuses it without a secret setting). An app installed before the permission existed (#542 shipped in the pre-release `srelens-v0.15.1-185`) declared its settings under `^0.3`, where `settings` is not an API field ([#709]), so it is quarantined until updated; a `^0.4` app without the permission keeps loading and cannot keep a secret until it is reinstalled with it. The install and update review shows the permission with the settings it covers and the host's metadata. It fails closed: a set is refused unless the vault's key is held by the OS keychain (or its biometric gate) or derived from the master password and the vault is unlocked, so a key only in a plain file beside the vault (no keychain and no master password) and a locked vault are refused with the reason, never written elsewhere. The value is write-only: the capability answers `{set}`; `extensions.list` returns the reference and reports a reference whose value is gone as not set; refusals never repeat the value (`secret` is refused by a deserializer that does not echo it); `SecretValue` and the vault's `Secrets` print no value in `Debug`; the MCP consent prompt shows the window only the action and, when they name an installed app's declared secret, the app and the setting, so a value an agent put anywhere else in the call never reaches the renderer (`shown_args` in `apps/desktop/src-tauri/src/mcp_confirm.rs`); every refusal is scrubbed of every value the call carried (`register` in `crates/registry/src/extensions/secret_store.rs`); the capability is `sensitive`, so its audit record blanks every argument value; and neither the settings export nor the settings bundle carries it (`secrets_for_export` in `apps/desktop/src-tauri/src/bundle_cmd.rs`). The store follows the inventory: after every inventory change the host deletes each stored secret no app references (`sweep`), so removing an app, and an update or rollback that drops the setting or changes its type, delete the secret; Reset in Settings → Apps clears the app's secrets explicitly before it resets the other settings. A setting reaches a capability only through a binding argument the capability marks settable, and never a secret (`PluginHost::interpolate`, `Capability::with_settable`); a secret reaches one only through `PluginHost::inject_secret`, into an argument the capability declares a secret slot. The one slot is `network.http`'s `secretHeaders` ([#568]): the host reads the secret only after the request has passed every allowlist and scheme rule, puts it into a sensitive header value rather than any JSON, refuses a redirect to another origin for a request that carries one (reqwest drops the standard credential headers such as `Authorization` and `Cookie` on its own, but not a custom one such as `DD-API-KEY`), and scrubs the URL and host from every error (`read_with`, `describe` in `crates/registry/src/extensions/network.rs`). A literal `Authorization` or `Cookie` header in a manifest is refused. Other settings are still plain text in two places. The inventory stores them, and `extensions.list` returns them. An `extensions.configure` call is also recorded in the local audit log, from MCP or from Settings → Apps (#555) (`audit.jsonl`, created with mode 0600 on Unix), whether consent is granted or denied. The capability is not sensitive-annotated, so `redact` (`crates/capability/src/audit.rs`) removes values by key name, which does not catch a setting named `credential` or `certificate`; the `settings` map is therefore redacted as a whole, keeping the action, the app ID and the setting names and blanking every value at any depth, and the recorded error is scrubbed of the same values, since a refused argument tends to be echoed by the refusal ([#605]). Other `extensions.configure` actions are recorded as before. Settings saved from Settings → Apps go through the same capability and so through the same redaction: the app ID and the setting names are recorded, never a value. | Audit-log redaction of `settings` shipped ([#605]). Typed settings, with secret values refused from the inventory, shipped ([#542]). Secret storage in the encrypted secrets vault, write-only, deleted with the app, shipped ([#543]). Injection into brokered HTTP headers shipped ([#568]); web storage planned in [#522]. See residual risk |
| VULN-4 | An app is slow on a large cluster | D | Each cluster request is bounded in time by `request_timeout` (`crates/kube/src/connect.rs`), 8 seconds by default and configurable from 1 to 120. App-reader lists are also bounded in size by `APP_LIST_CAP` (`crates/kube/src/list_cap.rs`). A table's joined columns list each joined reader once per five-second snapshot, however many rows, columns or badges read it, and a view that closes cancels its watches; both are held by the budget tests (`crates/registry/src/extensions/budget_tests.rs`). | App-read row caps shipped ([#609]). Performance budgets measured in CI ([#581]) |

Residual risk:

- **An app secret is as safe as the vault, and no safer.** Anything running as the user
  while the vault is unlocked can read `secrets.enc` with the vault's key, as it can
  read the MCP token and the provider API keys; the vault defends against another user
  and against a copied disk, not against local malware (see [Scope](#scope)).
- **A secret can outlive its app until the next change.** If the vault is locked when an
  app is removed, reset or updated, the inventory stops referencing the secret at once and
  the vault entry is deleted by the next inventory change made while it is unlocked. In
  between it is unreferenced, so no app and no request can reach it.
- **An MCP client can set or clear an app's secret with consent.** It cannot read one.
  Since `network.http` injects secrets into headers (#568), setting one chooses which
  account the app talks to. The host's sentence for the prompt names only the action; the app and the
  setting are in the arguments the prompt carries, with the secret blanked, and a
  headless run needs `--mcp-allow-destructive` and `_confirm` as for any write.
- **The audit trail says a secret changed, not for which app.** The capability is
  sensitive, so `redact` blanks every argument value, the app ID included.
### Malicious catalog or network position

An attacker who can alter traffic, or who controls the srelens/extensions repository
without the catalog key.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| NET-1 | Intercept or redirect a download | T, S | A catalog package is a GitHub release asset named `*.srelens-extension`, and a signed app's must be in the same release as its manifest (`parse_catalog`, `check_package_url`). Every download is HTTPS with no credentials, fragment or custom port (`https_url`, over `check_url` in `crates/registry/src/extensions/http_policy.rs`, the rules `network.http` shares). Only the fixed `CATALOG_URL`, `github.com/<owner>/<repo>/releases/download/…` and `release-assets.githubusercontent.com` are fetched, and every redirect is checked against the same list, at most four of them (`allowed_download`, `download` in `crates/registry/src/extensions/catalog.rs`, `redirects` in `http_policy.rs`). Requests time out after 20 seconds, 10 to connect, and a body is read only to one byte past its limit. | Shipped |
| NET-2 | Swap the manifest between listing and review | T | The catalog pins each release's SHA-256. The downloaded bytes must match it, and their ID, version and API range must equal the entry's (`verify_manifest`). Review names the release by ID and checksum, so a changed catalog needs a new review. A package is pinned the same way, and its `extension.json` must be the release's manifest byte for byte; installing it names the release and the package checksum that were reviewed, and downloads and verifies it again (`verify_package`, `download_package`). | Shipped |
| NET-3 | List a malicious or look-alike app from a compromised catalog | S, T | The catalog is signed by the catalog role of the pinned root ([#559]; `accept` and `verify_catalog` in `crates/registry/src/extensions/catalog.rs`). Control of the `srelens/extensions` repository or of the network is not enough to change it: an unsigned, wrongly signed or altered catalog is refused and the last verified one kept. Even a validly signed catalog cannot make an app signed: that takes the publisher's key (PUB-1). Every entry is validated (`parse_catalog`), and every manifest passes the same rules and permission review as a pasted one. | Shipped ([#559]) |
| NET-4 | Freeze or roll back the catalog | T, D | A catalog carries a `version` and an `expires` time under its signature. A host refuses a catalog older than the last it verified, and a different catalog under a version it already holds, and keeps its cache (`accept`). It refuses an expired catalog, fetches again one that has expired in its cache however recently it was fetched, and does not install from an expired one: `extensions.catalogManifest` refuses it, and `extensions.configure` and `extensions.validate` verify signatures only under the build's shipped delegations while the cached catalog is expired, though its namespaces stay reserved (`CatalogCache::authority`). A failed or refused refresh keeps the cached catalog, marks it stale and shows the error with the original time (`load_with`). A catalog's delegations cannot take a publisher back below the delegation version this build shipped, change one at the same version, or give a namespace the build ships to another publisher (`Delegations::merged` in `trust.rs`). | Shipped ([#559]). Revocation in [#561] |
| NET-5 | Alter the catalog cache on disk | T | The cache holds the signed catalog as it was fetched, and every read verifies its signature again (`read_cache`); one that fails is fetched again. A cache written before [#559] holds an unsigned catalog and is never trusted. `source: catalog` only records that the installed bytes match a cached release; it grants nothing. | Shipped |

Residual risk:

- Whoever holds the catalog key can list an unsigned app with any name, description and
  repository link, served from any GitHub release. It is labelled unsigned and still needs
  the user's review. A repository link is catalog metadata and says nothing about who
  signed a release.
- A host that has never verified a catalog, or lost its cache, accepts any validly signed
  catalog that has not expired, however old. The expiry bounds how old.
- A catalog that expires unrefreshed stops every install from the catalog until the
  catalog is signed again ([trust.md](trust.md#publishing-the-signed-catalog)).

### MCP client abuse

An agent that is connected and authenticated, but acting on bad instructions.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| MCP-1 | Connect without authorization | S | `/mcp` always requires a bearer token, compared in constant time, and no production constructor serves without one (`router_with_auth` and `token_guard` in `crates/mcp/src/http.rs`, `crates/mcp/src/auth.rs`). The desktop's in-app server binds `127.0.0.1` only (`start_server` in `apps/desktop/src-tauri/src/mcp.rs`). Headless `--mcp-http` defaults to `127.0.0.1:8765` and refuses a non-loopback address with an error naming it, before the vault is opened or a token minted, unless the process was started with `--mcp-expose-http`; the listener `serve_http` accepts can only come from that check (`check_bind_addr` and `HttpListener::bind` in `crates/mcp/src/http.rs`, `run_mcp_http` in `apps/desktop/src-tauri/src/main.rs`). The startup message reports the address actually bound and says when it is exposed. Every route rejects a `Host` header that is not a loopback IP or `localhost` (`host_guard`), the same loopback test the bind uses; an exposed listener also accepts any IP-literal `Host`, never a hostname. That stops DNS rebinding from a browser, but it does not restrict where a request comes from, because any client can send `Host: localhost`; the bind is the network boundary. See [MCP.md](../MCP.md#security-model). | Shipped ([#607]). See residual risk |
| MCP-2 | Install or enable an app, change its grants or settings, set or clear its secrets, or roll it back | E | `extension.secretStore` is mutating and sensitive: its audit record blanks every argument value, and the desktop's consent prompt blanks the secret before the window sees it (`shown_args` in `apps/desktop/src-tauri/src/mcp_confirm.rs`). `extensions.configure` is mutating, so `handle_request` (`crates/mcp/src/stdio.rs`) asks the consent policy first (`consent_kind` in `crates/mcp/src/lib.rs`). In the desktop app that is a dialog (`PromptUser` in `apps/desktop/src-tauri/src/mcp_confirm.rs`). Headless, it needs both `--mcp-allow-destructive` and `"_confirm": true` (`FlagGated` in `crates/mcp/src/policy.rs`). With no policy, it is denied (`AlwaysDeny`). An install, including a package install ([#562]), is previewed by the host before the prompt: `install_preview` reads a package's manifest, signature and digest list through `extensions.packageManifest` or `extensions.catalogManifest`, never from what the caller says about it, and refuses a catalog package that is no longer the one named; `extensions.validate` then gives the access changes the prompt names and the reviewed revision the install runs with. A package's bytes, and any error its install raises, are left out of the audit record (`redact`, `redact_call_error` in `crates/capability/src/audit.rs`). | Shipped |
| MCP-3 | Start a GitOps write | E | `extensions.action` and all action primitives are mutating and consent-gated (`action_dispatch_uses_bound_api_and_mcp_cannot_bypass_confirmation` in `crates/registry/src/extensions/resource.rs`). The write fetches the resource again and refuses a changed UID or resourceVersion, a sync while an Argo CD operation is present, a Suspend of a suspended resource or a Resume of one that is not, reconciliation while suspended, and a resource being deleted. It then sends the UID and resourceVersion as PATCH preconditions (declared predicates and `request` in `crates/kube/src/action_primitives.rs`). A sync never enables pruning. | Shipped |
| MCP-4 | Call a removed, disabled or updated app through a stale tool list | E | An app's tools ([#574]) are one snapshot per inventory, rebuilt on every announced inventory write when the apps in use changed, and a change another process made is found on the next list or call, or by a push session's two-second poll (`AppTools` in `crates/registry/src/extensions/tools.rs`). The snapshot a rebuild replaces is revoked: each handler checks the flag `Registration::revoke` clears. A call is decided and run against one snapshot, so an app updated or disabled while its confirmation is open is refused rather than run as a version nobody was asked about (`McpServer::resolve` in `crates/mcp/src/lib.rs`; `a_call_runs_in_the_snapshot_it_was_asked_about_or_not_at_all`). Behind that, readers and actions run through `extensions.read`'s and `extensions.action`'s paths, which read the inventory on every call at the tool's revision. | Shipped |
| MCP-6 | Use an app's tool to skip the consent its host capability needs | E | An app's tool runs under its host capability's row through `Annotations::for_binding`, so it can be raised above that row and never lowered: a reader is read-only, an action is gated at its primitive's impact with the primitive's own confirmation sentence, and nothing in a manifest names annotations or a schema (`PluginHost::register_tools` in `crates/plugin-host/src/lib.rs`). The consent gate reads the tool's annotations like any other's, so a gated app tool asks the host confirmation in the app, needs the flag and `_confirm` headlessly, and is denied with no policy. A sidecar operation runs under a host row too (`sidecar_operation_annotations` in `crates/plugin-host/src/manifest/sidecar.rs`): read-only for an app that declares no action, since its sidecar then reaches only its readers through the broker; gated as its strongest declared action for one that does, since the sidecar may ask the broker to run it, and each such write is confirmed again, naming the app. It is sensitive either way, so its arguments are redacted whole. The web host serves no app tool and refuses a `plugin/` id before dispatch (`WEB_DENIED_PREFIX` in `crates/server/src/api.rs`). Tests: `a_tool_runs_under_its_host_capabilitys_annotations`, `a_declared_action_is_a_gated_tool_that_runs_through_the_host_action_path`, `every_installed_apps_operation_is_mcp_exposed`. | Shipped |
| MCP-5 | Deny having made a call | R | Capability calls are recorded, best-effort, in a local JSONL audit log with the source, the consent decision, the outcome and redacted arguments (`JsonlAuditLog` and `redact` in `crates/capability/src/audit.rs`). The record is written by the registry itself (`Registry::invoke_audited`), which is where the two calling surfaces meet, so both are covered: MCP through `handle_request` (`crates/mcp/src/stdio.rs`), and the desktop UI through `invoke_capability` (`apps/desktop/src-tauri/src/bridge.rs`), which used to reach the registry directly and leave nothing behind. MCP records every call it handles; from the UI, mutating and sensitive capabilities are recorded and plain reads are not (`is_audited_from_ui`). Each record names the app and revision it went through, the cluster and the object. Recording fails open by design, so that a lost log line never breaks a cluster operation. A failed rotation, write or permission change is swallowed, a failed open is only reported on stderr, and the call goes ahead either way. An executed call is recorded after it returns. | Shipped ([#555]) |
| MCP-6 | An app's log or runtime metrics reach an agent, and through it an LLM provider, carrying a secret or instructions a third party wrote | I, E | Each app has its own log, bounded to 1,000 lines plus its last 20 errors, and kept in memory only ([inspector.md](inspector.md)). Every line is redacted on the one way into the buffer (`AppLog::push` in `crates/plugin-host/src/app_log.rs`). A value the host knows is secret is always removed. Text shaped like a credential is removed on a best-effort basis: headers, bearer credentials, URL passwords, credential keys, JWTs, private keys and token prefixes. `extensions.inspect` and `extensions.logs` are UI-only (`Capability::ui_only`). `McpServer::new` drops them, so no MCP path lists or calls them, and `app_logs_and_metrics_never_leave_through_mcp_or_the_audit_trail` (`crates/registry/src/lib.rs`) checks that on both transports. Neither is written to disk or recorded in the audit trail. | Shipped ([#575]). Residual: pattern redaction cannot recognise every secret in free text, and a sidecar can hide one in its own log on purpose. The UI shows that log to the person who installed the app |

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
  unsigned read-only app with the grants it asks for. The unsigned-app setting ([#558])
  covers only apps that declare actions, and it is itself an `extensions.configure` action
  (`unsignedApps`), so the same agent can turn it on and then install an unsigned app that
  writes.
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
| WEB-1 | Read or change another user's apps, grants or settings | I, T | Each user's registry is built over that user's own inventory, their row of `extension_inventories` in the server database (`DbInventory` in `crates/server/src/app_inventory.rs`, passed by `UserEnvs::env_for` in `crates/server/src/users.rs` to `build_registry_for_user` in `crates/registry/src/lib.rs`). No capability takes a user or an inventory as input, so another user's app ID and revision name nothing (`two_users_see_only_their_own_apps` in `crates/server/src/api.rs`). `/api/settings` rows are a separate table and cannot place an inventory. A web user's registry has no secret store, so `extension.secretStore` is not registered (and is in `WEB_DENIED_CAPABILITIES` too), and `@srelens/core` refuses to send a secret on the web (`setExtensionSecret`), so the value never leaves the page. The row is read one byte past 1 MiB and parsed and re-verified exactly as the desktop file is. One lock per user, kept across environment rebuilds, orders that user's writes. | Shipped ([#515]) |
| WEB-2 | Start a GitOps write with no consent prompt | E | The host action primitives are in `WEB_DENIED_CAPABILITIES`, so a caller cannot name a kind and a template directly (`host_action_primitives_are_denied_on_web` in `crates/server/src/api.rs`). A declared action reaches its primitive only through `extensions.action`, over the user's own installed manifest, with the exact group/kind/plural allowlist, a recheck of revision, grants and cluster scope, and UID/resourceVersion preconditions, after the host confirmation the app's screen shows. The web host has no MCP server and no agent, so no request there is answered on a person's behalf. | Shipped ([#515]) |
| WEB-3 | Poison the catalog another user installs from | T | One catalog cache serves every user, and only the server writes it: users' `extensions.catalog` and `extensions.catalogManifest` read it and never fetch into it (`SharedCatalog` in `crates/registry/src/extensions/catalog.rs`, `no_capability_fetches_into_or_writes_the_shared_catalog`). The server fills it from the fixed catalog URL, validated as a desktop's is (`refresh_if_stale`, scheduled in `serve`, `crates/server/src/lib.rs`). A release is downloaded and verified again for each install, per user. | Shipped ([#515]) |
| WEB-4 | Use the shared server as a proxy into its network, or into its own loopback | I, E | A web user's registry has `network.http` ([#568]) only when the server's extension policy names hosts in `networkCeiling` ([#578]): otherwise `build_registry_for_user` builds the broker without it (`BrokeredNetwork::Off` in `crates/registry/src/lib.rs`), so an app that binds it is refused at install with "This host does not provide network.http", a stored one is refused the same way on every read (`validate_app`, run by `extensions.read` before it dispatches), and there is nothing to send (`a_web_users_apps_cannot_send_network_requests`, `a_web_users_apps_reach_the_network_only_under_a_ceiling`). With a ceiling (`BrokeredNetwork::Ceiling`), every request and every redirect must be to a host and port both the app's hosts and the ceiling allow, over HTTPS only (`Policy::check` in `crates/registry/src/extensions/network.rs`), and the ceiling is the policy the app was read under on that call. Plain HTTP is refused under a policy, and so is the per-app loopback switch, which a load also clears in what it reports, because loopback is the server itself (`govern` in `crates/registry/src/extensions/app_policy.rs`; `under_a_policy_an_app_cannot_open_plain_http_to_the_host` and `network_http_under_a_policy_reaches_only_hosts_its_ceiling_allows` in `crates/registry/src/extensions/app_policy_tests.rs`, `a_network_host_outside_the_ceiling_is_refused` in `crates/server/src/extension_policy.rs`). A request from the desktop leaves from the person's own computer, as their browser's would. | Shipped. See residual risk |
| WEB-5 | Install or use an app, publisher, capability or write the server's operator does not allow | E | The server reads an extension policy from its deployment config at startup, checked whole, and refuses to start on one it cannot read or that names an app, publisher, capability or host that could never match (`load` in `crates/server/src/extension_policy.rs`, `AppPolicy` in `crates/registry/src/extensions/app_policy.rs`). Every user's apps are held to it through their inventory store (`Apps::governed_by`), so every read of the inventory, which every `extensions.*` call makes, applies the policy in force then: an app it refuses is marked `policyBlocked` and disabled, and each reader, resource read, action, resolver and stream refuses it, including an app installed before the policy changed (`govern`, `read_under` in `crates/registry/src/extensions.rs`; `a_capability_outside_the_policy_is_refused_at_call_time_for_an_app_installed_before_it`, `a_blocked_app_can_be_neither_installed_nor_called`). Install, update, rollback and enable refuse such an app, and `extensions.validate` reports it as `EXTENSION_POLICY_REFUSED`. A signed app's publisher is the delegation its signature verified under on that load (`signedBy`, [#559]), a publisher only the catalog delegates to included (`a_catalog_delegated_publisher_is_held_to_the_publishers_the_policy_allows`); one that no longer verifies counts as unsigned. The verdict is never saved (`configure` changes the inventory as saved and governs only its answer), so a user cannot keep an app enabled past a policy, and lifting the policy does not leave apps disabled. A required app cannot be removed or disabled. Every signed-in user can read the policy (`GET /api/extension-policy`, and `policy` in `extensions.list`); nothing writes it over the API (`the_policy_is_read_over_the_api_and_written_nowhere`). | Shipped. Administrator role and policy administration planned in [#739] |

Residual risk:

- **A ceiling host is reached from the server's network position.** The operator chooses
  the ceiling, but a wildcard covers every one-label subdomain, including ones created
  later, and a name that resolves to an internal address is reached like any other.
  Within the ceiling, APP-2's residual risk about allowed hosts applies.
- **The policy changes only when the server restarts.** There is no administrator role,
  so nothing may change it while the server runs; [#739] adds one, and a write path
  behind it (`SharedPolicy::replace`).
- **A refusal is whole.** An app that requests one capability the policy does not allow,
  or declares a write action when writes are off, can't be used at all, its views
  included.
- **Required apps are kept, not provided.** The server never installs an app for
  anyone: a user who has not installed a required app is told to, and until then does
  not have it. A user can still limit a required app to clusters of their choosing, and
  a required app that another rule refuses stays disabled.
- **A policy change alone does not end an open app stream.** A read stream is checked
  again at its next read, and a watch at the next inventory write or reconnect. The web
  host runs no app streams today.

### Malicious package

An author, or anyone between the author and the host, who crafts a `.srelens-extension`
archive ([#562], [packages.md](packages.md)). The reader and installer are
`crates/registry/src/extensions/package.rs`; its tests are `package/tests.rs` and
`package_tests.rs` beside it, and `fuzzing::package` holds the same properties for the
`package` fuzz target.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| PKG-1 | Write outside the app's directory | T, E | Only regular files and directories are read; symbolic and hard links, devices, pipes and sparse files are refused by entry type (`read`). A path must be relative, ASCII `[A-Za-z0-9._-]` segments with none empty, `.`, `..`, dot-leading, dot-trailing or a Windows device name, within the fixed layout (`segments`, `check_file`, `check_directory`). Two paths that differ only in case, or a file where another path needs a directory, are refused, so a case-insensitive file system cannot merge them (`Names`). Files are written with `create_new` under a fresh staging directory, never over an existing file. | Shipped |
| PKG-2 | Change, add or remove a file after it was signed | T | `digests.json` names every other file with its size and SHA-256, the app ID and the version, and the signature covers the list's exact bytes through the one key lookup (`verify_for`). A file that is not listed, a listed file that is missing, a file whose bytes differ, a list that names another app or version than the manifest, and a list in any but its exact form (unknown or repeated fields, paths out of byte order) are each refused (`parse_digests`, `read`). | Shipped |
| PKG-3 | Exhaust memory or disk | D | The package file is at most 16 MiB, its files 64 MiB together uncompressed, 256 entries, with per-file limits by place (`MAX_*` in `package.rs`). A declared size is checked against its limit at the header, before the data is read, and the whole uncompressed stream is bounded, so a compression bomb stops at the limit (`Bounded`). Headers are read raw: no GNU long name or PAX header is buffered, since each is refused. A package sent to `extensions.configure` or `extensions.packageManifest` is refused before decoding if its base64 could exceed the limit (`limits::package`). | Shipped |
| PKG-4 | Hide content that another reader would unpack | T | Only zero padding may follow the tar end-of-archive block, and nothing may follow the gzip stream, so a second archive or member cannot hide behind the first. The same path twice is refused, rather than letting the later entry win. | Shipped |
| PKG-5 | Pose as a trusted app through its logo | S | A logo is read from the installed package for `extensions.list`, checked against the digest list its version was unpacked with, and sent as a `data:` URL (`installed_icon`); the UI draws only a `data:image/svg+xml` or `data:image/png` URL, as an image and never as markup (`ExtensionLogo` in `packages/ui-next/src/extensions/ExtensionLogo.tsx`). A quarantined app shows none. Nothing is chosen by app ID and no logo is bundled; who published an app is shown only by its signature label. | Shipped |
| PKG-6 | Leave a half-installed app after a crash | T | A verified package is read again into a private staging directory, synced, and moved into place by one durable rename (`publish_dir` in `crates/registry/src/durable.rs`); the inventory save that names the version, an atomic replace of its own, is what installs it. Versions no inventory entry names, current or kept, and leftovers of an interrupted install are removed after every change, under the inventory's lock (`prune`). A reinstall moves the existing copy of the version aside before moving the new one in; if it stops between the two, `prune` puts the aside copy back instead of removing it, because the inventory still names that version, and a failure to put it back at once is reported, not ignored (`unpack`). | Shipped |
| PKG-7 | Ship code the review does not show, or that the host would run outside the sandbox | E | Only an executable app's package may carry anything under `bin/<platform>/`, and exactly the binaries its manifest's `sidecar.binaries` names: an extra binary, or a named one the package lacks, is refused at review and at install (`check_installable`, `binary_problems`). A pasted or single-file manifest of the kind is refused. Only those binaries are unpacked runnable, by their owner (`Writer`); everything else stays `0600`. A binary is run only by the sandboxed supervisor ([#572], VULN-1), and only after it is checked again against the digest list it was unpacked with (`installed_binary`). | Shipped. Executable kind in [#574] (epic [#521]) |

Residual risk:

- An unsigned package proves only that its files are the ones its list names, not who made
  them; it installs as an unsigned local app, as a pasted manifest does.
- A package can carry any image, including another project's logo. The signature label is
  what says who published it.
- The web host keeps no app files, so it installs catalog releases from their single-file
  manifest and refuses packages ([#522]).

### Tampered local state

Out of scope as an attacker (see [Scope](#scope)), but the host still checks what it reads.

| ID | Threat | STRIDE | Mitigation | Status |
|---|---|---|---|---|
| LOCAL-1 | Edit the inventory to add, enable or widen an app | T, E | The inventory is parsed strictly: unknown fields, a `schemaVersion` other than 1, a file over 1 MiB and duplicate IDs are all fatal. An oversized file is refused after reading one byte past the limit, never loaded whole (see VULN-2). Each app's manifest and signature proof are checked again, and a failing app is quarantined; the reason, and who signed the app (`signedBy`), are recomputed on every load and never saved (`read_under`, `reverify`, `saved_form` in `crates/registry/src/extensions.rs`). A delegation written into a proof counts only when the pinned root's catalog role signed it and it covers the app's ID. An unsigned entry under a namespace this build ships a delegation for is quarantined too (see APP-9, [#602]). A hand-added unsigned entry under any other ID is still confined to declarative readers, because every call runs `validate_app`. | Shipped |
| LOCAL-2 | Corrupt or lose the inventory through concurrent writes or a crash | T | Saves are serialized by a cross-process lock (`write_lock` in `crates/registry/src/settings.rs`). Each save goes through one helper (`replace` in `crates/registry/src/durable.rs`, called by the file store in `crates/registry/src/extensions/store.rs` for `write` in `crates/registry/src/extensions.rs`): it writes a private temporary file beside the inventory, syncs it, and renames it over the inventory, so a concurrent writer or a crashed process leaves the old file or the new one, never a torn one. The rename is then made durable, so a power loss or kernel crash just after a save cannot bring back the previous inventory, and an app that was just disabled or removed cannot reappear enabled. On Unix the parent directory is opened before the rename and synced after it, and a directory created for the first save (`create_dir_all` in the same file) is synced into its parent. A save that fails returns an error with the old inventory intact; if only the sync after a completed rename fails, the save still succeeds and a warning says it may not survive a power loss, so the UI never reports a change as failed when it has been applied. On Windows the rename is `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`; Windows cannot flush a directory without administrator rights, so neither the rename's directory entry nor newly created parent directories from a first save are flushed here — both rest on the NTFS metadata journal. The catalog cache (`load_with` in `crates/registry/src/extensions/catalog.rs`) and settings (`write_document` in `crates/registry/src/settings.rs`) save through the same helper. | Shipped ([#611]) |
| LOCAL-3 | Edit an unpacked package | T | A package's directory is owner-only. Its logo is checked against the digest list each time it is read, and the list against the SHA-256 the inventory records, so a changed file is not shown (PKG-5); a signed package's proof is checked on every load (PUB-2). | Shipped |

## Open work

| Issue | Addresses |
|---|---|
| [#560] | PUB-3, PUB-6, PUB-7: root and key rotation |
| [#561] | PUB-3, PUB-4, PUB-6, NET-4: revocation and a kill switch |
| [#563] | APP-8, PUB-4: update checks and downgrade protection |
| [#564] | NET-3: additional catalog sources and imported roots |
| [#744] ([#521]) | VULN-1: the escape-hardening review of the [#572] supervisor and its sandbox backends |
| [#713] ([#521]) | VULN-1: host-enforced memory and CPU limits for sidecars on macOS, weaker than kernel enforcement. The issue is closed and the watchdog is built, but the switch to host enforcement waits for a check with Seatbelt on a macOS 27 Mac: until then srelens refuses to start any sidecar on macOS |
| [#578] ([#522]) | WEB-4, WEB-5: the extension policy, shipped in part; left is a catalog source and version pin for each app the operator makes available |
| [#739] ([#522]) | WEB-5: an administrator role, policy administration in the web settings, and enabling apps for users |
| [#39] | Scope: CSP, update chain and the rest of the host |

[#39]: https://github.com/srelens/srelens/issues/39
[#515]: https://github.com/srelens/srelens/issues/515
[#578]: https://github.com/srelens/srelens/issues/578
[#739]: https://github.com/srelens/srelens/issues/739
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
[#562]: https://github.com/srelens/srelens/issues/562
[#560]: https://github.com/srelens/srelens/issues/560
[#561]: https://github.com/srelens/srelens/issues/561
[#563]: https://github.com/srelens/srelens/issues/563
[#564]: https://github.com/srelens/srelens/issues/564
[#568]: https://github.com/srelens/srelens/issues/568
[#567]: https://github.com/srelens/srelens/issues/567
[#569]: https://github.com/srelens/srelens/issues/569
[#571]: https://github.com/srelens/srelens/issues/571
[#572]: https://github.com/srelens/srelens/issues/572
[#573]: https://github.com/srelens/srelens/issues/573
[#574]: https://github.com/srelens/srelens/issues/574
[#575]: https://github.com/srelens/srelens/issues/575
[#713]: https://github.com/srelens/srelens/issues/713
[#744]: https://github.com/srelens/srelens/issues/744
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
[#709]: https://github.com/srelens/srelens/issues/709
