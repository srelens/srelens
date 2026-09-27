# Trust: the signed catalog and publisher keys

How a host decides whom to trust with an app ([#559](https://github.com/srelens/srelens/issues/559)),
the formats it reads, and the runbooks for the people who hold the keys: the key
ceremony, publishing the catalog, adding a publisher, and signing a release. Why the design is
TUF-style rather than Sigstore keyless is recorded in the
[architecture decision](../design/plugin-architecture.md#decision-a-tuf-style-signed-catalog-not-sigstore-keyless).

## The chain

Each link is signed by the one above it. Nothing in the host's code names a key, a
publisher or a namespace: they all come from documents that chain to the root the build
pins.

1. **Root.** The build pins a root document, `crates/registry/src/extensions/trust/root.json`.
   It lists the keys of two roles, each with a signature threshold:
   - `root` vouches for the root document itself. Its keys are kept offline.
   - `catalog` signs the catalog and the publisher delegations in it. Its key signs in
     the [srelens/extensions](https://github.com/srelens/extensions) CI.
2. **Catalog.** `catalog.signed.json` in srelens/extensions is signed by the catalog role,
   and carries a `version` and an `expires` time.
3. **Publishers.** The catalog delegates app ID namespaces to publishers. Each delegation
   is its own document signed by the catalog role, naming the publisher, its keys, and the
   namespaces those keys may sign. `org.srelens` is delegated to the srelens publisher.
4. **Releases.** A publisher's key signs the exact bytes of a release's `manifest.json`,
   and the signature file names the key.

The build also ships the publisher delegations its already-installed apps need
(`trust/publishers.json`): the srelens delegation, holding the key every release before #559
was signed with. Those apps were installed with a bare signature and no delegation, and
this lets the host keep verifying them with no catalog at hand.

## What the host refuses

- A catalog that is unsigned, or not signed by the catalog role's threshold of keys.
- A catalog that has expired. A catalog the host already holds that has since expired is
  fetched again however recently it was fetched, shown as stale when that fails, and never
  installed from (TUF's freeze protection).
- A catalog older than the last one it verified. A catalog with the same version must be
  the same catalog, byte for byte.
- A catalog with a publisher delegation the catalog role did not sign, or with two
  publishers whose namespaces overlap, or who share a key.
- A release in a delegated namespace without that publisher's signature, or with a
  signature naming a key the publisher does not hold.
- An unsigned local app in a delegated namespace. Namespaces are lowercase, and an ID is
  compared without regard to case, so `org.Srelens.flux` is in `org.srelens` too.
- A signature vouched for only by an expired catalog. An expired catalog still reserves
  its namespaces, but vouches for no key: until it is refreshed, an install is verified
  only under the delegations the build ships.

A refused or failed refresh keeps the last verified catalog, marked stale, with the
reason. The cache is verified again whenever it is read, so it is trusted no more than a
download would be.

A build whose pinned documents are still the placeholders (before the key ceremony below)
trusts no catalog and no publisher signature, and says so. It cannot tell which IDs are
reserved either, so it installs and restores nothing.

## Formats

Every signed document is a [DSSE](https://github.com/secure-systems-lab/dsse) envelope.
The signature covers the payload's type and exact bytes (DSSE's pre-authentication
encoding), so nothing is canonicalized before it is checked, and a signature made over one
document type cannot pass as another. Either base64 alphabet is accepted, padded or not.

```json
{
  "payloadType": "application/vnd.srelens.catalog+json",
  "payload": "<base64 of the document's bytes>",
  "signatures": [{ "keyid": "<key ID>", "sig": "<base64 Ed25519 signature>" }]
}
```

- **Keys** are Ed25519, written in TUF's shape:
  `{"keytype": "ed25519", "scheme": "ed25519", "keyval": {"public": "<64 hex>"}}`.
- **Key IDs** are the SHA-256 of the 32 raw public key bytes, in lowercase hex. The host
  computes them and never takes one from a document on trust.
- **Thresholds** count distinct keys. A key's second signature counts for nothing, and so
  does a signature from a key the role does not list.

| Document | Payload type | Signed by | Fields |
|---|---|---|---|
| Root | `application/vnd.srelens.root+json` | its own `root` role | `_type: "root"`, `version`, `keys` (key ID to key), `roles.root` and `roles.catalog` (`keyids`, `threshold`) |
| Publisher | `application/vnd.srelens.publisher+json` | the `catalog` role | `_type: "publisher"`, `version`, `id`, `name`, `keys` (a list), `namespaces` |
| Catalog | `application/vnd.srelens.catalog+json` | the `catalog` role | `_type: "catalog"`, `schemaVersion: 2`, `version`, `expires` (RFC 3339), `publishers` (signed publisher documents), `extensions` (entries as before) |

Unknown fields and roles are ignored, so later documents can add a timestamp role, a
revocation list ([#561](https://github.com/srelens/srelens/issues/561)) or catalog metadata
([#564](https://github.com/srelens/srelens/issues/564)) without breaking released hosts.

Rules a publisher document must meet:

- `id` is one label of lowercase letters, digits and `-`. `name`, shown as "Signed by
  <name>", is 1–64 characters with no control or invisible formatting character.
- 1–16 keys, and 1–16 `namespaces`. A namespace is two or more dot-separated labels of
  lowercase letters, digits and `-`, and covers itself and every ID below it at a label
  boundary: `org.srelens` covers `org.srelens.flux`, not `org.srelensx.flux`.
- Within a catalog, no two publishers share an ID, a key or an overlapping namespace, so
  every app ID has at most one publisher.

A **release signature** (`manifest.json.sig`, published beside `manifest.json`) is an
Ed25519 signature over the manifest's exact bytes, in one of two forms:

- Since #559, it names its key: `{"keyid": "<key ID>", "sig": "<base64>"}`, at most
  512 bytes. The named key must be one of the publisher's.
- Before #559, it was the 64 signature bytes alone. Those are still accepted, and are
  checked against each of the publisher's keys.

The signature's form is all that changed: the signed bytes are the manifest's, as before.

## Installed apps

An install keeps the signed manifest bytes and the signature, as before, plus the
publisher delegation that vouched for the signature (`signatureProof.delegation`). Every
inventory load verifies the delegation under the pinned root, then the signature under
the delegation, with no catalog needed. When the build's shipped delegations already
vouch for a signature, as they do for srelens releases, no delegation is kept, and an
inventory written this way stays readable by hosts from before #559.

The delegations the build ships stand over any other, the catalog's and a kept one alike,
at install and on every load (`Delegations::merged`). Another delegation replaces a
shipped one only when it is the same publisher's at a strictly later version. At the same
version the shipped one stands, and no other publisher takes a namespace the build ships.
Install and every later load use the same rule, so an app is never installed as signed
and then quarantined on its next read. A key that a later delegation withdrew stops
vouching for what it signed once a build shipping that delegation loads the inventory,
and the apps it signed are quarantined. Raise a delegation's `version` whenever its keys
or namespaces change: for a publisher the build ships, a catalog's change at the same
version is ignored.

The host reports who signed each app as `signedBy` (`{id, name}`), recomputed on every
read and never saved. The UI's "Signed by <name>" is that field and nothing else.

## Key ceremony

This creates the root and catalog keys, and the two documents the build pins. Run it once,
on a machine you trust, before the first release that includes #559. Each step uses
[`scripts/extensions/trust.mjs`](../../scripts/extensions/trust.mjs), which needs only
Node.js.

1. **Root keys**, kept offline: generate at least two and require two signatures, so that
   losing or leaking one key is recoverable by rotation
   ([#560](https://github.com/srelens/srelens/issues/560)). Keep each private key on its own
   offline medium.

   ```bash
   node scripts/extensions/trust.mjs keygen root-1.pem
   node scripts/extensions/trust.mjs keygen root-2.pem
   ```

2. **Catalog key**, which will sign in CI:

   ```bash
   node scripts/extensions/trust.mjs keygen catalog.pem
   ```

3. **The root document**, signed by both root keys:

   ```bash
   node scripts/extensions/trust.mjs root --version 1 \
     --root root-1.pem --root root-2.pem --root-threshold 2 \
     --catalog catalog.pem --catalog-threshold 1 \
     --sign root-1.pem --sign root-2.pem \
     > crates/registry/src/extensions/trust/root.json
   ```

   Only public keys enter the document. For `--root` and `--catalog`, a public key file
   (SPKI PEM) works as well as the private one.

4. **The srelens delegation**, with the key the Flux and Argo CD releases are signed with,
   and the file the build ships:

   ```bash
   node scripts/extensions/trust.mjs publisher --id srelens --name srelens \
     --key crates/registry/tests/fixtures/trust/srelens-apps.pub \
     --namespace org.srelens --sign catalog.pem > srelens.json
   node scripts/extensions/trust.mjs bundle srelens.json \
     > crates/registry/src/extensions/trust/publishers.json
   ```

   `srelens-apps.pub` is the published release key's 32 raw bytes (the public half of
   `APP_SIGNING_PRIVATE_KEY`; it matches `signing-public.pem` in the app repositories).

5. **Check and commit.** `the_pinned_root_verifies_and_delegates_org_srelens_to_the_release_key`
   in `crates/registry/src/extensions/trust.rs` checks the pinned documents. Run it and the
   rest of the trust tests in a workspace build, whose features are the ones CI unifies
   (`cargo test -p` can build the crate with others, see AGENTS.md):

   ```bash
   cargo test --workspace --lib -- extensions::trust
   ```

   Commit `trust/root.json` and `trust/publishers.json`. Keep `srelens.json` for the
   catalog (below).

6. **Custody.** Store `catalog.pem` as `CATALOG_SIGNING_PRIVATE_KEY` in the
   srelens/extensions repository's Actions secrets. Move the root keys offline. No private
   key is ever committed.

## Publishing the signed catalog

The catalog lives in [srelens/extensions](https://github.com/srelens/extensions). This is
the change that repository needs; this repository does not make it.

- **A new file.** Publish `catalog.signed.json` beside `catalog.json`. Hosts before #559
  read `catalog.json` and cannot read the signed one, so leave `catalog.json` in place and
  stop changing it. It must go on listing only releases whose `manifest.json.sig` is the
  bare 64-byte form, which those hosts read.
- **Build and sign.** Generate the unsigned catalog as today, then sign it with every
  delegation it carries:

  ```bash
  node trust.mjs catalog --in catalog.json \
    --publisher publishers/srelens.json [--publisher publishers/<id>.json ...] \
    --version <N> --expires <UTC time> \
    --sign "$CATALOG_SIGNING_PRIVATE_KEY_FILE" > catalog.signed.json
  ```

  `--in` accepts the unsigned catalog and takes its `extensions` unchanged.
- **Version.** Increase `version` on every publish. Deriving it from the commit count or
  the time keeps it monotonic without a counter file. A host refuses a lower version, and
  a different catalog under a version it has already seen.
- **Expiry.** Choose `expires` long enough that a missed publish does not strand users,
  and short enough to bound how long a withheld update goes unnoticed; 30 days is a
  reasonable start. Re-sign on a schedule well inside it, even when nothing changed, or
  every host stops offering installs when it passes.
- **Keep the delegations** (`publishers/*.json`) in the repository, signed once each, and
  re-sign one only when that publisher's keys or namespaces change, raising its `version`.
  A change at the same version does not take effect for a publisher the build ships, such
  as srelens.

## Adding a publisher

1. The publisher generates its own Ed25519 key (`node trust.mjs keygen <publisher>.pem`)
   and sends only the public half, as an SPKI PEM
   (`openssl pkey -in <publisher>.pem -pubout -out <publisher>.pub.pem`).
   `node trust.mjs key <publisher>.pem` prints the key ID both sides can compare.
2. A catalog maintainer confirms the publisher controls the namespace it asks for (for
   example, the domain it reverses) and that no other publisher's namespace overlaps it.
3. The maintainer signs the delegation with the catalog key and adds it to the catalog:

   ```bash
   node trust.mjs publisher --id example --name "Example Labs" --key example.pub.pem \
     --namespace io.example --sign catalog.pem > publishers/example.json
   ```

4. The publisher signs each release (below). Its apps then install as "Signed by Example
   Labs", and its key cannot sign an ID outside `io.example`.

## Signing a release

For extension-flux, extension-argocd and any other publisher. The one change the Flux and
Argo CD release workflows need is the form of `manifest.json.sig`: `scripts/sign.mjs`
writes a signature that names its key. `APP_SIGNING_PRIVATE_KEY` is unchanged, as is the
check against `signing-public.pem` and the signed bytes. In `scripts/sign.mjs`:

```js
import { createHash } from "node:crypto";
// ...after `signature` has been made and verified against `trusted`:
// The key ID is the SHA-256 of the 32 raw key bytes, the last 32 of the SPKI encoding.
const keyid = createHash("sha256")
  .update(trusted.export({ format: "der", type: "spki" }).subarray(-32))
  .digest("hex");
writeFileSync(
  "dist/manifest.json.sig",
  `${JSON.stringify({ keyid, sig: signature.toString("base64") })}\n`,
);
```

Or sign with `node trust.mjs release --sign <key> dist/manifest.json > dist/manifest.json.sig`.
The release workflow still publishes `manifest.json`, `manifest.json.sig` and
`SHA256SUMS`, and the catalog entry still names the manifest's SHA-256.

A release signed this way is readable only by hosts with #559, so list it in
`catalog.signed.json` only, never in the frozen `catalog.json`. The published releases
(Argo CD 0.3.0, Flux 0.4.0 and earlier) keep their bare signatures and keep installing.

## Rollout order

1. Run the key ceremony and commit the two pinned documents.
2. Publish `catalog.signed.json` in srelens/extensions, delegating `org.srelens` to the
   srelens release key and listing the current releases.
3. Release the host. Its first catalog refresh fetches the signed catalog. Apps installed
   before it keep verifying through the shipped srelens delegation, catalog or not.
4. Switch the Flux and Argo CD release workflows to key-named signatures.

A host from before step 3 is unaffected throughout: it reads `catalog.json`.

## What this leaves for later

- **Root rotation ([#560](https://github.com/srelens/srelens/issues/560)).** The root
  document is versioned and signed by its own threshold, which is what TUF's rotation
  needs: version N+1 signed by the thresholds of both N and N+1, walked from the pinned
  root. The host does not walk a chain yet. A publisher rotates its own keys within its
  delegation: list the old and new keys, raise the delegation's `version`, then drop the
  old key. Installed apps keep the delegation that vouched for them, so dropping a key
  from the catalog alone does not quarantine apps it signed, while a build that ships the
  later delegation does. An overlap window and a key history are #560's.
- **Revocation ([#561](https://github.com/srelens/srelens/issues/561)).** The catalog can
  gain a revocation list without breaking released hosts. Load-time verification takes the
  trust root as a parameter; revocation adds the latest verified revocations to it.
- **Downgrades ([#563](https://github.com/srelens/srelens/issues/563)).** A replayed older
  catalog is refused, so the catalog cannot be rolled back to re-offer old releases as
  current. Installing an older signed release by other means (a pasted manifest, a
  rollback) is still possible, and is #563's per-app version floor.
- **Other catalogs ([#564](https://github.com/srelens/srelens/issues/564)).**
  `TrustRoot::from_signed_root` accepts any self-signed root document, and
  `SharedCatalog::with_trust` and `Apps::with_trust` verify against it. A user-imported
  root and a configurable URL build on those.
