//! Publisher signatures over app releases (#559). Which key may sign which app is decided by
//! the publisher delegations in `trust.rs`: a key signs only for the namespaces delegated to
//! its publisher. Nothing here names a publisher, a key or a repository.
use super::trust::{self, Delegations, PublicKey, Signer};
use super::*;

/// The largest release signature file. A signature that names its key is a small JSON
/// object; one from before #559 is the 64 signature bytes alone.
pub(super) const MAX_RELEASE_SIGNATURE: usize = 512;

/// An Ed25519 signature over a release's exact manifest bytes, and the key that made it
/// when the signature names one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReleaseSignature {
    pub(super) key_id: Option<String>,
    pub(super) bytes: Vec<u8>,
}

#[derive(Deserialize)]
struct KeyedSignature {
    keyid: String,
    sig: String,
}

/// Reads a release's signature file (`manifest.json.sig`): the 64 signature bytes that
/// every release before #559 published, or a signature naming its key,
/// `{"keyid": "<hex>", "sig": "<base64>"}`, as `scripts/extensions/trust.mjs release`
/// writes it.
pub(super) fn parse_release_signature(raw: &[u8]) -> Result<ReleaseSignature, String> {
    if raw.len() == limits::SIGNATURE_BYTES {
        return Ok(ReleaseSignature {
            key_id: None,
            bytes: raw.to_vec(),
        });
    }
    if raw.len() > MAX_RELEASE_SIGNATURE {
        return Err(format!(
            "A release signature is at most {MAX_RELEASE_SIGNATURE} bytes"
        ));
    }
    let keyed: KeyedSignature = serde_json::from_slice(raw).map_err(|_| {
        "A release signature is neither 64 signature bytes nor a signature naming its key"
            .to_owned()
    })?;
    if !trust::is_key_id(&keyed.keyid) {
        return Err(
            "A release signature's keyid must be 64 lowercase hexadecimal characters".into(),
        );
    }
    let bytes = trust::decode_base64(&keyed.sig, limits::SIGNATURE_BYTES)?;
    if bytes.len() != limits::SIGNATURE_BYTES {
        return Err(format!(
            "A release signature is {} bytes",
            limits::SIGNATURE_BYTES
        ));
    }
    Ok(ReleaseSignature {
        key_id: Some(keyed.keyid),
        bytes,
    })
}

/// IDs in a delegated namespace install only with that publisher's signature.
pub(super) fn reserved(id: &str, delegations: &Delegations) -> bool {
    delegations.owner(id).is_some()
}

/// Who signed `raw`, a manifest, as the app its own ID names.
pub(super) fn verify(
    raw: &[u8],
    signature: &[u8],
    key_id: Option<&str>,
    delegations: &Delegations,
) -> Result<Signer, String> {
    let source = std::str::from_utf8(raw).map_err(|_| "Signed manifest is not UTF-8")?;
    let manifest = Manifest::parse(source)?;
    verify_for(&manifest.id, raw, signature, key_id, delegations)
}

