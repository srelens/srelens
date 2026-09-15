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

It registers operations under `plugin/<app-id>/<operation>` in the shared capability
registry, where MCP discovers and gates them through its normal request path.

Registration is all-or-nothing. Unregistering removes operations from the mutable
registry and revokes their handlers in older snapshots; calls already admitted may
finish. A host must rebuild its MCP snapshot after a lifecycle change to refresh
discovery. An older snapshot may still list a revoked tool but cannot execute it, so
live tool-list updates are not advertised yet.

## App lifecycle

Both desktop designs manage apps through **Settings → Apps**, from the catalog or a
pasted local manifest. The backend owns installation, grants, enable/disable,
updates, removal and per-app JSON settings. Installation requires an explicit review
of the requested permissions. There is no developer mode.

Installation and enablement are app-wide, not per kubeconfig context. Enabled pages
are available on every cluster; each page checks the APIs it needs when it opens (see
[ui-contributions.md](ui-contributions.md#requirement-checks)).

## The inventory

The inventory lives next to the desktop settings file: the settings path with its
extension replaced by `extensions.json`, so `settings.extensions.json`.

- Saves use a private temporary file, sync and atomic replacement under a
  cross-process lock.
- Updating an ID preserves its settings and assigns a new revision. The app keeps up to
  the last three versions it replaced, fewer when they would take the inventory past
  1 MiB; restoring one grants its permissions again after
  review, keeps settings and assigns a new revision
  (see [migration.md](migration.md#rolling-back)).
- Each installed version records its source: `catalog` when its exact bytes are a
  release in the cached catalog, otherwise `local`. The host decides this, not the caller.
- An app may be limited to chosen kubeconfig contexts, kept by stable ID because a context's
  display name changes when another kubeconfig declares the same name (#265). The broker
  resolves each request's context name to that ID the same way a connection does. Navigation and resource slots
  hide it on the other clusters, and the broker refuses its reads and actions there with
  "App is not enabled for this cluster", distinct from a missing-CRD requirement.
- Every read checks the durable inventory and revision, so a disabled, removed or
  replaced installation cannot be invoked through an old registry instance. Calls
  already admitted may finish.
- Nothing is persisted in browser storage.
- Stored settings are JSON data; this declarative version does not interpolate them
  into capability arguments.

## Quarantine

Every load re-verifies each installed app: its manifest against this host's supported
API versions and rules, and a signed app's stored proof against the trusted publisher
table. An app that fails is **quarantined on its own**:

- it loads disabled, with its reason shown in Settings → Apps
- it cannot be re-enabled until it is reinstalled or removed
- every other app keeps working

This covers a rotated or withdrawn signing key, a modified proof or manifest, and an
API version the host no longer supports. The reason is recomputed on each load and
never written to the inventory. A corrupt file or duplicate app IDs still fail the
whole inventory, because no single entry can be trusted then.

## Hosts

- **Desktop:** full support in both the new and classic designs.
- **Web:** every `extensions.*` capability is refused until app state is kept per
  user ([#515](https://github.com/srelens/srelens/issues/515)). See
  [capabilities.md](capabilities.md#web-host).

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
