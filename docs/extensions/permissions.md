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
- A grant covers whatever the manifest binds to that capability, so the review also
  shows the bindings once the host has accepted the manifest: for each
  `k8s.listCustomResource` reader its group, version, kind, plural, scope and printer
  columns (with their JSON paths); for each `k8s.listEvents` reader the API groups its
  dashboards show; for anything else its fixed arguments. **View manifest** opens the
  full manifest before installing, whether it came from the Catalog or was pasted.
- Installing a new version of an installed app shows its permissions again, with
  what access the update changes
  ([#554](https://github.com/srelens/srelens/issues/554)). The host compares the
  incoming manifest's access with the installed revision's: the grants, what each
  reader binds, the settings it keeps secrets for, and each action. The review in
  Settings → Apps lists what is added and removed before what is unchanged, and the
  consent prompt for an install over MCP names the added and removed access. The
  update must name the installed revision it was reviewed against, and is refused if
  the app has changed since. The comparison is a review aid and refuses nothing: an
  update that widens access installs once it is approved. The application never
  silently replaces a manifest or expands its grants.
- A rollback gets no such comparison. Its review in Settings → Apps compares
  capability IDs only: when they differ from the grants held now, it lists those the
  kept version requests and those it no longer uses
  ([threat-model.md](threat-model.md#malicious-app)).

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
- A badge without a join ([#541](https://github.com/srelens/srelens/issues/541)) is the
  one place the host reads a built-in kind on an app's behalf, and it needs no grant:
  when a built-in table the user opened shows that kind, the host lists the same kind
  in the same namespace, with the user's credentials, and keeps only each object's
  name, namespace, UID, labels, annotations and owner references, plus the kind's own `apiVersion` and `kind`. The badge's rules
  may address only `.metadata`, so no spec or status is read for an app. Secrets are
  refused outright, because their annotation values are redacted on every ungated
  read. What the app gets is its own word, drawn by the host; no app code sees the
  metadata.
- The app receives no kubeconfig or token.
- Reads remain subject to the selected cluster's RBAC. RBAC and discovery failures are
  shown as errors, never as empty results.

## What an app may write

Only through a declared action ([manifest.md](manifest.md#declared-actions)), and only
one of the four host action primitives, each a separate permission the user grants:
`k8s.annotate`, `k8s.setFields`, `k8s.setStatusCondition` and `k8s.mergePatch`.

- A reader grant buys no write, and an action grant buys no read.
- An action reaches only the kind of a reader binding in the same manifest, because
  the host copies that binding's group, version, plural, kind and scope into the
  request. The app cannot name a kind, and the only inputs are the object the operator
  reviewed: `context`, `namespace`, `name`, `uid` and `resourceVersion`.
- What is written is fixed in the manifest and shown at install time. The only values
  the host substitutes are `$now` and `$uuid`.
- The host refuses writes to `metadata.finalizers`, `ownerReferences`, `managedFields`,
  the pin fields and `status` through `k8s.mergePatch`, along with a Secret's values
  and RBAC kinds.
- Every write re-reads the object, refuses one that has changed or is being deleted,
  and is still subject to the cluster's RBAC.

## Secrets

`extension.secretStore` lets the host keep an app's `secret-reference` settings in the
desktop's encrypted secrets vault, whose key the OS keychain holds or the master password
derives ([#543](https://github.com/srelens/srelens/issues/543)).

- A manifest lists it exactly when it declares a `secret-reference` setting, and never
  binds it. It is granted at install like any other permission.
- The review names it with the secret settings it covers and the host's metadata for
  it: sensitive, `medium` impact, and its confirmation wording. An update that keeps
  another secret shows as changed access.
- Without the grant, a secret cannot be set. A secret is write-only: nothing returns
  it to the app, the UI, MCP or an export, and the host injects one only into an
  argument a host capability declares for it. None does yet (#568 will).
- Removing the app, or an update or rollback that drops the setting, deletes it. Reset
  in Settings → Apps clears the app's secrets before it resets the other settings.

See [Secret settings](manifest.md#secret-settings).

## Consent

Annotations come from the host capability and cannot be weakened by a binding —
the gate, the impact level and the confirmation wording alike. See
[Host-defined capability metadata](capabilities.md#host-defined-capability-metadata)
for what each field means and for the rule (`Annotations::for_binding`) that
raises a binding's row and never lowers it.

The app-level operations follow the normal MCP consent gate:

- `extensions.configure` (install, enable, remove, settings, rollback, clusters) is mutating,
  `medium` impact. A rollback takes the grants explicitly, like an install, because it
  grants the restored version's permissions again.
- `extension.secretStore` (set or clear an app's secret) is mutating, sensitive and
  `medium` impact. Because it is sensitive, its audit record keeps the argument names
  and blanks every value, and the desktop's consent prompt never carries the secret to
  the window.
- `extensions.action` (declared app actions) is mutating and `high` impact, because it
  can dispatch `k8s.mergePatch`, including an Argo CD sync. The per-action level
  is lower for most actions and travels with the resource; in the UI every action opens
  a review naming the cluster and resource first.

See [capabilities.md](capabilities.md) for the full list.

## Revocation of access

`extensions.read`, `extensions.resource` and `extensions.action` reload the inventory
on every request and check the app's enabled state, revision and grants. Disabling,
removing, updating or quarantining an app stops its calls immediately; calls already
admitted may finish.

## Web host

Each user of the multi-user web host grants permissions to their own apps; one user's
grants never reach another's. Declared actions run only through `extensions.action`,
after the host confirmation, and the host action primitives stay refused when called
directly. See [capabilities.md](capabilities.md#web-host).
