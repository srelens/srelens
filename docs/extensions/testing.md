# Testing

How to exercise an app manifest and the extension platform itself.

## Developer harness

`crates/plugin-host` ships a harness that registers one manifest against the real host
capabilities. It installs nothing, so it accepts the example IDs unchanged, and it never
connects to a cluster merely to validate a manifest.

```sh
cargo run -p srelens-plugin-host --example extension_host -- \
  examples/extensions/argocd.json --grant=k8s.listCustomResource

cargo run -p srelens-plugin-host --example extension_host -- \
  examples/extensions/flux.json --grant=k8s.listCustomResource --grant=k8s.listEvents

# Print the manifest JSON Schema.
cargo run -p srelens-plugin-host --example extension_host -- --schema
```

`--grant` is a developer-harness grant for the named host capability, not the
application's install-consent flow. The harness uses the normal kubeconfig sources and
never passes credentials to app code.

To expose only the manifest's operations to an MCP client, add `--mcp`. Supply
`context` on every call, and optionally `namespace`; omitting it lists across
namespaces. For example:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"plugin/org.srelens.argocd/applications","arguments":{"context":"staging","namespace":"argocd"}}}
```

Its default MCP policy denies mutating and sensitive calls. Do not use
`Registry::invoke` or `McpServer::call_tool` as a substitute for the MCP request
handler's consent gate.

## Platform test suites

| Suite | Covers |
|---|---|
| `cargo test -p srelens-plugin-host` | Manifest parsing and validation, API version negotiation, broker registration, revocation, consent, and that `schemas/extension-manifest.v0.1.json` equals the generated schema |
| `cargo test -p srelens-registry` | Inventory lifecycle, quarantine, catalog parsing and caching, signing, app capabilities |
| `cargo test -p srelens-kube --lib gitops` | Resource inspection, events, GitOps action allowlist, guards and conditional PATCH |
| `cargo test -p srelens-server` | Web-host denials |
| `packages/core/src/lib/extensionManifestSchema.test.ts` | Every example manifest validates against the committed schema and names it in `$schema` |
| `packages/ui-next/src/extensions/*.test.tsx` | Settings → Apps, catalog, workspace, resource details |

The extension capabilities are not yet covered by the live-cluster e2e suite
([#536](https://github.com/srelens/srelens/issues/536)). An authoring CLI with a test
command is planned ([#577](https://github.com/srelens/srelens/issues/577)).

## On Windows

- The tests that verify the published Argo CD signature read
  `crates/registry/tests/fixtures/argocd-manifest.json`, which `.gitattributes` keeps
  byte-exact, so `core.autocrlf=true` does not break them.
- The `srelens-kube` lib tests do not compile on Windows because of a Unix-only
  symlink test in `toolbox.rs` (see AGENTS.md).
