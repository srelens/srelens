//! Official app trust is pinned in the host, never supplied by catalog metadata.
use super::*;
use ring::signature::{UnparsedPublicKey, ED25519};

const PUBLIC_KEY: &[u8; 32] = include_bytes!("srelens-apps.pub");

pub(super) fn repository(id: &str) -> Option<&'static str> {
    match id {
        "org.srelens.flux" => Some("https://github.com/srelens/extension-flux"),
        "org.srelens.argocd" => Some("https://github.com/srelens/extension-argocd"),
        _ => None,
    }
}

pub(super) fn verify(raw: &[u8], signature: &[u8]) -> Result<(), String> {
    verify_key(PUBLIC_KEY, raw, signature)?;
    let source = std::str::from_utf8(raw).map_err(|_| "Signed manifest is not UTF-8")?;
    let manifest = Manifest::parse(source)?;
    if repository(&manifest.id).is_none() {
        return Err("Signing key is not trusted for this app ID".into());
    }
    Ok(())
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
}
