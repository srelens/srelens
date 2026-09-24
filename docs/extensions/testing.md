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
| `cargo test -p srelens-plugin-host` | Manifest parsing and validation, API version negotiation, broker registration, revocation, consent, and that `schemas/extension-manifest.v0.3.json` equals the generated schema |
| `cargo test -p srelens-plugin-host --lib fuzzing` | Manifest decoding, validation and parsing on arbitrary bytes and on edits of the example manifests: no panic, a value or a coded problem, the 256 KiB limit to the byte, and an accepted manifest re-serializes to an equal one |
| `cargo test -p srelens-registry` | Inventory lifecycle, quarantine, catalog parsing and caching, signing, app capabilities |
| `cargo test -p srelens-registry --lib fuzzing` | The same properties for catalog parsing, publisher signature verification and the inventory reader with its legacy migration, starting from `crates/registry/tests/fixtures` |
| `cargo test -p srelens-kube --lib gitops` | Resource inspection, events, GitOps action allowlist, guards and conditional PATCH |
| `cargo test -p srelens-server` | Web-host denials |
| `packages/core/src/lib/extensionManifestSchema.test.ts` | Every example manifest validates against the committed schema and names it in `$schema` |
| `packages/core/src/lib/extensionTypes.test.ts` | The TypeScript manifest and inventory types have the Rust field names and optionality, from `extension-inventory.schema.json` |
| `packages/ui-next/src/extensions/*.test.tsx` | Settings → Apps, catalog, workspace, resource details |
| `cargo test -p srelens-registry --lib budget_tests` | [Performance budgets](#performance-budgets), the host's half: loading 50 apps, a call's own overhead, resolving 1,000 rows, closing a view |
| `packages/ui-next/src/extensions/extensionBudgets.test.tsx` | [Performance budgets](#performance-budgets), the client's half: the sidebar's apps, the app list, closing a view |
| `cargo test -p srelens-desktop --test e2e -- --ignored` (kind) | Every `extensions.*` capability, `k8s.getCustomResource` and the action primitives against a live cluster: the example Flux and Argo CD apps are validated, installed, listed, read and inspected; the historical signed API 0.1 release is refused as incompatible; suspend, resume and refresh land on the object; a stale `resourceVersion` is refused; a disabled app stops reading |
| `.github/workflows/extension-catalog.yml` (daily) | The ignored `public_catalog_release_smoke`: every release in the live public catalog downloads, matches its checksum and publisher signature, and validates on this host |

### Live cluster

The e2e suite applies `apps/desktop/src-tauri/tests/fixtures/gitops-crds.yaml` itself:
minimal Flux `Kustomization` and Argo CD `Application` CRDs with the upstream groups,
versions and plurals, and no controllers. Reads and conditional patches need nothing
more. Run it against a throwaway kind cluster (see
[DEVELOPMENT.md](../DEVELOPMENT.md#live-cluster-tests)). It refuses a cluster that
already has real Flux or Argo CD CRDs, because teardown deletes the CRDs it applied,
and deleting a CRD deletes every object of that kind.

The examples use reserved `org.srelens.` IDs, which install only with the publisher's
signature, so the suite installs them as `org.example.flux` and `org.example.argocd`. The
signed Argo CD release is installed with its signature under its own ID.

The catalog check needs the network and no cluster:

```sh
cargo test -p srelens-registry --lib -- --ignored --exact \
  extensions::catalog::tests::public_catalog_release_smoke
```

An authoring CLI with a test command is planned
([#577](https://github.com/srelens/srelens/issues/577)).

## Performance budgets

The roadmap sets three targets for the platform: loading the installed apps' manifests
in under 50 ms, building the navigation apps contribute in under 10 ms, and keeping a
typical host call's own overhead under 5 ms. It also requires batching, deduplication
and cancellation. [#581](https://github.com/srelens/srelens/issues/581) measures them in
two suites, one per side of the bridge:

| Budget | What is measured | Target |
|---|---|---|
| `inventory-load-50-apps` | `extensions.list` over 50 installed apps (45 Argo CD, 5 Flux, from `examples/extensions`) | 50 ms |
| `host-call-overhead-1-app`, `-50-apps` | A broker call with nothing to read, from the bridge's entry: it resolves the context and loads the whole inventory to authorize one app | 5 ms |
| `resolve-columns-1000-rows-warm` | `extensions.resolveColumns` over 1,000 rows, three joined columns and a badge, from the snapshot | none, tracked |
| `close-view-6-streams` | The host's side of closing a view that holds three watches and three read streams | 5 ms |
| `navigation-build-50-apps` | The sidebar's Apps group from 50 apps (`appNavigation.ts`) | 10 ms |
| `inventory-store-load-50-apps` | The app list's own part of loading 50 apps: from the host's answer to a ready list | 50 ms |
| `resolve-columns-lists-per-join` | Kubernetes lists for five resolves of 1,000 rows (four at once, then a refresh) over three joins on two readers, against a loopback API server | exactly one per joined reader |
| `close-view-releases` | Watch sessions and streams the host still holds after views are closed | none left |
| `client-view-close-releases` | Streams left open on the host, and channel listeners left in the client, after a view with three apps' watches unmounts | none left |

Counts are held exactly, on every run. Times are held to a **ceiling**, not to the
target: `backend` runs the Rust suite as a debug build under coverage instrumentation on
a shared runner, many times slower than the release build a target describes, and a test
that failed at the target there would fail on every run. A ceiling sits far enough above
the target to catch a path that became an order of magnitude slower, and never a noisy
runner. When one fails, find the change; do not raise the number.

With `SRELENS_PERF_REPORT_DIR` set, each suite writes one JSON file per measurement into
that directory: the median, fastest and slowest run, the target, the ceiling, whether the
target was met, the commit, and the Rust build or the Node version. A debug build leaves `withinTarget` empty,
because the targets describe a release build. The `extension budgets (release)` job in
`ci.yml` runs both suites as a release build without coverage and uploads the directory
as the `extension-budgets` artifact on every run, so a drift is visible across runs long
before it reaches a ceiling. To compare against the targets locally:

```sh
SRELENS_PERF_REPORT_DIR=/tmp/budgets cargo test --release -p srelens-registry --lib budget_tests -- --test-threads=1 --nocapture
SRELENS_PERF_REPORT_DIR=/tmp/budgets pnpm exec vitest run packages/ui-next/src/extensions/extensionBudgets.test.tsx
```

The broker loads the whole inventory on every call, so a call's overhead grows with the
number of installed apps; at 50 it is close to its target. See
[PERFORMANCE.md](../PERFORMANCE.md#extension-platform-budgets) for the figures.

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

`+nightly` is needed because `rust-toolchain.toml` pins the repository to stable. A
prebuilt cargo-fuzz (from `cargo binstall`, as CI gets it) is a static musl binary and
builds for musl by default, which AddressSanitizer refuses; pass
`--target x86_64-unknown-linux-gnu` to both `fuzz build` and `fuzz run`. To
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
