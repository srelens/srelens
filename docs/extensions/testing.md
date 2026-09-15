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

A manifest that breaks the contract is not registered. The harness prints every
problem, one per line as `path: message (CODE)`, and exits with a failure status. The
codes are listed in [Validation errors](specification.md#validation-errors). It checks
the manifest's own rules; the desktop app's narrower rules are checked by
`extensions.validate` and the install review in Settings → Apps.

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
| `cargo test -p srelens-plugin-host --lib fuzzing` | Manifest decoding, validation and parsing on arbitrary bytes and on edits of the example manifests: no panic, a value or a coded problem, the 256 KiB limit to the byte, and an accepted manifest re-serializes to an equal one |
| `cargo test -p srelens-registry` | Inventory lifecycle, quarantine, catalog parsing and caching, signing, app capabilities |
| `cargo test -p srelens-registry --lib fuzzing` | The same properties for catalog parsing, publisher signature verification and the inventory reader with its legacy migration, starting from `crates/registry/tests/fixtures` |
| `cargo test -p srelens-kube --lib gitops` | Resource inspection, events, GitOps action allowlist, guards and conditional PATCH |
| `cargo test -p srelens-server` | Web-host denials |
| `packages/core/src/lib/extensionManifestSchema.test.ts` | Every example manifest validates against the committed schema and names it in `$schema` |
| `packages/core/src/lib/extensionTypes.test.ts` | The TypeScript manifest and inventory types have the Rust field names and optionality, from `extension-inventory.schema.json` |
| `packages/ui-next/src/extensions/*.test.tsx` | Settings → Apps, catalog, workspace, resource details |

The extension capabilities are not yet covered by the live-cluster e2e suite
([#536](https://github.com/srelens/srelens/issues/536)). An authoring CLI with a test
command is planned ([#577](https://github.com/srelens/srelens/issues/577)).

## Fuzzing

The parsers that read extension input from outside the host have cargo-fuzz targets in
`fuzz/`: `manifest`, `catalog`, `signed-manifest` and `inventory`. `signed-manifest` reads
one byte giving the signature's length, the signature, then the manifest.

Each target calls one function in its crate's `fuzzing` module, and the `fuzzing` property
tests above call the same function, so a property is written once. `cargo test` runs a
fixed set of generated cases, the same on every run, on stable and on every platform. The
fuzzer keeps looking for new ones. When it finds a crash, add the input it saved to that
module's tests as a regression test, then fix it.

The **Fuzz** workflow (`.github/workflows/fuzz.yml`) runs every target for a minute on a
pull request that touches the parsers, their fixtures or `Cargo.lock`, and for fifteen
minutes each night. Crashing inputs are uploaded as the `fuzz-artifacts` artifact. It is
not a required check.

libFuzzer needs a nightly toolchain and does not build on Windows. On Linux or macOS:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cp Cargo.lock fuzz/Cargo.lock   # build the dependency versions the app ships
sh fuzz/seed.sh                 # start from the examples and fixtures
cargo +nightly fuzz run manifest -- -dict=fuzz/extensions.dict -max_total_time=300
```

`+nightly` is needed because `rust-toolchain.toml` pins the repository to stable. To
explore past the fixed cases without the fuzzer, on any platform, give the property tests a
seed and a count:

```sh
PROPTEST_RNG_SEED=7 PROPTEST_CASES=10000 cargo test -p srelens-registry --lib fuzzing
```

## On Windows

- The tests that verify the published Argo CD signature read
  `crates/registry/tests/fixtures/argocd-manifest.json`, which `.gitattributes` keeps
  byte-exact, so `core.autocrlf=true` does not break them.
- The `srelens-kube` lib tests do not compile on Windows because of a Unix-only
  symlink test in `toolbox.rs` (see AGENTS.md).
