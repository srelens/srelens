# Distribution

How apps reach users: the signed catalog, signed releases, and local installation. An
app is released as one manifest file, or as a `.srelens-extension` package that carries its
logo and files as well ([packages.md](packages.md)).

## The catalog

On the desktop, open **Settings → Apps → Catalog**. The **Apps** tab lists installed
apps; a collapsed **Install a local manifest** section holds the JSON installer. The
**Catalog** tab loads on first opening and keeps its search and list state when you
switch tabs.

The catalog comes from [srelens/extensions](https://github.com/srelens/extensions), signed
([Signed releases and publishers](#signed-releases-and-publishers)). Each entry points to
its own repository and a versioned GitHub release asset. The
initial entries are [Flux](https://github.com/srelens/extension-flux) and
[Argo CD](https://github.com/srelens/extension-argocd). Search by name, ID or
description.

- The backend caches the verified catalog for 24 hours in `*.extensions.catalog.json`,
  next to the inventory, and verifies its signature again whenever it reads it. **Refresh
  catalog** checks immediately, and so does opening a catalog that has expired. A failed
  or refused refresh keeps the cache and shows the failure and the original timestamp. An
  expired catalog is shown, but not installed from.
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
  package, at most 512 MiB. Its SHA-256 must match `release.package.sha256`, it is verified
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
an installed ID is explicit and keeps the settings the new release still declares. There are no automatic updates or
downgrade decisions ([#563](https://github.com/srelens/srelens/issues/563)).

## Signed releases and publishers

Since [#559](https://github.com/srelens/srelens/issues/559) the catalog is signed, and it
says which publisher may sign which apps. The formats, the key ceremony and the runbooks
are in [trust.md](trust.md); in short:

- The host pins a root, and trusts a catalog only when the root's catalog role signed it.
  A catalog that is unsigned, wrongly signed, expired, or older than the last one the host
  verified is refused, and the last verified catalog stays in use, marked stale, with the
  reason.
- The catalog delegates app ID namespaces to publishers: `org.srelens` to srelens, and
  others to third parties. A release in a delegated namespace carries a detached Ed25519
  signature (`manifest.json.sig`) over the exact manifest bytes, made by one of that
  publisher's keys, and is refused before review without it. A publisher's key cannot
  sign an ID outside its namespaces.
- A release signature names its key: `{"keyid": "<key ID>", "sig": "<base64>"}`. The
  releases published before #559 carry the 64 signature bytes alone, and still verify.
- A repository URL grants nothing. Neither a repository nor an ID prefix makes an entry
  signed; only a delegated namespace does.
- Installation re-verifies the signature and stores it, with the delegation that vouched
  for it, and every inventory load checks both against the installed manifest with no
  catalog needed ([architecture.md](architecture.md#quarantine)).
- Permission review is still required. A checksum alone is not a publisher signature.

IDs in a delegated namespace install **only** with that publisher's signature. A pasted
manifest cannot use an `org.srelens.` ID, or another publisher's, and cannot replace a
signed installation. Signed apps are labelled **Signed by** the publisher the host
verified; apps with IDs outside delegated namespaces install unsigned and are labelled
**Unsigned local**.

## Releasing a signed app

The Flux and Argo CD release workflows sign with `APP_SIGNING_PRIVATE_KEY` (PKCS#8
Ed25519 PEM) in GitHub Actions secrets, check it against `signing-public.pem`, and publish
`manifest.json.sig`. The private key must never be committed. Since #559 the signature
file names its key; [Signing a release](trust.md#signing-a-release) has the change to
`scripts/sign.mjs`, and [Adding a publisher](trust.md#adding-a-publisher) covers a third
party's first release.

Releases are listed in `catalog.signed.json`. Hosts released before #559 read the unsigned
`catalog.json`, which stays frozen at the releases they can verify
([Publishing the signed catalog](trust.md#publishing-the-signed-catalog)).

Rotating a key without losing installed apps is [#560](https://github.com/srelens/srelens/issues/560).
A host that stops trusting a stored signature quarantines only that app.

## Local installation

Paste a manifest under **Settings → Apps → Install a local manifest or package**, or choose
a `.srelens-extension` file there on the desktop. Either goes through the same validation
and permission review; a package is verified whole first, every file against its digest
list ([packages.md](packages.md#installing)). Unsigned, its ID must be outside reserved
namespaces. See [introduction.md](introduction.md#try-an-app) for a walkthrough.
