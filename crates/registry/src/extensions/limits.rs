//! Size limits on caller-supplied extension inputs, checked while the input is decoded.
//!
//! A refused field is named in the error with its limit, and nothing past the limit is
//! kept: a signature is refused at its 65th byte, not after the array is collected.
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{Map, Value};
use srelens_plugin_host::MAX_MANIFEST_BYTES;
use std::fmt;

/// The length of an Ed25519 signature, the only kind a publisher key verifies.
pub(super) const SIGNATURE_BYTES: usize = 64;
/// The largest `settings` object `extensions.configure` accepts, measured as compact JSON.
pub(super) const MAX_SETTINGS_BYTES: usize = 64 * 1024;

/// Decodes an optional `signature`, refusing any length but [`SIGNATURE_BYTES`].
pub(super) fn signature<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
    Option::<Signature>::deserialize(d).map(|s| s.map(|s| s.0))
}

struct Signature(Vec<u8>);

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_seq(SignatureVisitor)
    }
}

struct SignatureVisitor;

fn wrong_signature_length<E: de::Error>() -> E {
    E::custom(format_args!(
        "signature must be exactly {SIGNATURE_BYTES} bytes"
    ))
}

impl<'de> Visitor<'de> for SignatureVisitor {
    type Value = Signature;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a signature of exactly {SIGNATURE_BYTES} bytes")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Signature, A::Error> {
        let mut bytes = Vec::with_capacity(SIGNATURE_BYTES);
        while let Some(byte) = seq.next_element::<u8>()? {
            if bytes.len() == SIGNATURE_BYTES {
                return Err(wrong_signature_length());
            }
            bytes.push(byte);
        }
        if bytes.len() != SIGNATURE_BYTES {
            return Err(wrong_signature_length());
        }
        Ok(Signature(bytes))
    }
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Signature, E> {
        if v.len() != SIGNATURE_BYTES {
            return Err(wrong_signature_length());
        }
        Ok(Signature(v.to_vec()))
    }
}

/// Decodes manifest text, refusing more than [`MAX_MANIFEST_BYTES`] before it is decoded.
pub(super) fn manifest<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let text = String::deserialize(d)?;
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(de::Error::custom(format_args!(
            "manifest exceeds {} KiB ({MAX_MANIFEST_BYTES} bytes)",
            MAX_MANIFEST_BYTES / 1024
        )));
    }
    Ok(text)
}

/// Decodes a package file sent as base64 (#562), refusing text that could decode to more
/// than [`MAX_PACKAGE_BYTES`] before decoding it.
///
/// [`MAX_PACKAGE_BYTES`]: super::package::MAX_PACKAGE_BYTES
pub(super) fn package<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    use base64::Engine as _;
    const MOST: usize = super::package::MAX_PACKAGE_BYTES;
    let too_large =
        || de::Error::custom(format_args!("package exceeds {} MiB", MOST / (1024 * 1024)));
    let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
    // Padding makes a few lengths past the limit encode as long as the limit does, so the
    // decoded length is checked too.
    if text.len() > MOST.div_ceil(3) * 4 {
        return Err(too_large());
    }
    let package = base64::engine::general_purpose::STANDARD
        .decode(text.as_bytes())
        .map_err(|e| de::Error::custom(format_args!("package is not base64: {e}")))?;
    if package.len() > MOST {
        return Err(too_large());
    }
    Ok(package)
}

/// Decodes an optional digest list, refusing one over [`MAX_DIGESTS_BYTES`].
///
/// [`MAX_DIGESTS_BYTES`]: super::package::MAX_DIGESTS_BYTES
pub(super) fn digests<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    const MOST: usize = super::package::MAX_DIGESTS_BYTES;
    let text = Option::<String>::deserialize(d)?;
    if text.as_ref().is_some_and(|text| text.len() > MOST) {
        return Err(de::Error::custom(format_args!(
            "digests exceed {} KiB ({MOST} bytes)",
            MOST / 1024
        )));
    }
    Ok(text)
}

