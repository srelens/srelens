//! Who built a release, checked before `srectl update` trusts it (#448).
//!
//! The real release's checksum file and signature prove the keys compiled
//! into the binary verify what the release workflow actually publishes. The
//! rest are gpg fixtures, one per way a signature can fail to be good enough;
//! `tests/fixtures/release-signing/README.md` says how each was made.
//!
//! Most refusals are paired with the same fixture passing under the
//! conditions it lacks: the key trusted, the clock turned back. That pins the
//! refusal on the one thing being tested and not on a broken fixture.

use std::path::Path;
use std::time::{Duration, SystemTime};

use srectl::update_signature::{verify_release_signature, SignatureProblem, RELEASE_KEYS};

/// The release key `KEYS` holds today, as `docs/INSTALL.md` prints it.
const RELEASE_KEY: &str = "6CFC34803A21C0E6DB18BA47DDEEDBFF499D9481";
const UNTRUSTED_KEY: &str = "79A7FDABF98337B9F889BB1E4567A76BA5727326";
const REVOKED_KEY: &str = "3769ED123FE6D0C9750A664623B17B0E060783D7";
const EXPIRED_KEY: &str = "CC9DECE43DBE1C6F783500AFC7551F85FF680736";
const EXTENDED_KEY: &str = "A2DE0B17939F5A89086F4DF99B1F88FA2B04614B";

fn fixture(name: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/release-signing");
    std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn keys(name: &str) -> String {
    String::from_utf8(fixture(name)).expect("an armored key is text")
}

/// A moment as seconds since the Unix epoch.
fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// 2020-07-01, when the 2020 fixtures' key and signature were both valid.
fn mid_2020() -> SystemTime {
    at(1_593_561_600)
}

/// 2026-09-21, the day after 0.15.0 was signed. The release key was valid then
/// and has a fixed life, so a test that read the real clock would start
/// failing the day that key expires. That refusal is right, and
/// `an_expired_key_is_refused_even_for_what_it_signed_while_valid` already
/// pins it.
fn while_0_15_0_was_current() -> SystemTime {
    at(1_790_000_000)
}

#[test]
fn a_real_release_s_checksums_verify_against_the_keys_compiled_in() {
    assert_eq!(
        verify_release_signature(
            &fixture("srelens-tui-0.15.0-SHA256SUMS.txt"),
            &fixture("srelens-tui-0.15.0-SHA256SUMS.txt.asc"),
            RELEASE_KEYS,
            while_0_15_0_was_current(),
        ),
        Ok(RELEASE_KEY.to_string())
    );
}

#[test]
fn one_changed_byte_does_not_verify() {
    let mut sums = fixture("srelens-tui-0.15.0-SHA256SUMS.txt");
    // A hex digit of the first hash: what pointing a name at a different
    // archive looks like.
    sums[0] = if sums[0] == b'0' { b'1' } else { b'0' };
    assert_eq!(
        verify_release_signature(
            &sums,
            &fixture("srelens-tui-0.15.0-SHA256SUMS.txt.asc"),
            RELEASE_KEYS,
            SystemTime::now(),
        ),
        Err(SignatureProblem::Mismatch)
    );
}

#[test]
fn a_good_signature_from_a_key_outside_keys_is_refused() {
    let (data, signature) = (fixture("data.txt"), fixture("data.txt.untrusted.asc"));
    assert_eq!(
        verify_release_signature(&data, &signature, RELEASE_KEYS, SystemTime::now()),
        Err(SignatureProblem::UnknownSigner)
    );
    // The signature itself is fine: trusted, it verifies.
    assert_eq!(
        verify_release_signature(
            &data,
            &signature,
            &keys("untrusted-key.asc"),
            SystemTime::now()
        ),
        Ok(UNTRUSTED_KEY.to_string())
    );
}

/// `KEYS` is an archive that grows by appending. A new key arrives as a
/// second armored block, and must be trusted as much as the first.
#[test]
fn a_key_in_a_later_armored_block_of_keys_is_trusted() {
    let keys = format!("{RELEASE_KEYS}\n{}", keys("untrusted-key.asc"));
    assert_eq!(
        verify_release_signature(
            &fixture("data.txt"),
            &fixture("data.txt.untrusted.asc"),
            &keys,
            SystemTime::now(),
        ),
        Ok(UNTRUSTED_KEY.to_string())
    );
}

/// Revocation is how a lost or stolen key is announced, so it applies to
/// everything the key ever signed, whenever that was.
#[test]
fn a_revoked_key_is_refused_even_for_what_it_signed_before_revocation() {
    assert_eq!(
        verify_release_signature(
            &fixture("data.txt"),
            &fixture("data.txt.revoked.asc"),
            &keys("revoked-key.asc"),
            SystemTime::now(),
        ),
        Err(SignatureProblem::KeyRevoked {
            fingerprint: REVOKED_KEY.to_string()
        })
    );
}

/// The key must be current when the update runs, not merely when it signed.
/// A retired key that leaked could otherwise sign anything with a back-dated
/// timestamp.
#[test]
fn an_expired_key_is_refused_even_for_what_it_signed_while_valid() {
    let (data, signature, keys) = (
        fixture("data.txt"),
        fixture("data.txt.expired.asc"),
        keys("expired-key.asc"),
    );
    assert_eq!(
        verify_release_signature(&data, &signature, &keys, SystemTime::now()),
        Err(SignatureProblem::KeyExpired {
            fingerprint: EXPIRED_KEY.to_string()
        })
    );
    assert_eq!(
        verify_release_signature(&data, &signature, &keys, mid_2020()),
        Ok(EXPIRED_KEY.to_string()),
        "before the key expired, the same signature is good"
    );
}

/// Extending a key re-signs it with a later expiry, and the newest
/// self-signature is the one that counts. Reading the oldest would retire a
/// key its owner had kept alive.
#[test]
fn a_key_extended_before_it_expired_is_still_trusted() {
    assert_eq!(
        verify_release_signature(
            &fixture("data.txt"),
            &fixture("data.txt.extended.asc"),
            &keys("extended-key.asc"),
            SystemTime::now(),
        ),
        Ok(EXTENDED_KEY.to_string())
    );
}

#[test]
fn a_signature_past_its_own_expiry_is_refused() {
    let (data, signature, keys) = (
        fixture("data.txt"),
        fixture("data.txt.sig-expired.asc"),
        keys("untrusted-key.asc"),
    );
    assert_eq!(
        verify_release_signature(&data, &signature, &keys, SystemTime::now()),
        Err(SignatureProblem::SignatureExpired)
    );
    // Made 2020-06-01 with a one-day life.
    assert_eq!(
        verify_release_signature(&data, &signature, &keys, at(1_591_000_000)),
        Ok(UNTRUSTED_KEY.to_string()),
        "within its day, the same signature is good"
    );
}

/// A text-mode signature is over the text with its line endings normalised,
/// not over the bytes that were published.
#[test]
fn a_text_mode_signature_is_not_accepted_for_release_bytes() {
    assert_eq!(
        verify_release_signature(
            &fixture("data.txt"),
            &fixture("data.txt.textmode.asc"),
            &keys("untrusted-key.asc"),
            SystemTime::now(),
        ),
        Err(SignatureProblem::NotBinary)
    );
}

#[test]
fn something_that_is_not_a_signature_is_unreadable() {
    assert!(matches!(
        verify_release_signature(
            &fixture("data.txt"),
            b"<html>404 Not Found</html>",
            &keys("untrusted-key.asc"),
            SystemTime::now(),
        ),
        Err(SignatureProblem::Unreadable(_))
    ));
}
