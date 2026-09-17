# Permissions

An app never inherits srelens's access. It declares the host capabilities it binds,
the user grants them at installation, and every call still runs under the selected
cluster's RBAC.

## Declaring and granting

- `permissions` lists the exact host capability IDs the manifest's bindings target,
  no more and no fewer (see [manifest.md](manifest.md#capability-bindings)).
- A declaration is not an authorization. The host supplies grants separately:
  installation shows the requested permissions for review, and **Install and grant
  permissions** grants that list.
- Installing a new version of an installed app shows its permissions again. The
  application never silently replaces a manifest or expands its grants. A
  permission diff on update is planned
  ([#554](https://github.com/srelens/srelens/issues/554)).

## What an app may read

The desktop app accepts `k8s.listCustomResource` bindings with a fixed API group,
version, plural, kind and scope, plus explicitly granted `k8s.listEvents` readers.

- Only `context` and `namespace` are forwarded from the host view.
- Core-group resources, caller-supplied resource selectors, executable entry points
  and operations that need consent are rejected.
- A reader reaches custom resources only. A group with no dot, such as `apps` or
  `batch`, is refused at install and quarantines an installed app. Each read,
  inspection and action first confirms that a CustomResourceDefinition named
  `{plural}.{group}` serves the bound version on the cluster, so a dotted built-in
  group such as `networking.k8s.io`, or an aggregated API, is refused there.
- The app receives no kubeconfig or token.
- Reads remain subject to the selected cluster's RBAC. RBAC and discovery failures are
  shown as errors, never as empty results.

## Consent

Annotations come from the host capability and cannot be weakened by a binding. The
app-level operations follow the normal MCP consent gate:

- `extensions.configure` (install, enable, remove, settings, rollback, clusters) is mutating. A
  rollback takes the grants explicitly, like an install, because it grants the restored
  version's permissions again.
- `extensions.action` (host GitOps actions) is mutating, and in the UI every action
  opens a review naming the cluster and resource first.

See [capabilities.md](capabilities.md) for the full list.

## Revocation of access

`extensions.read`, `extensions.resource` and `extensions.action` reload the inventory
on every request and check the app's enabled state, revision and grants. Disabling,
removing, updating or quarantining an app stops its calls immediately; calls already
admitted may finish.

## Web host

Apps are not available on the multi-user web host yet. See
[capabilities.md](capabilities.md#web-host).