/// Decodes a `settings` object, refusing one over [`MAX_SETTINGS_BYTES`] as compact JSON.
pub(super) fn settings<'de, D: Deserializer<'de>>(d: D) -> Result<Map<String, Value>, D::Error> {
    let settings = Map::deserialize(d)?;
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, &settings).map_err(de::Error::custom)?;
    if counter.0 > MAX_SETTINGS_BYTES {
        return Err(de::Error::custom(format_args!(
            "settings exceed {} KiB ({MAX_SETTINGS_BYTES} bytes) as JSON",
            MAX_SETTINGS_BYTES / 1024
        )));
    }
    Ok(settings)
}

/// Counts what would be written, so measuring `settings` allocates nothing.
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Deserialize, Debug)]
    struct In {
        #[serde(default, deserialize_with = "signature")]
        signature: Option<Vec<u8>>,
    }

    #[test]
    fn signature_must_be_exactly_64_bytes() {
        let decode = |v: Value| serde_json::from_value::<In>(v).map(|i| i.signature);
        assert_eq!(decode(json!({})).unwrap(), None);
        assert_eq!(decode(json!({"signature": null})).unwrap(), None);
        assert_eq!(
            decode(json!({"signature": vec![7u8; 64]})).unwrap(),
            Some(vec![7; 64])
        );
        for len in [0, 63, 65, 1024 * 1024] {
            let err = decode(json!({"signature": vec![0u8; len]})).unwrap_err();
            assert!(
                err.to_string()
                    .contains("signature must be exactly 64 bytes"),
                "{err}"
            );
        }
        assert!(decode(json!({"signature": [256]})).is_err());
    }

    #[test]
    fn a_package_is_base64_within_its_limit_and_a_digest_list_within_its_own() {
        use base64::Engine as _;
        #[derive(Deserialize)]
        struct P {
            #[serde(deserialize_with = "package")]
            package: Vec<u8>,
        }
        #[derive(Deserialize)]
        struct V {
            #[serde(default, deserialize_with = "digests")]
            digests: Option<String>,
        }
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let decode = |v: Value| serde_json::from_value::<P>(v).map(|p| p.package);
        assert_eq!(
            decode(json!({"package": encode(b"\x1f\x8b")})).unwrap(),
            b"\x1f\x8b"
        );
        let most = super::super::package::MAX_PACKAGE_BYTES;
        let at_limit = encode(&vec![0; most]);
        assert_eq!(decode(json!({"package": at_limit})).unwrap().len(), most);
        for over in [most + 1, most + 3, most + 4] {
            let refused = decode(json!({"package": encode(&vec![0; over])})).map(|p| p.len());
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains("package exceeds 16 MiB")),
                "{over} bytes: {refused:?}"
            );
        }
        assert!(decode(json!({"package": "not base64!"}))
            .unwrap_err()
            .to_string()
            .contains("not base64"));
        // The wrapper sends a string; an array of numbers is refused, not decoded.
        assert!(decode(json!({"package": [31, 139]})).is_err());

        let digests = |v: Value| serde_json::from_value::<V>(v).map(|v| v.digests);
        assert_eq!(digests(json!({})).unwrap(), None);
        let most = super::super::package::MAX_DIGESTS_BYTES;
        assert!(digests(json!({"digests": "x".repeat(most)})).is_ok());
        assert!(digests(json!({"digests": "x".repeat(most + 1)}))
            .unwrap_err()
            .to_string()
            .contains("digests exceed 64 KiB"));
    }

    #[test]
    fn settings_over_the_limit_are_refused() {
        #[derive(Deserialize)]
        struct S {
            #[serde(deserialize_with = "settings")]
            #[allow(dead_code)]
            settings: Map<String, Value>,
        }
        // `{"a":"…"}` is 8 bytes of framing around the value.
        let fits = "x".repeat(MAX_SETTINGS_BYTES - 8);
        assert!(serde_json::from_value::<S>(json!({"settings": {"a": fits}})).is_ok());
        let over = "x".repeat(MAX_SETTINGS_BYTES - 7);
        let err = serde_json::from_value::<S>(json!({"settings": {"a": over}}))
            .err()
            .unwrap();
        assert!(err.to_string().contains("settings exceed 64 KiB"), "{err}");
    }
}
