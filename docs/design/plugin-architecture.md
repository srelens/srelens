# srelens extension architecture

Status: native-only direction, updated 2026-09-14. This supersedes the earlier
proposal for a Freelens/OpenLens compatibility runtime. The delivery plan and its
decisions are tracked in [#163](https://github.com/srelens/srelens/issues/163).

## Supported contract

Extensions target the versioned srelens manifest and capability broker in
`crates/plugin-host`. The host declares a set of supported extension API versions and
serves each manifest under the highest one its `srelensApiVersion` range matches; the
versioning and compatibility rules are in the
[extension API specification](../extensions/specification.md). The desktop application renders declarative pages,
dashboards, resource details, navigation groups and actions with host components
in both classic and new designs. No third-party JavaScript executes in the host.
Native Flux and Argo CD manifests are the reference integrations.

The application installs declarative manifests from the catalog, where official
releases are signature-verified, or as local manifests after an explicit
permission review. There is no developer mode. IDs under `org.srelens.` are
reserved for signed srelens releases. See [Extensions](../EXTENSIONS.md) for
installation, examples, supported fields and current limitations. The executable
SDK, third-party publisher signing, key rotation and revocation described below
are future work, not shipped capabilities.

## Data and lifecycle

The backend owns installation, permissions, enabled state, revisions and settings.
It writes the inventory atomically under a cross-process lock. No extension
preference is persisted in browser storage. Every read validates the current
installation and revision; disable, removal and update revoke stale readers.
Already admitted reads may finish. Every load also re-verifies each app's
manifest and stored signature proof. An app that fails is quarantined and
disabled on its own, without affecting the others.

Views pin cluster identity in their routes and broker calls. Contributions use
host theme, density, navigation and searchable namespace controls. Failed reads
retain their error and retry action; they never become empty-cluster claims.

The application exposes the `extensions.*` capabilities through the shared
registry; install, configuration and GitOps actions are consent-gated.
The declarative broker inherits core capability annotations and binds fixed
resource selectors. Callers cannot override those selectors or obtain ambient
network, filesystem, process, kubeconfig or credential access.

The developer broker can register `plugin/<id>/<operation>` capabilities for MCP.
The app uses the revision-checked read facade; dynamic MCP discovery remains
separate lifecycle work. Multi-user web installations remain disabled until
per-user inventory and lifecycle isolation are implemented.

## Native platform roadmap

1. Keep the declarative manifest and host-rendered UI as the default authoring
   model. Prove additions through the Flux and Argo CD reference extensions.
2. Add resource workflows through explicit broker operations with inherited
   mutation annotations and confirmation; do not bypass host consent.
3. Design a native SDK for supervised, sandboxed JSON-RPC sidecars. Require
   quotas, cancellation and teardown; refuse executable extensions on unsupported
   sandbox backends. No renderer bridge is planned: contributions use host
   components.
4. Extend signed distribution from official releases to third-party publishers,
   with key rotation, update verification, permission-diff consent and
   revocation. Unsigned local manifests stay outside reserved namespaces.

There is no Lens API shim, Node compatibility host or third-party package runtime.
Extensions must target the srelens contract. Retired archive inventory entries are
excluded on read and removed on the next successful inventory save, preserving
native installations and settings.
