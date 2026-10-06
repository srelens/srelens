# Security

The trust boundary for apps today, and what it does not yet cover. The
[threat model](threat-model.md) takes it threat by threat, with the code behind each
mitigation and the risk that remains.

## What holds today

- **Third-party code runs only in a sandbox.** A declarative app runs no code: no
  JavaScript, subprocess, iframe, npm install or lifecycle script. An executable app's
  binary runs only as a sidecar in the OS sandbox, from the package it was installed
  with and checked against it first, with no kubeconfig, no network, none of srelens's
  environment and one directory of its own; it is refused wherever no sandbox exists,
  and an unsigned one needs the unsigned-apps setting
  ([manifest.md](manifest.md#executable-apps)). Executable apps are a preview: they run
  out of the box on Windows only, and Linux and macOS are set out in
  [where they run](manifest.md#where-executable-apps-run).
- **No ambient access.** Apps never receive kubeconfig, tokens, files or network
  access. Every call goes through the broker with fixed arguments, under the selected
  cluster's RBAC ([permissions.md](permissions.md)).
- **Queries are the host's to bind.** A metric, log or trace provider (#569) declares a
  template; the host binds the view's names into double-quoted strings, escaped, and
  refuses a template that puts one anywhere else, so a cluster, namespace or pod name
  never becomes PromQL, LogQL or TraceQL. What comes back is read by the host into a
  chart, lines or a list of traces, and drawn by the host.
- **Network requests are brokered.** An app that requests `network.http` reaches only
  the hosts it lists and the person approved, over HTTPS (plain HTTP only to this
  computer, per app, when a person allows it). The host sends each request and checks
  it and every redirect; a secret goes into a header by reference and never leaves for
  another origin ([manifest.md](manifest.md#network-requests)).
- **Pods are reached only in scope.** Logs, exec and port-forwards reach only the pods
  an object of a kind the app reads selects with its own selector, or pods in the
  namespaces a person granted, and the host matches every pod itself on every open. An
  exec binding runs one command fixed in the manifest, never a shell, and only after
  the host confirmation names the pod, container and exact command; a forward listens
  on a local port the host picks and closes, with every connection, when its view does
  ([manifest.md](manifest.md#logs-exec-and-port-forwards)).
- **Consent cannot be weakened.** Bindings inherit the host capability's annotations,
  and so do an app's MCP tools ([MCP.md](../MCP.md#installed-apps-tools)). Mutating
  operations stay behind the MCP consent gate, and every UI action opens a host-owned
  review.
- **Reviews cannot go stale.** The review keeps the UID and resourceVersion the reader
  saw, and the backend's conditional PATCH rejects the write if the resource changed
  ([capabilities.md](capabilities.md#declared-gitops-actions)).
- **An update shows what access it changes.** The host compares the incoming manifest's
  access with the installed revision's: the grants, what each reader binds, the settings
  it keeps secrets for, each action, each host `network.http` may reach, and each
  namespace, command and pod scope of its pod bindings. The review
  in Settings → Apps lists what is
  added and removed before what is unchanged, and the consent prompt for an install over
  MCP names the added and removed access. The update must name the installed revision it
  was reviewed against, and is refused if the app has changed since. A rollback gets no
  such comparison: its review compares capability IDs, and for `network.http` the hosts
  and requests, but not reader or action bindings
  ([threat-model.md](threat-model.md#malicious-app)).
- **Publisher identities are reserved.** The signed catalog delegates app ID namespaces
  to publishers, `org.srelens` to srelens, and an ID in one installs only with that
  publisher's signature, as a manifest or as a package. A pasted manifest cannot take a
  publisher's ID, and a publisher's key cannot sign outside its namespaces
  ([distribution.md](distribution.md#signed-releases-and-publishers), [trust.md](trust.md)).
- **The catalog is signed.** The host trusts a catalog only when the catalog role of
  the root it pins signed it, and refuses one that has expired or is older than the last
  it verified, keeping that one ([trust.md](trust.md#what-the-host-refuses)).
- **A logo never implies trust.** An app's logo comes only from its installed package, as
  an image checked against the package's digest list; nothing is chosen by app ID, and the
  signature label, not the logo, says who published an app ([packages.md](packages.md#logos)).
- **A package is taken whole.** Every file is covered by one digest list and one
  signature; links, traversal, extra, missing or changed files and oversized archives are
  refused before anything is unpacked, into a private directory
  ([packages.md](packages.md#what-the-host-refuses)).
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
- **The web host keeps apps per user.** Each user's inventory is their own database
  row, the one shared catalog cache is written only by the server, and no app secret is
  kept there until per-user secret storage exists
  ([capabilities.md](capabilities.md#web-host)).
- **The web host holds every user's apps to its operator's policy.** Allowed and
  blocked apps, publishers, unsigned apps, capabilities, write actions, a host ceiling
  for `network.http` and required apps are applied on every call, not only at install,
  so an app installed before the policy changed is refused from its next call
  ([WEB.md](../WEB.md#extension-policy)). The policy comes from deployment config and
  nothing writes it over the API.
- **Every write is recorded locally, wherever it came from.** A mutating or
  sensitive capability call is appended to `audit.jsonl` whether an agent made
  it over MCP or a person clicked it in srelens — with the source, the app and
  revision it went through, the cluster, the object and the outcome. The sink
  sits beside the capability registry, which is the one place both paths meet
  (`crates/capability/src/audit.rs`), so a new surface cannot acquire writes
  without acquiring the record. App exec and port-forward sessions, which run as
  streams rather than calls, write to the same sink when they start or are refused
  ([threat-model.md](threat-model.md#mcp-client-abuse)). Argument values are
  redacted before anything is written, and the file never leaves the machine.

## Not yet protected

The platform does not claim these protections yet:

- signing key rotation ([#560](https://github.com/srelens/srelens/issues/560))
- revocation and a kill switch ([#561](https://github.com/srelens/srelens/issues/561))
- executable apps' sandbox, against a sidecar that tries to break out: the
  escape-hardening review ([#744](https://github.com/srelens/srelens/issues/744)), and
  memory and CPU limits on macOS: its watchdog is built but not yet checked with
  Seatbelt on a macOS 27 Mac, and macOS refuses every sidecar until it is
  ([#713](https://github.com/srelens/srelens/issues/713))
