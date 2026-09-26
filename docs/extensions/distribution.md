# Distribution

How apps reach users: the catalog, signed official releases, and local installation. An
app is released as one manifest file, or as a `.srelens-extension` package that carries its
logo and files as well ([packages.md](packages.md)).

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
- An installed app's logo comes from its package; every other app, and every catalog
  entry, gets an initials mark. Nothing is chosen by app ID, and a logo never indicates
  trust or signing ([packages.md](packages.md#logos)).
- A release may also be published as a package (`release.package`). A host that installs
  packages reviews that instead; a host that predates them, and the web host, install
  `manifestUrl` ([packages.md](packages.md#in-the-catalog)).

## Reviewing an installation

**Review installation** downloads the release over HTTPS and checks it before the
permission review. What it downloads depends on the release and the host:

- **A single-file release**, or any release on a host that keeps no app files (the web
  host): a size-bounded manifest. Its SHA-256 must match the selected catalog release,
  its ID, version and API range must match the entry, and it must pass the desktop app's
  capability rules. The exact verified bytes then go through the permission review, and
  the install action sends those bytes.
- **A release with a package** (`release.package`) on a host that installs packages: the
  package, at most 16 MiB. Its SHA-256 must match `release.package.sha256`, it is verified
  whole ([packages.md](packages.md#what-the-host-refuses)), and its `extension.json` must be
  the release's manifest: its SHA-256 is the release's `sha256`. The ID, version, API range
  and capability checks above then apply to that manifest. The install action sends no
  bytes. It names the release and the package checksum that was reviewed, and the host
  downloads the package again and verifies it the same way. It refuses the install if the
  catalog now lists another package ([packages.md](packages.md#installing)).

Either way, the review lists what each permission is bound to and opens the full manifest
on request, so a catalog app can be read before it is installed (see
[permissions.md](permissions.md#declaring-and-granting)).

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
- Repository URLs are compared case-insensitively, because GitHub resolves owner and
  repository names that way: `https://github.com/SRELENS/extension-argocd` is the
  trusted repository and needs the same signature. A lookalike owner such as `srelensx`
  is a different repository, and its apps are ordinary unsigned third-party apps. The
  release asset URL itself must still equal the pinned repository's
  `v<version>/manifest.json`.
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

Paste a manifest under **Settings → Apps → Install a local manifest or package**, or choose
a `.srelens-extension` file there on the desktop. Either goes through the same validation
and permission review; a package is verified whole first, every file against its digest
list ([packages.md](packages.md#installing)). Unsigned, its ID must be outside reserved
namespaces. See [introduction.md](introduction.md#try-an-app) for a walkthrough.
