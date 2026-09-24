# Security

The trust boundary for apps today, and what it does not yet cover. The
[threat model](threat-model.md) takes it threat by threat, with the code behind each
mitigation and the risk that remains.

## What holds today

- **No third-party code runs.** No JavaScript, subprocess, iframe, npm install or
  lifecycle script. Catalog downloads contain JSON data only.
- **No ambient access.** Apps never receive kubeconfig, tokens, files or network
  access. Every call goes through the broker with fixed arguments, under the selected
  cluster's RBAC ([permissions.md](permissions.md)).
- **Consent cannot be weakened.** Bindings inherit the host capability's annotations.
  Mutating operations stay behind the MCP consent gate, and every UI action opens a
  host-owned review.
- **Reviews cannot go stale.** The review keeps the UID and resourceVersion the reader
  saw, and the backend's conditional PATCH rejects the write if the resource changed
  ([capabilities.md](capabilities.md#declared-gitops-actions)).
- **Official identities are reserved.** IDs under `org.srelens.` install only with the
  srelens publisher signature, so a pasted manifest cannot take an official app's ID
  or logo ([distribution.md](distribution.md#signed-official-releases)).
- **Names display as written.** App names, titles and groups, and catalog names and
  descriptions, refuse bidirectional overrides, zero-width characters and other Unicode
  format characters ([specification.md](specification.md#identifiers)).
- **One bad app is contained.** An app that fails re-verification is quarantined on
  its own ([architecture.md](architecture.md#quarantine)).
- **Downloads are constrained.** Only the fixed catalog URL, GitHub release assets and
  GitHub's release-asset redirects, with bounded sizes, timeouts and redirect counts.
  The frontend never fetches catalog URLs or writes catalog caches to browser storage.
- **App secrets are write-only and kept in the encrypted secrets vault.** A
  `secret-reference` setting's value is kept in srelens's encrypted secrets vault
  (`secrets.enc`), whose one master key is held by the OS keychain or derived from the
  master password. It needs the `extension.secretStore` permission, is never returned to
  the app, the UI, MCP, an export or a log, is refused rather than stored when the
  vault's key is only in a plain file beside it (no keychain and no master password) or
  the vault is locked, and is deleted with the app
  ([manifest.md](manifest.md#secret-settings)).
- **The web host refuses app capabilities** until state is per user
  ([capabilities.md](capabilities.md#web-host)).
- **Every write is recorded locally, wherever it came from.** A mutating or
  sensitive capability call is appended to `audit.jsonl` whether an agent made
  it over MCP or a person clicked it in srelens — with the source, the app and
  revision it went through, the cluster, the object and the outcome. The sink
  sits beside the capability registry, which is the one place both paths meet
  (`crates/capability/src/audit.rs`), so a new surface cannot acquire writes
  without acquiring the record
  ([threat-model.md](threat-model.md#mcp-client-abuse)). Argument values are
  redacted before anything is written, and the file never leaves the machine.

## Not yet protected

Declarative support does not claim these protections:

- third-party publisher signing ([#559](https://github.com/srelens/srelens/issues/559))
- signing key rotation ([#560](https://github.com/srelens/srelens/issues/560))
- revocation and a kill switch ([#561](https://github.com/srelens/srelens/issues/561))
- executable apps and OS sandboxing ([#521](https://github.com/srelens/srelens/issues/521))
- a permission diff on update ([#554](https://github.com/srelens/srelens/issues/554))
