# Packages

A `.srelens-extension` package carries an app's manifest together with its logo and other
files, under one publisher signature ([#562](https://github.com/srelens/srelens/issues/562)).
A single-file release, one `manifest.json` with its detached `manifest.json.sig`, is still
valid and still installs everywhere ([distribution.md](distribution.md)). A package adds what
one JSON file cannot hold: the app's logo today, and per-platform binaries once executable
apps exist ([#521](https://github.com/srelens/srelens/issues/521)).

The reader, installer and packer are in `crates/registry/src/extensions/package.rs`.

## Layout

A package is a gzip-compressed POSIX tar archive:

```text
example.srelens-extension
├── extension.json        the manifest, exactly as a single-file release carries it
├── README.md
├── LICENSE
├── icons/                .svg and .png images; icons/icon.svg or icons/icon.png is the logo
├── schemas/              .json files
├── bin/<platform>/       darwin-arm64, darwin-amd64, linux-amd64, linux-arm64, windows-amd64
├── digests.json          what the package holds
└── digests.json.sig      optional: the publisher's signature over digests.json
```

Only `extension.json` and `digests.json` are required. Any other name at the top level,
a file directly under `bin/`, or a platform not in the list is refused.

## The digest list

`digests.json` names every other file in the package, with its size and SHA-256, and binds
them to one app and version:

```json
{
  "format": "srelens-extension-package",
  "formatVersion": 1,
  "id": "org.example.packaged",
  "version": "1.0.0",
  "files": [
    { "path": "LICENSE", "size": 68, "sha256": "…" },
    { "path": "README.md", "size": 171, "sha256": "…" },
    { "path": "extension.json", "size": 799, "sha256": "…" },
    { "path": "icons/icon.svg", "size": 211, "sha256": "…" },
    { "path": "schemas/settings.schema.json", "size": 84, "sha256": "…" }
  ]
}
```

That is the test fixture in `crates/registry/tests/fixtures/packages/example`, with its
digests shortened.

It is read only in this exact form:

- No other fields, and no field twice. A repeated key is refused, not read last-wins.
- `format` must be `srelens-extension-package`, so a manifest or anything else signed with
  the same key cannot pass for a digest list, and a digest list cannot pass for a manifest.
- `formatVersion` must be 1. A later format is refused with the version this host reads.
- Each path appears once, in byte order, and follows the path rules below. `digests.json`
  and `digests.json.sig` are never listed. `extension.json` always is.
- Each `sha256` is 64 lowercase hex digits.
- `id` and `version` must equal the manifest's own `id` and `version`.

## The signature

`digests.json.sig` is 64 raw bytes of Ed25519 over the exact bytes of `digests.json`. This
is the scheme a single-file release uses, applied to the list instead of the manifest. The
host looks the key up the same way, by the app ID (`verify_for` in
`crates/registry/src/extensions/signing.rs`). An `org.srelens.` package must be signed by
srelens. A signature on any other app ID is refused, because no other publisher is trusted
yet ([#559](https://github.com/srelens/srelens/issues/559)). Because the list covers every
file, a changed, missing or extra file fails verification just as a changed manifest does.

An unsigned package installs as an unsigned local app. The digest list still checks the
archive's integrity and completeness, but it does not say who made the package.

## What the host refuses

A package is taken whole or not at all:

| Rule | Limit |
|---|---|
| Package file, compressed | 16 MiB |
| Every file together, uncompressed | 64 MiB |
| Entries (files and directories) | 256 |
| `digests.json` | 64 KiB |
| `extension.json` | 256 KiB, as any manifest |
| An icon | 256 KiB |
| `README.md`, `LICENSE`, a schema | 1 MiB |
| A path | 160 bytes, 6 segments, 64 bytes each |

- **Entries.** Only regular files and directories. Symbolic links, hard links, devices,
  pipes, sparse files, PAX extended headers and GNU long names are refused. Headers are
  read as they are written, so no extension header can rename the next entry, and none is
  held in memory before it is checked.
- **Paths.** Relative, `/`-separated, ASCII letters, digits, `.`, `_` and `-` only. No
  segment may be empty, `.` or `..`, start or end with a dot, or be a Windows device name
  (`nul`, `COM1`, …). No two paths may differ only in case, and no file may sit where
  another path needs a directory. This way a path names the same file on every platform
  the host runs on.
- **Nothing after the archive.** Only zero padding may follow the end of the tar archive,
  and nothing may follow the gzip stream. Otherwise another reader could unpack entries
  this one never checked.
- **Logos.** `icons/icon.svg` must be UTF-8 holding an `<svg>`, and `icons/icon.png` must
  start with the PNG signature.
- **Binaries.** This host runs declarative apps only, so it refuses to install a package
  that carries anything under `bin/`. The format already has a place for binaries so that
  executable apps ([#521](https://github.com/srelens/srelens/issues/521)) do not need a new
  one.

## Installing

A package reaches the host in one of two ways:

- **From a file.** Settings → Apps → **Install a local manifest or package** → **Local app
  package**. The desktop app reads the file and sends it as base64. `extensions.packageManifest`
  verifies it and returns the manifest, its signature and the package's review: its files,
  its digest list and its logo. The review then works like a manifest's review:
  `extensions.validate` gets the manifest, signature and digest list together, and
  `extensions.configure` `installPackage` sends the same bytes, which are verified again.
- **From the catalog.** A catalog release may name a package beside its manifest
  (`release.package`, see below). **Review installation** then reviews the package, and
  `installCatalogPackage` names the release and the exact package that was reviewed. The
  host downloads the package again, before it takes the inventory's lock.

Installing verifies the package whole, checks its manifest as installing a single-file
manifest would, and only then unpacks it. The files go into a directory that belongs to the
app, beside the inventory:

```text
settings.extensions.packages/<app ID, lowercase>/<SHA-256 of digests.json>/
```

The directory is private: `0700` directories and `0600` files on Unix. The package is read
a second time into a staging directory beside the target, with every file synced. Then one
durable rename moves it into place (`publish_dir` in `crates/registry/src/durable.rs`,
which syncs the parent directories as the inventory's saves do, [#611]). The inventory save
that names the new version is what installs it, and that save is an atomic replace of its
own. So a crash leaves either the old version installed or the new one, never a
half-unpacked app. Versions the inventory no longer names, current or kept for rollback,
are removed after each change.

A signed package's proof is kept in the inventory: the manifest, the signature and the
digest list. Every load checks the signature over the list, that the list names the
manifest, and that the list is the one the installed directory was unpacked with. An app
that fails is quarantined on its own ([architecture.md](architecture.md#quarantine)).

The web host keeps no files for its apps, so it refuses package installs. It installs a
catalog release from its `manifestUrl` instead.

## Logos

An app's logo comes from its installed package, and from nowhere else. The host reads
`icons/icon.svg` or `icons/icon.png` for `extensions.list`, checks the file against the
digest list it was unpacked with, and sends it as a `data:` URL. The UI draws that URL as an
image, never as markup, so an SVG's scripts do not run. Any other value is drawn as the
app's initials. A file changed on disk since the install is not shown, and neither is the
logo of a quarantined app.

**A logo never implies trust.** A package can carry any image, including another project's
logo. Who published an app is shown only by its signature label: **Signed by srelens**,
**Unsigned local** or **Signature not verified**. Nothing is chosen by app ID. Official apps
get no bundled logo either: Flux and Argo CD show their initials until their releases ship
as packages.

## In the catalog

A catalog release may list its package:

```json
"release": {
  "version": "1.0.0",
  "manifestUrl": "https://github.com/example/app/releases/download/v1.0.0/manifest.json",
  "sha256": "<SHA-256 of manifest.json>",
  "srelensApiVersion": "^0.4",
  "prerelease": false,
  "package": {
    "url": "https://github.com/example/app/releases/download/v1.0.0/app.srelens-extension",
    "sha256": "<SHA-256 of the package file>"
  }
}
```

- The field is additive. A host that predates packages ignores it and installs
  `manifestUrl`, so a release with a package keeps publishing its single-file manifest.
- `package.url` must be a GitHub release asset whose name ends in `.srelens-extension`.
  For an official app, it must also be in its pinned repository's release for that
  version, beside `manifest.json`.
- The package's `extension.json` must be the release's manifest, byte for byte: its SHA-256
  is the release's `sha256`. The release's ID, version and API range checks apply to it
  unchanged.
- An official app's package must be signed. Any other app's package must not be.

## Making a package

Lay the package out in a directory, then:

```sh
cargo run -p srelens-registry --example pack-extension -- digests <dir>
```

This writes `<dir>/digests.json`. To sign the package, sign that file with the publisher
key before packing, just as a single-file release signs `manifest.json`. The result must be
64 raw bytes in `<dir>/digests.json.sig`.

```sh
cargo run -p srelens-registry --example pack-extension -- pack <dir> app.srelens-extension
```

`pack` refuses a directory whose files no longer match its `digests.json`, and the same
checks the host makes refuse anything outside the layout, such as a `.DS_Store`. Entries are
plain ustar files in byte order with no owner or time, so the same directory always packs to
the same archive.

[#611]: https://github.com/srelens/srelens/issues/611
