//! Official app trust is pinned in the host, never supplied by catalog metadata.
use super::*;
use ring::signature::{UnparsedPublicKey, ED25519};

/// A publisher this host trusts: its key, the app ID namespace reserved for it,
/// and the only repository each of its apps may be released from.
struct Publisher {
    key: &'static [u8; 32],
    namespace: &'static str,
    repository_owner: &'static str,
    apps: &'static [(&'static str, &'static str)],
}

const PUBLISHERS: &[Publisher] = &[Publisher {
    key: include_bytes!("srelens-apps.pub"),
    namespace: "org.srelens.",
    repository_owner: "https://github.com/srelens/",
    apps: &[
        (
            "org.srelens.flux",
            "https://github.com/srelens/extension-flux",
        ),
        (
            "org.srelens.argocd",
            "https://github.com/srelens/extension-argocd",
        ),
    ],
}];

/// IDs in a trusted publisher's namespace install only with that publisher's signature.
pub(super) fn reserved(id: &str) -> bool {
    PUBLISHERS.iter().any(|p| id.starts_with(p.namespace))
}

/// A catalog entry naming a trusted namespace or repository must carry a valid signature.
pub(super) fn claims_official(id: &str, repository: &str) -> bool {
    reserved(id)
        || PUBLISHERS
            .iter()
            .any(|p| repository.starts_with(p.repository_owner))
}

fn publisher(id: &str) -> Option<(&'static Publisher, &'static str)> {
    PUBLISHERS.iter().find_map(|p| {
        p.apps
            .iter()
            .find(|(app, _)| *app == id)
            .map(|(_, repository)| (p, *repository))
    })
}

pub(super) fn repository(id: &str) -> Option<&'static str> {
    publisher(id).map(|(_, repository)| repository)
}

pub(super) fn verify(raw: &[u8], signature: &[u8]) -> Result<(), String> {
    let source = std::str::from_utf8(raw).map_err(|_| "Signed manifest is not UTF-8")?;
    let manifest = Manifest::parse(source)?;
    verify_for(&manifest.id, raw, signature)
}

/// Checks `signature` over `raw` with the key of the publisher that owns `id`. It does not
/// depend on the manifest passing its rules, so both can be reported together.
pub(super) fn verify_for(id: &str, raw: &[u8], signature: &[u8]) -> Result<(), String> {
    let (publisher, _) = publisher(id).ok_or("Signing key is not trusted for this app ID")?;
    verify_key(publisher.key, raw, signature)
}

fn verify_key(public_key: &[u8], raw: &[u8], signature: &[u8]) -> Result<(), String> {
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(raw, signature)
        .map_err(|_| "App publisher signature is invalid".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    #[test]
    fn rejects_tampered_bytes_wrong_keys_and_missing_signatures() {
        let key = Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
        let other = Ed25519KeyPair::from_seed_unchecked(&[43; 32]).unwrap();
        let raw = b"{\"id\":\"org.srelens.flux\"}";
        let signature = key.sign(raw);
        assert!(verify_key(key.public_key().as_ref(), raw, signature.as_ref()).is_ok());
        assert!(verify_key(key.public_key().as_ref(), b"changed", signature.as_ref()).is_err());
        assert!(verify_key(other.public_key().as_ref(), raw, signature.as_ref()).is_err());
        assert!(verify_key(key.public_key().as_ref(), raw, &[]).is_err());
        assert!(verify_key(key.public_key().as_ref(), raw, &[0; 65]).is_err());
    }

    #[test]
    fn official_identity_comes_from_one_publisher_table() {
        assert!(reserved("org.srelens.flux"));
        assert!(reserved("org.srelens.not-yet-released"));
        assert!(!reserved("org.srelensx.flux"));
        assert!(!reserved("org.example.flux"));
        assert_eq!(
            repository("org.srelens.argocd"),
            Some("https://github.com/srelens/extension-argocd")
        );
        assert_eq!(repository("org.srelens.not-yet-released"), None);
        assert!(claims_official(
            "org.other.app",
            "https://github.com/srelens/app"
        ));
        assert!(claims_official(
            "org.srelens.app",
            "https://github.com/other/app"
        ));
        assert!(!claims_official(
            "org.other.app",
            "https://github.com/srelensx/app"
        ));
    }
}
