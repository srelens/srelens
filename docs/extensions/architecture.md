# Architecture

How srelens hosts apps today. The decisions behind this design, and the parts still
ahead, are in the [plugin ADR](../design/plugin-architecture.md) and
[#163](https://github.com/srelens/srelens/issues/163).

## The broker

`crates/plugin-host` implements the extension API (see
[specification.md](specification.md)) and a declarative broker. A manifest binds
existing host capabilities to app-specific operations and declares pages, detail tabs
and detail links.

The broker:

- validates explicit grants
- fixes resource arguments and rejects caller overrides
- derives the public input schema from the allowed inputs
- inherits the host capability's consent annotations, which an app cannot weaken

Operations are addressed as `plugin/<app-id>/<operation>`. The app's own screens reach
them through the `extensions.*` capabilities, which register them for one call at a time
with the app's settings as saved then. MCP clients reach them as tools
([#574](https://github.com/srelens/srelens/issues/574), `crates/registry/src/extensions/tools.rs`):
one snapshot per inventory in a process, built by `PluginHost::register_tools` from the
apps in use, whose readers and actions route through the same broker paths as
`extensions.read` and `extensions.action`, and whose sidecar operations route to the
app's sidecar. A pod binding is a stream a view opens, and never a tool.

Registration is all-or-nothing. Every announced inventory write rebuilds the MCP snapshot
when the apps in use changed, and revokes the one it replaces: a caller still holding it
may list a revoked tool but cannot execute it; calls already admitted may finish. The
servers send `notifications/tools/list_changed` for each rebuild, and notice a change
another process made within a poll ([MCP.md](../MCP.md#installed-apps-tools)).

## Sidecars

Executable apps (API 0.6, [#574](https://github.com/srelens/srelens/issues/574)) run out
of process, as supervised sidecars in the OS sandbox, speaking JSON-RPC over stdio
([sidecar-protocol.md](sidecar-protocol.md)). The supervisor and its per-OS backends are
in `crates/plugin-host/src/sidecar/`; the registry starts one per executable app in use,
on its first operation call, and stops it when the app changes
(`crates/registry/src/extensions/sidecars.rs`). Each is started with its app's data
directory, a broker over the MCP host's registry, and its app's log, and shows in the
Inspector. The broker's consent is the MCP host's: the desktop app's confirmation
prompt, naming the app, or `NoConsent` headless. See
[Executable apps](manifest.md#executable-apps).

Executable apps are a preview. They run out of the box on Windows. On Linux they need the
sandbox launcher `srelens-sandbox-launch`, a cgroup v2 directory delegated to srelens and a
kernel with Landlock enabled. The bundles do not ship the launcher, so set up the launcher
and the cgroup by hand, naming them with `SRELENS_SANDBOX_LAUNCHER` and
`SRELENS_SANDBOX_CGROUP_ROOT` ([recipe](../DEVELOPMENT.md#everyday-commands)). On macOS they do not
run yet. srelens refuses to start any sidecar until its memory and CPU watchdog has
been checked with Seatbelt on a macOS 27 Mac.

## App lifecycle

Both desktop designs manage apps through **Settings → Apps**, from the catalog or a
pasted local manifest. The backend owns installation, grants, enable/disable,
updates, removal and per-app typed settings. Installation requires an explicit review
of the requested permissions. There is no developer mode.

Installation and enablement are app-wide, not per kubeconfig context. Enabled pages
are available on every cluster; each page checks the APIs it needs when it opens (see
[ui-contributions.md](ui-contributions.md#requirement-checks)).

## The inventory

The inventory lives next to the desktop settings file: the settings path with its
extension replaced by `extensions.json`, so `settings.extensions.json`.

- Saves use a private temporary file, sync and atomic replacement under a
  cross-process lock.
- Updating an ID keeps the settings the new version still declares and accepts, and
  assigns a new revision. The app keeps up to
  the last three versions it replaced, fewer when they would take the inventory past
  1 MiB; restoring one grants its permissions again after
  review, keeps the settings it declares and assigns a new revision
  (see [migration.md](migration.md#rolling-back)).
- Each installed version records its source: `catalog` when its exact bytes are a
  release in the cached catalog, otherwise `local`. The host decides this, not the caller.
- An app may be limited to chosen kubeconfig contexts, kept by context key (`k8s.listContexts` `key`: the stable ID with `#` and `%` encoded, so two contexts never share one, #623) because a context's
  display name changes when another kubeconfig declares the same name (#265). The broker
  resolves each request's context name to that ID the same way a connection does, and sends
  the request on under that ID, so a kubeconfig change mid-request cannot reach a cluster that
  took the name since. Navigation and resource slots
  hide it on the other clusters, and the broker refuses its reads and actions there with
  "App is not enabled for this cluster", distinct from a missing-CRD requirement. When the
  contexts cannot be listed, an app page says so and offers a retry rather than calling the
  app not enabled, and so does a resource view where a limited app offers tabs or actions.
  The broker refuses a limited app on a context it cannot resolve with the reason (no
  kubeconfig declares it, or which kubeconfig could not be read), not as not enabled.
- Every read checks the durable inventory and revision, so a disabled, removed or
  replaced installation cannot be invoked through an old registry instance. Calls
  already admitted may finish.
- Streams do not finish: every inventory write ends the streams the new state no
  longer authorizes — a disabled, updated or removed app's — and nothing else. See
  [streams.md](streams.md#ownership).
- Nothing is persisted in browser storage.
- Stored settings are held to the typed `settings` the manifest declares (#542). A
  setting reaches a capability only through a binding argument the capability marks
  settable, checked at install, on save and on every request. A secret-reference
  setting's value never enters the inventory; it holds only a reference, and the value is
  one more entry in srelens's encrypted secrets vault (`secrets.enc`), whose one master
  key is held by the OS keychain or derived from the master password. The registry sees
  the vault only as a `SecretStore`, which follows the
  inventory: whatever the inventory stops referencing is deleted (#543). See
  [Secret settings](manifest.md#secret-settings).

## Quarantine

Every load re-verifies each installed app: its manifest against this host's supported
API versions and rules, and a signed app's stored proof against the publisher delegation
that vouched for it, under the root the host pins ([trust.md](trust.md#installed-apps)).
An app that fails is **quarantined on its own**:

- it loads disabled, with its reason shown in Settings → Apps
- it cannot be re-enabled until it is reinstalled or removed
- every other app keeps working

This covers a rotated or withdrawn signing key, a modified proof or manifest, an
API version the host no longer supports, and a reader bound to a group no
CustomResourceDefinition can declare, such as `apps`. The reason is recomputed on each load and
never written to the inventory. A corrupt file or duplicate app IDs still fail the
whole inventory, because no single entry can be trusted then.

## Hosts

- **Desktop:** full support in both the new and classic designs.
- **Web:** each signed-in user has their own inventory, kept in the server database,
  and every user reads one shared catalog only the server writes
  ([#515](https://github.com/srelens/srelens/issues/515)). Every user's apps are held
  to the operator's extension policy, which the broker applies on every call
  ([#578](https://github.com/srelens/srelens/issues/578)). App streams are not run
  there, so views read on Refresh. See [capabilities.md](capabilities.md#web-host).

## Where it is heading

The plan, its decisions and milestones are tracked in
[#163](https://github.com/srelens/srelens/issues/163) and its child epics:

1. **Foundation:** this specification, a committed JSON Schema, structured validation
   errors and lifecycle gaps ([#516](https://github.com/srelens/srelens/issues/516)).
2. **Declarative UI contributions:** table columns, detail panels, dashboard cards,
   status resolvers and badges, typed settings, commands
   ([#517](https://github.com/srelens/srelens/issues/517)).
3. **Declared, host-enforced actions** replacing the built-in Flux/Argo CD action list
   ([#518](https://github.com/srelens/srelens/issues/518)), alongside third-party
   signing, rotation and revocation
   ([#519](https://github.com/srelens/srelens/issues/519)).
4. **Streaming and providers**, then a supervised, sandboxed **executable SDK** that
   refuses to run where no OS sandbox exists
   ([#520](https://github.com/srelens/srelens/issues/520),
   [#521](https://github.com/srelens/srelens/issues/521)).

Contributions render with host components only; no iframe or third-party renderer code
is planned.
