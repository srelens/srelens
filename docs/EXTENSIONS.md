# Extension platform

Tracking: [#163](https://github.com/srelens/srelens/issues/163). Architecture:
[plugin ADR](design/plugin-architecture.md).

The platform has two authoring paths over one host:

- **Native srelens extensions:** a versioned manifest, brokered capabilities and
  UI contributions. Native extensions must not depend on Lens APIs.
- **Freelens/OpenLens extensions:** a compatibility adapter maps their lifecycle,
  APIs and UI registrations to the native layer. This is a required platform
  feature, not a replacement for native authoring.

## What this first implementation provides

`crates/plugin-host` provides the native API 0.1 contract and a runnable
**declarative host prototype**. A manifest binds existing host capabilities to
extension-specific operations and declares pages, detail tabs and row actions.
The broker validates explicit grants, fixes resource arguments, rejects caller
overrides, derives the public input schema and inherits host consent annotations.
It registers operations under `plugin/<extension-id>/<operation>` in the shared
capability registry. MCP discovers and gates those operations through its existing
request path.

Registration is all-or-nothing. Unregistering removes operations from the mutable
registry and revokes their handlers in older snapshots. Calls already admitted
may finish. A host must rebuild its MCP snapshot after a lifecycle change to
refresh discovery; an older snapshot may still list a revoked tool, but cannot
execute it. Do not advertise live tool-list updates until that lifecycle wiring
exists.

This is **not yet an app installer or a Freelens runtime**. The desktop/web UIs do
not load these manifests. Contribution descriptors are validated but not rendered.
No third-party code, subprocess, iframe, download, npm install or lifecycle script
is executed. Code-bearing and `lens-compat` manifests are rejected explicitly.
Signing, durable installation/grants/settings and OS sandboxing belong to the next
host stages below; the developer harness does not claim those protections.

## Try a native extension

The examples use the existing CRD reader. They do not install CRDs or connect to
any cluster merely by validating the manifest.

```sh
cargo run -p srelens-plugin-host --example extension_host -- \
  examples/extensions/argocd.json --grant=k8s.listCustomResource

cargo run -p srelens-plugin-host --example extension_host -- \
  examples/extensions/flux.json --grant=k8s.listCustomResource

# Generate the JSON schema used for authoring and validation.
cargo run -p srelens-plugin-host --example extension_host -- --schema
```

Argo CD binds `argoproj.io/v1alpha1` Applications and exposes health/sync columns.
Flux binds `kustomize.toolkit.fluxcd.io/v1` Kustomizations and
`helm.toolkit.fluxcd.io/v2` HelmReleases, including Ready status. The cluster must
serve those versions. RBAC/discovery errors remain errors, never empty success.

To expose only the example's operations to an MCP client, add `--mcp`. Supply
`context` explicitly on every call, and optionally `namespace`; omitting namespace
lists across namespaces. For example, a `tools/call` request is:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"plugin/org.srelens.argocd/applications","arguments":{"context":"staging","namespace":"argocd"}}}
```

The explicit `--grant` flag is a developer-harness grant for the named host
capability, not an application install-consent flow. The harness uses the normal
kubeconfig sources and never supplies credentials to extension code. Its default
MCP policy denies mutating and sensitive calls. Do not use `Registry::invoke` or
`McpServer::call_tool` as a substitute for the MCP request handler's consent gate.

## Authoring contract

See [argocd.json](../examples/extensions/argocd.json) and
[flux.json](../examples/extensions/flux.json) for complete manifests.

- `id`: reverse-domain identifier; `version`: SemVer; `srelensApiVersion`: a
  compatible range for the extension API, independently versioned from the app.
- `kind`: `declarative` in this prototype.
- `permissions`: exact host capability IDs used by bindings. The host supplies
  grants separately; manifest declarations are not self-authorization.
- `capabilities`: local name/title, trusted-host `target`, fixed `arguments`, and
  allowed caller `inputs`. Every required target argument must be supplied by one
  of those sources. Their keys cannot overlap. Target handlers validate values.
- `contributions`: `pages`, `detailTabs`, `rowActions`, each referencing a declared
  local capability. Detail/action `forKinds` values include the API group to avoid
  confusing CRDs with equally named built-ins. Use `/Pod` for the core API
  group and `argoproj.io/Application` for an Argo CD custom kind.

The author cannot supply a handler, JavaScript, schema or safety annotations.
Annotations come from the trusted core; mutations, destructive operations and
sensitive reads cannot lose their confirmation requirement. Fixed arguments are
excluded from the public input schema and rejected if a caller tries to override
them. Only host capabilities can be targets; plugin-to-plugin chaining is not
supported in API 0.1.

## Delivery plan and exit checks

Each stage is a separate reviewable PR; #163 stays open until both authoring paths
work end-to-end.

1. **Native contract and broker (this PR):** executable manifest examples,
   collision/permission/input validation, revocation and real MCP consent tests.
2. **Application lifecycle and declarative UI:** backend-owned install inventory,
   grants and per-extension settings; atomic save/update/remove; developer mode
   off by default for unsigned local manifests; signed distributed packages;
   enable/disable and contribution removal in both classic and new UI. One sample
   must visibly add a page, detail tab and menu action without editing app source.
   Cluster identity belongs in extension routes and broker calls. Web instances
   must use per-user state, never a process-global extension inventory.
3. **Native executable and renderer SDKs:** supervised JSON-RPC sidecars and
   sandboxed iframe bridge; managed runtime, quotas, cancellation and teardown;
   deny ambient network/filesystem/process access. Unsupported OS sandbox backends
   must refuse executable extensions. Persist settings only through the backend.
4. **Freelens/OpenLens adapter:** package API aliases, main lifecycle, React/MobX
   iframe runtime, Kubernetes stores/watch adapters and registration translation.
   Rebuild and run the real Flux extension first, then ArgoCD. Unsupported APIs
   must fail with the API name and migration guidance; no silent stub success.
5. **GitOps workflows and distribution:** native ArgoCD/Flux pages and detail
   panels, confirm-gated Sync/Refresh/Reconcile, reference Trivy integration,
   signed update verification, permission-diff consent, marketplace and revocation.

The sequence refines the ADR: the declarative broker can be proven without
executing untrusted code, but neither executable runtime is released before
sandboxing and trust verification. Native and compatibility runtimes share grants,
registry, contribution lifecycle and consent; they do not create parallel stores.

## Freelens/OpenLens compatibility matrix

The following is a **porting plan**, not a supported-runtime claim. As of this
foundation, existing extension packages cannot run in srelens.

Source audit on 2026-09-11:
[Flux package](https://github.com/freelensapp/freelens-fluxcd-extension/blob/e76a2f11a77add08dd9ed0ace02d9f2f59bdd58d/package.json),
[Flux renderer](https://github.com/freelensapp/freelens-fluxcd-extension/blob/e76a2f11a77add08dd9ed0ace02d9f2f59bdd58d/src/renderer/index.tsx),
[ArgoCD package](https://github.com/Sebastian-Prokesch/freelens-argocd-extension/blob/e41c3fed658472b239bf6c49d6d5cc6060cd7522/package.json),
[ArgoCD renderer](https://github.com/Sebastian-Prokesch/freelens-argocd-extension/blob/e41c3fed658472b239bf6c49d6d5cc6060cd7522/src/renderer/index.tsx).
Both audited projects expose separate main/renderer entries and React 17/MobX
renderer dependencies. Srelens uses a Tauri webview; those bundles cannot be loaded
as host-renderer modules or assumed to share its React instance.

| Extension surface | Native destination | Compatibility work still required |
| --- | --- | --- |
| `Main.LensExtension`, `Renderer.LensExtension`, activation/disposal | Supervised instance lifecycle | Node host + iframe adapter, cleanup on failure/disable |
| `@freelensapp/extensions`, legacy `@k8slens/extensions` | `@srelens/lens-compat` API aliases | Versioned export mappings and actionable unsupported errors |
| Cluster pages and menus | Page contributions and cluster-scoped routes | Translate registrations and mount isolated React components |
| Kubernetes detail/menu registrations | Detail tabs and row actions | Match group/kind; pin resource identity and enforce consent |
| `KubeObject`, `KubeApi`, stores and watches | Brokered CRD capabilities/streams | Store semantics, reconnect, cancellation and disposal |
| React 17, MobX, renderer styling | Isolated iframe runtime | Bundle dependencies there; adapt theme tokens without host CSS access |
| `clusterFrameComponents` / dialogs | Isolated overlay contribution | Focus, keyboard dismissal and bridge ownership |
| Extension preferences/store | Backend-owned per-extension settings | Serialization, scoped keys, migration and observable adapter |
| Node filesystem/network/process/Electron APIs | Explicit broker methods | Deny ambient access; add allow-listed adapters case by case |

Binary/drop-in compatibility with arbitrary historical Lens packages is not
promised. The initial acceptance target is rebuilding representative supported
extensions against the compatibility shim, then demonstrating their pages, resource
reads, settings and guarded mutations inside the app. OpenLens API coverage must
be tested against real legacy imports rather than inferred from Freelens branding.
