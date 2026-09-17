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
