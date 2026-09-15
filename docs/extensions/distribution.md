# Distribution

How apps reach users: the catalog, signed official releases, and local installation.

## The catalog

On the desktop, open **Settings → Apps → Catalog**. The **Apps** tab lists installed
apps; a collapsed **Install a local manifest** section holds the JSON installer. The
**Catalog** tab loads on first opening and keeps its search and list state when you
switch tabs.

The catalog comes from [srelens/extensions](https://github.com/srelens/extensions).
Each entry points to its own repository and a versioned GitHub release asset. The
initial entries are [Flux](https://github.com/srelens/extension-flux) and
[Argo CD](https://github.com/srelens/extension-argocd). Search by name, ID or
description.

- The backend caches validated metadata for 24 hours in `*.extensions.catalog.json`,
  next to the inventory. **Refresh catalog** checks immediately. A failed refresh keeps
  the cache and shows the failure and the original timestamp.
- Browsing does not connect clusters or install anything.
- Catalog metadata is additive: hosts ignore fields they do not recognize (see
  [Unknown fields](specification.md#unknown-fields)).
- Releases whose `srelensApiVersion` matches none of the host's supported API versions
  stay visible but cannot be installed. The catalog shows the host's versions.
- Preview labels come from catalog metadata. `testedHost.revision` records test
  provenance, not an exact-build restriction.
- Flux and Argo CD use bundled project logos; other apps get an initials mark. Logos
  identify an integration and do not indicate trust or signing.

## Reviewing an installation

**Review installation** downloads a size-bounded manifest over HTTPS, checks its
SHA-256 against the selected catalog release, verifies its ID, version and API range
against the entry, and validates the desktop app's capability rules. The exact verified
bytes then go through the permission review and install action.

A catalog change invalidates the selected checksum and requires a new review. Replacing
an installed ID is explicit and keeps its settings. There are no automatic updates or
downgrade decisions ([#563](https://github.com/srelens/srelens/issues/563)).

## Signed official releases

Official Flux and Argo CD releases carry a detached Ed25519 signature
(`manifest.json.sig`) over the exact manifest bytes. The host pins a **trusted-publisher
table** in `crates/registry/src/extensions/signing.rs`, holding for each publisher:

- the public key
- the app ID namespace reserved for it (`org.srelens.` for srelens)
- the only repository each of its apps may be released from

Catalog metadata cannot supply a trusted key.

- A catalog entry that names a reserved ID *or* a trusted publisher's repository must
  be signed. Missing signatures, modified bytes and repository substitution are rejected
  before review.
- Installation re-verifies the proof and stores it, and every inventory load checks it
  against the installed manifest ([architecture.md](architecture.md#quarantine)).
- Permission review is still required. A checksum alone is not a publisher signature.

IDs in a reserved namespace install **only** with that publisher's signature. A pasted
manifest cannot use an `org.srelens.` ID, and cannot replace a signed installation.
Apps with IDs outside reserved namespaces install unsigned and are labelled **Unsigned
local**.

## Releasing an official app

The release workflows in the app repositories require `APP_SIGNING_PRIVATE_KEY`
(PKCS#8 Ed25519 PEM) in GitHub Actions secrets, check it against `signing-public.pem`,
and publish a 64-byte binary signature. The private key must never be committed. The
current public key is also stored as raw 32 bytes in
`crates/registry/src/extensions/srelens-apps.pub`.

Key rotation needs a host update that trusts the new key before new release signatures
are published ([#560](https://github.com/srelens/srelens/issues/560)). A host that stops
trusting a stored signature quarantines only that app.

## Local installation

Paste a manifest under **Settings → Apps → Install a local manifest**. It goes through
the same validation and permission review, with an ID outside reserved namespaces. See
[introduction.md](introduction.md#try-an-app) for a walkthrough.
