# Migration

Upgrading, downgrading, and moving an app between API versions.

## Moving an app to a new API version

The rules are in [specification.md](specification.md#versioning). In practice:

- A manifest keeps working on every host whose supported set still matches its range.
  A `^0.1` app keeps installing on a host that also supports 0.2.
- To use a field or contribution introduced in a newer API version, require that
  version (for example `^0.2`) and publish a new app version. Hosts that do not support
  it report the version the app needs.
- When a host retires the API version an installed app requires, the app is
  quarantined, not removed. Update it to a release that requires a supported version.

## Updating an installed app

Install the new release from the catalog, or paste the new manifest, to replace the
installed version. The replacement goes through permission review again, keeps the
app's settings and assigns a new revision; open views refresh against it. The
application never silently replaces a manifest or expands its grants.

For example, the updated Flux release requests an additional event-read grant for its
dashboard, which the review shows.

## Rolling back

Each update keeps the version it replaced, up to the last three per app, with the
grants, source and install time it had. The oldest are dropped sooner when keeping them
would take the inventory past its 1 MiB limit, so an app with very large manifests keeps
fewer. **Settings → Apps → Details → Previous
versions** restores one:

- A version whose permissions differ from what is granted now goes through permission
  review again. One with the same permissions asks for confirmation.
- The restored version is checked as installing it now would be: against its publisher
  signature, if it had one, and against this host's rules.
- Settings are kept. The versions after the restored one are discarded, so going forward
  again means installing the newer release.
- The app gets a new revision, so open views refresh against the restored version.

## Reserved IDs

Since #528, IDs under `org.srelens.` install only with the srelens signature:

- Pasting an example manifest unchanged is refused. Give a local copy its own ID, such
  as `org.example.argocd`.
- Unsigned `org.srelens.` apps installed before the rule keep working, still labelled
  unsigned. Reinstall them from the catalog to get the signed release.

## Developer mode

Developer mode was removed in #511. An inventory saved with developer mode off keeps
its apps disabled; the field is dropped on the next save.

## Downgrading

Inventory changes are one-way. A host older than #511 cannot read an inventory written
by #511 or later, because `developerMode` was removed and `signatureProof` added, and
older hosts reject both. Since #534 every installed app also records `source`,
`installedAt` and `history`, so an inventory saved before #534 is not readable either;
move it aside and reinstall the apps. After such a downgrade, Settings → Apps reports the inventory as
unreadable. Upgrade again, or move `settings.extensions.json` aside to start with no
apps.

## The retired compatibility prototype

Freelens/OpenLens archive installations from the earlier prototype are excluded when
the inventory is read; native apps, permissions, revisions and settings stay intact. The
next successful inventory change removes the retired entries from the saved file.
Archive installation is no longer available: install Flux or Argo CD from **Settings →
Apps → Catalog** instead.
