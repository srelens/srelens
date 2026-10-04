//! Who built a release: the check `srelens-tui update` makes before it trusts
//! anything a release publishes (#448).
//!
//! A checksum proves a download arrived intact, not who made it. The checksum
//! file sits on the same release as the archives, so anyone able to replace
//! one can replace both. The release workflow signs every asset with the key
//! in the repository's `KEYS` (`sign-artifacts` in `release.yml`), and this
//! checks a signature against the keys compiled into this binary. A release is
//! trusted only if a key that was in `KEYS` when this binary was built signed
//! it.

use std::fmt;
use std::io::Cursor;
use std::time::{Duration, SystemTime};

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::packet::{Signature, SignatureType};
use pgp::types::KeyDetails;

/// The release signing keys this binary trusts: the repository's `KEYS`,
/// compiled in. Compiled in rather than downloaded, because a key fetched
/// from the release being checked would only vouch for itself.
///
/// Binaries already out there keep the keys they were built with, so a new
/// key has to reach `KEYS` at least one release before it signs anything (see
/// "Release signing key" in `docs/DEVELOPMENT.md`).
pub const RELEASE_KEYS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../KEYS"));

/// Why a signature was not good enough to trust what it signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureProblem {
    /// Not a detached OpenPGP signature at all.
    Unreadable(String),
    /// A text-mode signature, which covers the text with its line endings
    /// normalised rather than the bytes that were published.
    NotBinary,
    /// Made by a key that is not among the trusted ones.
    UnknownSigner,
    /// Made by a trusted key that has since been revoked. Revocation is how a
    /// lost or stolen key is announced, so it applies to everything the key
    /// ever signed.
    KeyRevoked { fingerprint: String },
    /// Made by a trusted key that has expired. The key has to be valid when
    /// the update runs, not only when it signed: a retired key that leaked
    /// could otherwise sign anything with a back-dated timestamp.
    KeyExpired { fingerprint: String },
    /// The signature carries its own expiry, and that has passed.
    SignatureExpired,
    /// A trusted key's signature, but not over these bytes.
    Mismatch,
}

impl fmt::Display for SignatureProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the signature could not be read ({why})"),
            Self::NotBinary => write!(
                f,
                "it is a text-mode signature, which does not cover the published bytes"
            ),
            Self::UnknownSigner => write!(f, "no srelens release key made the signature"),
            Self::KeyRevoked { fingerprint } => {
                write!(
                    f,
                    "the release key {fingerprint} that signed it has been revoked"
                )
            }
            Self::KeyExpired { fingerprint } => {
                write!(
                    f,
                    "the release key {fingerprint} that signed it has expired"
                )
            }
            Self::SignatureExpired => write!(f, "the signature has expired"),
            Self::Mismatch => write!(f, "the file does not match its signature"),
        }
    }
}

/// Check that `signature`, an armored detached OpenPGP signature, is a good
/// signature over `data` by one of the keys in `keys`, and that the key is
/// neither revoked nor expired at `now`. Returns the signing key's
/// fingerprint, in the upper-case hex the install guide prints.
pub fn verify_release_signature(
    data: &[u8],
    signature: &[u8],
    keys: &str,
    now: SystemTime,
) -> Result<String, SignatureProblem> {
    let (signature, _) = DetachedSignature::from_armor_single(Cursor::new(signature))
        .map_err(|e| SignatureProblem::Unreadable(e.to_string()))?;
    let signature = signature.signature;
    if signature.typ() != Some(SignatureType::Binary) {
        return Err(SignatureProblem::NotBinary);
    }
    let key = trusted_keys(keys)
        .into_iter()
        .find(|key| made_by(&signature, key))
        .ok_or(SignatureProblem::UnknownSigner)?;
    let fingerprint = format!("{:X}", key.fingerprint());

    // The bytes first, so a changed file is reported as changed, whatever
    // else is also wrong.
    signature
        .verify(&key, data)
        .map_err(|_| SignatureProblem::Mismatch)?;
    if !key.details.revocation_signatures.is_empty() {
        return Err(SignatureProblem::KeyRevoked { fingerprint });
    }
    if key_expires_at(&key).is_some_and(|end| end <= now) {
        return Err(SignatureProblem::KeyExpired { fingerprint });
    }
    if signature_expires_at(&signature).is_some_and(|end| end <= now) {
        return Err(SignatureProblem::SignatureExpired);
    }
    Ok(fingerprint)
}

const KEY_BLOCK: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----";

/// Every key in `keys` whose own self-signatures verify.
///
/// `KEYS` grows by appending, so it can hold several armored blocks, and each
/// is read; the armor reader stops at the end of the block it started in. A
/// key whose self-signatures do not verify is not trusted at all, so a
/// damaged `KEYS` fails closed. That includes a revocation that does not
/// verify, which would otherwise hide a key's revocation.
fn trusted_keys(keys: &str) -> Vec<SignedPublicKey> {
    keys.match_indices(KEY_BLOCK)
        .filter_map(|(start, _)| {
            SignedPublicKey::from_armor_many(Cursor::new(keys[start..].as_bytes())).ok()
        })
        .flat_map(|(parsed, _)| parsed.filter_map(Result::ok).collect::<Vec<_>>())
        .filter(|key| key.verify_bindings().is_ok())
        .collect()
}

/// Whether `signature` names `key` as its issuer, by fingerprint or key ID.
fn made_by(signature: &Signature, key: &SignedPublicKey) -> bool {
    let fingerprint = key.fingerprint();
    let key_id = key.legacy_key_id();
    signature.issuer_fingerprint().contains(&&fingerprint)
        || signature.issuer_key_id().contains(&&key_id)
}

/// When `key` stops being valid, if it ever does: its creation time plus the
/// life its newest self-signature gives it. Re-signing with a new expiry is
/// how a key's life is extended, so the newest one decides.
fn key_expires_at(key: &SignedPublicKey) -> Option<SystemTime> {
    let newest = key
        .details
        .users
        .iter()
        .flat_map(|user| &user.signatures)
        .chain(&key.details.direct_signatures)
        .filter(|signature| made_by(signature, key))
        .max_by_key(|signature| signature.created())?;
    let life = newest.key_expiration_time()?.as_secs();
    // Zero is the wire format's "never", the same as no expiry at all.
    (life > 0).then(|| SystemTime::from(key.created_at()) + Duration::from_secs(life.into()))
}

/// When `signature` stops being valid, if it carries an expiry of its own.
fn signature_expires_at(signature: &Signature) -> Option<SystemTime> {
    let created = SystemTime::from(signature.created()?);
    let life = signature.signature_expiration_time()?.as_secs();
    (life > 0).then(|| created + Duration::from_secs(life.into()))
}