/// Who signed `raw` as app `id`: the publisher delegated `id`'s namespace, when the key
/// the signature names verifies it — or, for a signature that names none, any of that
/// publisher's keys. It does not depend on the manifest passing its rules, so both can be
/// reported together.
pub(super) fn verify_for(
    id: &str,
    raw: &[u8],
    signature: &[u8],
    key_id: Option<&str>,
    delegations: &Delegations,
) -> Result<Signer, String> {
    let publisher = delegations
        .owner(id)
        .ok_or_else(|| format!("No publisher is trusted to sign {id}"))?;
    let keys: Vec<&PublicKey> = match key_id {
        // The name only chooses the key. A key of another publisher is refused here, before
        // anything is verified with it: it may be a valid key, but not for this ID.
        Some(key_id) => vec![publisher
            .keys
            .iter()
            .find(|key| key.id == key_id)
            .ok_or_else(|| {
                format!(
                    "Key {key_id} may not sign {id}: apps in that namespace are signed only by {}",
                    publisher.name
                )
            })?],
        None => publisher.keys.iter().collect(),
    };
    if keys.iter().any(|key| key.verify(raw, signature)) {
        Ok(publisher.signer())
    } else {
        Err("App publisher signature is invalid".into())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::trust::testing;
    use super::*;
    use ring::signature::Ed25519KeyPair;

    /// The seed of the test publisher's key (#562), which the test root delegates
    /// `test.signed`: a private key in the repository, trusted by nothing but this crate's
    /// unit tests, which pin the test root.
    pub(in crate::extensions) const TEST_PUBLISHER_SEED: [u8; 32] =
        [testing::TEST_PUBLISHER_SEED; 32];

    /// Signs `raw` as the test publisher.
    pub(in crate::extensions) fn test_publisher_sign(raw: &[u8]) -> Vec<u8> {
        Ed25519KeyPair::from_seed_unchecked(&TEST_PUBLISHER_SEED)
            .unwrap()
            .sign(raw)
            .as_ref()
            .to_vec()
    }

    /// The test publisher is a delegation under the test root like any other: its key signs
    /// `test.signed` and nothing else (#562).
    #[test]
    fn the_test_publisher_key_signs_its_namespace_only() {
        let shipped = testing::root().shipped();
        let raw = b"signed by the test publisher";
        let signer = verify_for(
            "test.signed.packaged",
            raw,
            &test_publisher_sign(raw),
            None,
            &shipped,
        )
        .unwrap();
        assert_eq!(signer.id, "test-publisher");
        assert!(verify_for(
            "org.srelens.flux",
            raw,
            &test_publisher_sign(raw),
            None,
            &shipped
        )
        .is_err());
        assert_eq!(
            include_bytes!("../../tests/fixtures/packages/test-publisher.pub"),
            &testing::public(&testing::key(testing::TEST_PUBLISHER_SEED))[..]
        );
    }

    fn delegations() -> Delegations {
        let root = testing::root();
        let example = root.publisher(&testing::example_publisher()).unwrap();
        Delegations::merged(&root.shipped(), &Delegations::new(vec![example]).unwrap())
    }

    fn keyed(key: &Ed25519KeyPair, raw: &[u8]) -> ReleaseSignature {
        ReleaseSignature {
            key_id: Some(testing::id(key)),
            bytes: key.sign(raw).as_ref().to_vec(),
        }
    }

    fn manifest(id: &str) -> String {
        super::super::tests::manifest().replacen("org.example.argocd", id, 1)
    }

    #[test]
    fn a_publisher_key_signs_its_namespace_and_nothing_else() {
        let delegations = delegations();
        let key = testing::key(testing::EXAMPLE_SEED);
        let own = manifest("com.example-labs.app");
        let signature = keyed(&key, own.as_bytes());
        let signer = verify(
            own.as_bytes(),
            &signature.bytes,
            signature.key_id.as_deref(),
            &delegations,
        )
        .unwrap();
        assert_eq!(
            signer,
            Signer {
                id: "example".into(),
                name: "Example Labs".into()
            }
        );
        // The same key over an ID in srelens's namespace: refused by name before any
        // verification, and refused by verification when the signature names no key.
        let theirs = manifest("org.srelens.app");
        let signature = keyed(&key, theirs.as_bytes());
        let refused = verify(
            theirs.as_bytes(),
            &signature.bytes,
            signature.key_id.as_deref(),
            &delegations,
        )
        .unwrap_err();
        assert!(refused.contains("signed only by srelens"), "{refused}");
        let refused = verify(theirs.as_bytes(), &signature.bytes, None, &delegations).unwrap_err();
        assert!(refused.contains("signature is invalid"), "{refused}");
        // And over an ID no publisher was delegated.
        let nobody = manifest("com.nobody.app");
        let signature = keyed(&key, nobody.as_bytes());
        let refused = verify(
            nobody.as_bytes(),
            &signature.bytes,
            signature.key_id.as_deref(),
            &delegations,
        )
        .unwrap_err();
        assert!(refused.contains("No publisher is trusted"), "{refused}");
    }

    #[test]
    fn rejects_tampered_bytes_other_keys_and_malformed_signatures() {
        let delegations = delegations();
        let key = testing::key(testing::EXAMPLE_SEED);
        let raw = manifest("com.example-labs.app");
        let signature = keyed(&key, raw.as_bytes());
        let mut changed = raw.clone().into_bytes();
        changed.push(b' ');
        assert!(verify_for(
            "com.example-labs.app",
            &changed,
            &signature.bytes,
            None,
            &delegations
        )
        .is_err());
        let other = testing::key(0x42);
        let forged = keyed(&other, raw.as_bytes());
        assert!(verify_for(
            "com.example-labs.app",
            raw.as_bytes(),
            &forged.bytes,
            None,
            &delegations
        )
        .is_err());
        // Naming the right key does not make another key's signature valid.
        assert!(verify_for(
            "com.example-labs.app",
            raw.as_bytes(),
            &forged.bytes,
            signature.key_id.as_deref(),
            &delegations
        )
        .is_err());
        assert!(verify_for(
            "com.example-labs.app",
            raw.as_bytes(),
            &[],
            None,
            &delegations
        )
        .is_err());
        assert!(verify_for(
            "com.example-labs.app",
            raw.as_bytes(),
            &[0; 65],
            None,
            &delegations
        )
        .is_err());
    }

    #[test]
    fn releases_signed_before_559_verify_under_the_shipped_srelens_delegation() {
        let delegations = testing::root().shipped();
        for (id, raw, signature) in [
            (
                "org.srelens.argocd",
                &include_bytes!("../../tests/fixtures/argocd-0.3.0-manifest.json")[..],
                &include_bytes!("../../tests/fixtures/argocd-0.3.0-manifest.sig")[..],
            ),
            (
                "org.srelens.flux",
                &include_bytes!("../../tests/fixtures/flux-0.4.0-manifest.json")[..],
                &include_bytes!("../../tests/fixtures/flux-0.4.0-manifest.sig")[..],
            ),
        ] {
            let parsed = parse_release_signature(signature).unwrap();
            assert_eq!(parsed.key_id, None, "{id}");
            let signer = verify_for(id, raw, &parsed.bytes, None, &delegations).unwrap();
            assert_eq!(signer.name, "srelens");
            // Named, as a release signed since #559 names it.
            let release_key = trust::key_id(include_bytes!(
                "../../tests/fixtures/trust/srelens-apps.pub"
            ));
            assert!(verify_for(id, raw, &parsed.bytes, Some(&release_key), &delegations).is_ok());
        }
    }

    /// `scripts/extensions/trust.mjs release` wrote this signature, naming its key; the
    /// host reads and verifies it as a release signed since #559.
    #[test]
    fn a_release_signed_by_the_reference_script_verifies() {
        let raw = include_bytes!("../../tests/fixtures/trust/example-release.json");
        let file = include_bytes!("../../tests/fixtures/trust/example-release.json.sig");
        let signature = parse_release_signature(file).unwrap();
        assert_eq!(
            signature.key_id,
            Some(testing::id(&testing::key(testing::EXAMPLE_SEED)))
        );
        let signer = verify(
            raw,
            &signature.bytes,
            signature.key_id.as_deref(),
            &delegations(),
        )
        .unwrap();
        assert_eq!(signer.name, "Example Labs");
    }

    #[test]
    fn reads_both_release_signature_forms_and_nothing_else() {
        let raw = [7u8; 64];
        assert_eq!(
            parse_release_signature(&raw).unwrap(),
            ReleaseSignature {
                key_id: None,
                bytes: raw.to_vec()
            }
        );
        let key_id = "ab".repeat(32);
        let keyed = format!(
            "{{\"keyid\":\"{key_id}\",\"sig\":\"{}\"}}\n",
            trust::encode_base64(&raw)
        );
        assert_eq!(
            parse_release_signature(keyed.as_bytes()).unwrap(),
            ReleaseSignature {
                key_id: Some(key_id.clone()),
                bytes: raw.to_vec()
            }
        );
        for bad in [
            format!(
                "{{\"keyid\":\"{}\",\"sig\":\"{}\"}}",
                key_id.to_uppercase(),
                trust::encode_base64(&raw)
            ),
            format!(
                "{{\"keyid\":\"{key_id}\",\"sig\":\"{}\"}}",
                trust::encode_base64(&raw[..63])
            ),
            format!("{{\"keyid\":\"{key_id}\"}}"),
            "not a signature".into(),
            " ".repeat(MAX_RELEASE_SIGNATURE + 1),
        ] {
            assert!(parse_release_signature(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(parse_release_signature(&[0; 63]).is_err());
    }
}
