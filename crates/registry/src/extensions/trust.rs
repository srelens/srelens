//! What the host trusts extension distribution to (#559): a TUF-style chain from the root
//! this build pins to the publisher that signed an app.
//!
//! 1. **Root.** This build pins a root document (`trust/root.json`). It lists the keys
//!    of two roles, each with a signature threshold: `root`, which vouches for the root
//!    itself and is kept offline, and `catalog`, which signs the catalog.
//! 2. **Catalog.** The catalog is signed by the catalog role, and carries a version and
//!    an expiry (`catalog.rs`).
//! 3. **Publishers.** The catalog delegates app ID namespaces to publishers. Each
//!    delegation is its own document signed by the catalog role, so an installed app can
//!    keep the one that vouched for it and be verified again with no catalog at hand.
//! 4. **Releases.** A publisher's key signs its releases, and only for the namespaces
//!    delegated to it (`signing.rs`).
//!
//! Every signed document is a [DSSE](https://github.com/secure-systems-lab/dsse)
//! envelope. The signature covers the document's type and its exact bytes, so nothing is
//! canonicalized before it is checked, and a signature over one kind of document never
//! verifies as another.
//!
//! No key, publisher or namespace is written in this code. The only ones this build
//! trusts are in the documents it pins, and the ones those vouch for.
use base64::{
    alphabet,
    engine::{
        general_purpose::{GeneralPurpose, GeneralPurposeConfig},
        DecodePaddingMode,
    },
    Engine as _,
};
use ring::signature::{UnparsedPublicKey, ED25519};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use srelens_plugin_host::is_format_character;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

/// The payload type of each signed document. A signature covers its document's type, so
/// one made over a publisher delegation cannot pass as a catalog or a root.
pub(super) const ROOT_TYPE: &str = "application/vnd.srelens.root+json";
pub(super) const CATALOG_TYPE: &str = "application/vnd.srelens.catalog+json";
pub(super) const PUBLISHER_TYPE: &str = "application/vnd.srelens.publisher+json";

/// The most keys a root lists, a role names or a publisher holds.
const MAX_KEYS: usize = 16;
/// The most signatures an envelope carries. Each costs a verification, so a document
/// cannot make the host check thousands.
const MAX_SIGNATURES: usize = 16;
/// The largest root and publisher documents, decoded.
const MAX_ROOT_BYTES: usize = 64 * 1024;
const MAX_PUBLISHER_BYTES: usize = 16 * 1024;
/// The most namespaces one publisher is delegated, and the most publishers in a catalog.
const MAX_NAMESPACES: usize = 16;
pub(super) const MAX_PUBLISHERS: usize = 256;
/// Namespaces and publisher IDs are held to the app ID's length limit.
const MAX_NAMESPACE_BYTES: usize = 128;
/// A publisher's name is shown beside every app it signed.
const MAX_PUBLISHER_NAME_CHARS: usize = 64;

/// DSSE lets a signer use either base64 alphabet, padded or not, and a verifier must
/// accept all four.
const PADDING: GeneralPurposeConfig =
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
const BASE64: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, PADDING);
const BASE64_URL: GeneralPurpose = GeneralPurpose::new(&alphabet::URL_SAFE, PADDING);

/// Decodes base64 of at most `max` bytes, refusing longer text before decoding it.
pub(super) fn decode_base64(text: &str, max: usize) -> Result<Vec<u8>, String> {
    if text.len() > max.div_ceil(3) * 4 {
        return Err(format!("A signed value exceeds {max} bytes"));
    }
    BASE64
        .decode(text)
        .or_else(|_| BASE64_URL.decode(text))
        .map_err(|_| "A signed value is not base64".to_owned())
}

/// The host never signs; tests and the fuzz targets do.
#[cfg(any(test, feature = "fuzzing"))]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A key's ID: the SHA-256 of its 32 raw bytes, in lowercase hex. It is computed, never
/// taken from a document, so a document cannot name one key by another's ID.
pub(super) fn key_id(public: &[u8]) -> String {
    format!("{:x}", Sha256::digest(public))
}

/// A key ID as a document or a caller writes it: 64 lowercase hexadecimal characters.
pub(super) fn is_key_id(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    if !is_key_id(text) {
        return None;
    }
    let mut bytes = [0; 32];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(bytes)
}

/// A signed document: a DSSE envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(super) struct Envelope {
    #[serde(rename = "payloadType")]
    pub(super) payload_type: String,
    /// The document's exact bytes, in base64.
    pub(super) payload: String,
    pub(super) signatures: Vec<EnvelopeSignature>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(super) struct EnvelopeSignature {
    /// Which key made it: DSSE's optional, unauthenticated hint. One that names a key
    /// narrows the search to that key; one left out, or empty, is tried against each of
    /// the role's keys. Either way a signature counts only when a key the role lists
    /// verifies it, and each key counts once.
    #[serde(default)]
    pub(super) keyid: String,
    /// The Ed25519 signature, in base64.
    pub(super) sig: String,
}

/// DSSE's pre-authentication encoding: what a signature actually covers.
pub(super) fn pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut message = format!(
        "DSSEv1 {} {payload_type} {} ",
        payload_type.len(),
        payload.len()
    )
    .into_bytes();
    message.extend_from_slice(payload);
    message
}

/// A key as a document lists it, in TUF's shape. Only Ed25519 is accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(super) struct KeySpec {
    pub(super) keytype: String,
    pub(super) scheme: String,
    pub(super) keyval: KeyValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(super) struct KeyValue {
    /// The 32-byte public key, in lowercase hex.
    pub(super) public: String,
}

#[cfg(any(test, feature = "fuzzing"))]
#[cfg_attr(not(test), allow(dead_code))]
impl KeySpec {
    pub(super) fn ed25519(public: &[u8]) -> Self {
        Self {
            keytype: "ed25519".into(),
            scheme: "ed25519".into(),
            keyval: KeyValue {
                public: public.iter().map(|b| format!("{b:02x}")).collect(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PublicKey {
    pub(super) id: String,
    bytes: [u8; 32],
}

impl PublicKey {
    fn parse(spec: &KeySpec) -> Result<Self, String> {
        if spec.keytype != "ed25519" || spec.scheme != "ed25519" {
            return Err(format!(
                "Unsupported key type {:?}; only ed25519 keys are trusted",
                spec.keytype
            ));
        }
        let bytes = hex32(&spec.keyval.public)
            .ok_or("An ed25519 key must be 64 lowercase hexadecimal characters")?;
        Ok(Self {
            id: key_id(&bytes),
            bytes,
        })
    }

    pub(super) fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        UnparsedPublicKey::new(&ED25519, &self.bytes)
            .verify(message, signature)
            .is_ok()
    }
}

/// Keys that must together sign a document: at least `threshold` distinct ones.
#[derive(Clone, Debug)]
struct Role {
    name: &'static str,
    keys: Vec<PublicKey>,
    threshold: usize,
}

impl Role {
    /// The payload of `envelope`, when it is a `payload_type` document of at most `max`
    /// bytes signed by at least `threshold` of this role's keys. Only the bytes that were
    /// verified are returned, as DSSE requires: nothing is read from the envelope again.
    fn open(&self, envelope: &Envelope, payload_type: &str, max: usize) -> Result<Vec<u8>, String> {
        if envelope.payload_type != payload_type {
            return Err(format!(
                "Expected a document of type {payload_type}, not {:?}",
                envelope.payload_type
            ));
        }
        if envelope.signatures.len() > MAX_SIGNATURES {
            return Err(format!(
                "A signed document carries more than {MAX_SIGNATURES} signatures"
            ));
        }
        let payload = decode_base64(&envelope.payload, max)?;
        let message = pae(payload_type, &payload);
        // A key counts once, however many of its signatures an envelope repeats.
        let mut signed = BTreeSet::new();
        for signature in &envelope.signatures {
            let Ok(bytes) = decode_base64(&signature.sig, 64) else {
                continue;
            };
            // At most MAX_SIGNATURES of MAX_KEYS verifications, for an envelope that names
            // no key.
            let verified = self
                .keys
                .iter()
                .filter(|key| signature.keyid.is_empty() || key.id == signature.keyid)
                .filter(|key| !signed.contains(&key.id))
                .find(|key| key.verify(&message, &bytes));
            if let Some(key) = verified {
                signed.insert(key.id.clone());
            }
        }
        if signed.len() < self.threshold {
            return Err(format!(
                "Not signed by the {} role: {} of the {} signatures it needs verified",
                self.name,
                signed.len(),
                self.threshold
            ));
        }
        Ok(payload)
    }
}

#[derive(Deserialize)]
struct RoleDocument {
    keyids: Vec<String>,
    threshold: usize,
}

/// A root document. Unknown fields and roles are ignored, so a later root can add a role
/// (a timestamp role, say) without every released host refusing it.
#[derive(Deserialize)]
struct RootDocument {
    #[serde(rename = "_type")]
    kind: String,
    version: u64,
    keys: BTreeMap<String, KeySpec>,
    roles: BTreeMap<String, RoleDocument>,
}

/// A verified root: its version and the two roles this host uses.
#[derive(Clone, Debug)]
struct Root {
    version: u64,
    root: Role,
    catalog: Role,
}

fn parse_root(payload: &[u8]) -> Result<Root, String> {
    let document: RootDocument =
        serde_json::from_slice(payload).map_err(|e| format!("Invalid root document: {e}"))?;
    if document.kind != "root" {
        return Err("Not a root document".into());
    }
    if document.version == 0 {
        return Err("A root document's version starts at 1".into());
    }
    if document.keys.len() > MAX_KEYS {
        return Err(format!("A root lists more than {MAX_KEYS} keys"));
    }
    let mut keys = BTreeMap::new();
    for (id, spec) in &document.keys {
        let key = PublicKey::parse(spec)?;
        if &key.id != id {
            return Err(format!("Root key {id} is listed under another key's ID"));
        }
        keys.insert(id.clone(), key);
    }
    let role = |name: &'static str| -> Result<Role, String> {
        let spec = document
            .roles
            .get(name)
            .ok_or_else(|| format!("The root names no {name} role"))?;
        if spec.keyids.len() > MAX_KEYS {
            return Err(format!("The {name} role names more than {MAX_KEYS} keys"));
        }
        let mut role_keys: Vec<PublicKey> = Vec::new();
        for id in &spec.keyids {
            let key = keys.get(id).ok_or_else(|| {
                format!("The {name} role names key {id}, which the root does not list")
            })?;
            if role_keys.iter().any(|known| &known.id == id) {
                return Err(format!("The {name} role names key {id} twice"));
            }
            role_keys.push(key.clone());
        }
        if spec.threshold == 0 || spec.threshold > role_keys.len() {
            return Err(format!(
                "The {name} role's threshold must be between 1 and its {} keys",
                role_keys.len()
            ));
        }
        Ok(Role {
            name,
            keys: role_keys,
            threshold: spec.threshold,
        })
    };
    Ok(Root {
        version: document.version,
        root: role("root")?,
        catalog: role("catalog")?,
    })
}

/// Opens a signed root document: it must be signed by its own root role.
fn open_root(raw: &[u8]) -> Result<Root, String> {
    let envelope: Envelope =
        serde_json::from_slice(raw).map_err(|e| format!("Invalid signed root: {e}"))?;
    // The keys come from the payload being checked; they are trusted only once enough of
    // them have signed it.
    let payload = decode_base64(&envelope.payload, MAX_ROOT_BYTES)?;
    let claimed = parse_root(&payload)?;
    let payload = claimed.root.open(&envelope, ROOT_TYPE, MAX_ROOT_BYTES)?;
    parse_root(&payload)
}

/// A publisher delegation as its document holds it. Unknown fields are ignored.
#[derive(Deserialize)]
struct PublisherDocument {
    #[serde(rename = "_type")]
    kind: String,
    version: u64,
    id: String,
    name: String,
    keys: Vec<KeySpec>,
    namespaces: Vec<String>,
}

/// A publisher the catalog role vouches for: its keys may sign apps in its namespaces,
/// and no others.
#[derive(Clone, Debug)]
pub(super) struct Publisher {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) version: u64,
    pub(super) keys: Vec<PublicKey>,
    pub(super) namespaces: Vec<String>,
    /// The signed delegation it was read from, which an install keeps as its evidence.
    pub(super) envelope: Envelope,
}

/// Who signed an app, as the host reports it: "Signed by <name>".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(super) struct Signer {
    pub(super) id: String,
    pub(super) name: String,
}

impl Publisher {
    pub(super) fn signer(&self) -> Signer {
        Signer {
            id: self.id.clone(),
            name: self.name.clone(),
        }
    }

    pub(super) fn covers(&self, app_id: &str) -> bool {
        self.namespaces.iter().any(|ns| covers(ns, app_id))
    }
}

/// Whether `id` is in namespace `ns`: the namespace itself, or below it at a label
/// boundary, so `org.srelens` covers `org.srelens.flux` and not `org.srelensx.flux`.
/// Namespaces are lowercase; a local manifest's ID may not be, and `org.Srelens.flux` is
/// covered too, so a change of case cannot step outside a reservation.
pub(super) fn covers(ns: &str, id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id == ns || (id.len() > ns.len() && id.starts_with(ns) && id.as_bytes()[ns.len()] == b'.')
}

/// Whether two namespaces share an ID: one is the other, or covers it.
fn overlap(left: &str, right: &str) -> bool {
    covers(left, right) || covers(right, left)
}

/// Dot-separated labels of lowercase letters, digits and `-`, as an app ID's are.
fn labels(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAMESPACE_BYTES
        && text.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn parse_publisher(payload: &[u8], envelope: &Envelope) -> Result<Publisher, String> {
    let document: PublisherDocument = serde_json::from_slice(payload)
        .map_err(|e| format!("Invalid publisher delegation: {e}"))?;
    if document.kind != "publisher" {
        return Err("Not a publisher delegation".into());
    }
    if document.version == 0 {
        return Err("A publisher delegation's version starts at 1".into());
    }
    if !labels(&document.id) || document.id.contains('.') {
        return Err("A publisher ID is one label of lowercase letters, digits and -".into());
    }
    let name_chars = document.name.chars().count();
    // Shown as "Signed by <name>", so it is held to the rule a catalog name is.
    if document.name.trim().is_empty()
        || name_chars > MAX_PUBLISHER_NAME_CHARS
        || document
            .name
            .chars()
            .any(|c| c.is_control() || is_format_character(c))
    {
        return Err(format!(
            "Publisher {}'s name must be 1-{MAX_PUBLISHER_NAME_CHARS} characters with no control or invisible formatting character",
            document.id
        ));
    }
    if document.keys.is_empty() || document.keys.len() > MAX_KEYS {
        return Err(format!(
            "Publisher {} must hold 1-{MAX_KEYS} keys",
            document.id
        ));
    }
    let mut keys: Vec<PublicKey> = Vec::new();
    for spec in &document.keys {
        let key = PublicKey::parse(spec)?;
        if keys.contains(&key) {
            return Err(format!(
                "Publisher {} lists key {} twice",
                document.id, key.id
            ));
        }
        keys.push(key);
    }
    if document.namespaces.is_empty() || document.namespaces.len() > MAX_NAMESPACES {
        return Err(format!(
            "Publisher {} must be delegated 1-{MAX_NAMESPACES} namespaces",
            document.id
        ));
    }
    for (index, ns) in document.namespaces.iter().enumerate() {
        // Two labels at least: a delegation of `org` or `com` would hand one publisher
        // every app under a top-level name.
        if !labels(ns) || !ns.contains('.') {
            return Err(format!(
                "Publisher {}'s namespace {ns:?} must be two or more dot-separated labels of lowercase letters, digits and -",
                document.id
            ));
        }
        if document.namespaces[..index]
            .iter()
            .any(|other| overlap(ns, other))
        {
            return Err(format!(
                "Publisher {} is delegated namespace {ns} twice",
                document.id
            ));
        }
    }
    Ok(Publisher {
        id: document.id,
        name: document.name,
        version: document.version,
        keys,
        namespaces: document.namespaces,
        envelope: envelope.clone(),
    })
}

/// Publishers with at most one per app ID: no two share a namespace, an ID or a key.
#[derive(Clone, Debug, Default)]
pub(super) struct Delegations(Vec<Publisher>);

impl Delegations {
    pub(super) fn new(publishers: Vec<Publisher>) -> Result<Self, String> {
        if publishers.len() > MAX_PUBLISHERS {
            return Err(format!("More than {MAX_PUBLISHERS} publishers"));
        }
        for (index, publisher) in publishers.iter().enumerate() {
            for other in &publishers[..index] {
                if publisher.id == other.id {
                    return Err(format!("Publisher {} is delegated twice", publisher.id));
                }
                // A key signs for one publisher, so revoking it (#561) names whose apps
                // it takes with it.
                if let Some(key) = publisher.keys.iter().find(|key| other.keys.contains(key)) {
                    return Err(format!(
                        "Key {} is delegated to both {} and {}",
                        key.id, other.id, publisher.id
                    ));
                }
                if let Some(ns) = publisher
                    .namespaces
                    .iter()
                    .find(|ns| other.namespaces.iter().any(|theirs| overlap(ns, theirs)))
                {
                    return Err(format!(
                        "Namespace {ns} of publisher {} overlaps publisher {}'s",
                        publisher.id, other.id
                    ));
                }
            }
        }
        Ok(Self(publishers))
    }

    /// The delegations this build `shipped`, then each of `other`'s (a catalog's, or the
    /// one an install kept) that touches none of their IDs, keys or namespaces. `other`
    /// moves a shipped publisher on only with a later version of that publisher's
    /// delegation: at the same version the shipped one stands, and no other publisher
    /// takes a namespace the build ships. One rule for every caller, so an install and
    /// each later load agree on who signed an app, and a changed delegation must carry a
    /// higher version, as TUF requires of changed metadata.
    pub(super) fn merged(shipped: &Self, other: &Self) -> Self {
        fn clash(left: &Publisher, right: &Publisher) -> bool {
            left.id == right.id
                || left.keys.iter().any(|key| right.keys.contains(key))
                || left
                    .namespaces
                    .iter()
                    .any(|ns| right.namespaces.iter().any(|theirs| overlap(ns, theirs)))
        }
        let mut publishers = shipped.0.clone();
        for publisher in &other.0 {
            match publishers.iter().position(|known| known.id == publisher.id) {
                Some(index) if publishers[index].version < publisher.version => {
                    let others_clear = publishers
                        .iter()
                        .enumerate()
                        .all(|(other, known)| other == index || !clash(known, publisher));
                    if others_clear {
                        publishers[index] = publisher.clone();
                    }
                }
                Some(_) => {}
                None => {
                    if !publishers.iter().any(|known| clash(known, publisher)) {
                        publishers.push(publisher.clone());
                    }
                }
            }
        }
        Self(publishers)
    }

    /// The publisher delegated `app_id`'s namespace, if any: the only one whose keys may
    /// sign it.
    pub(super) fn owner(&self, app_id: &str) -> Option<&Publisher> {
        self.0.iter().find(|publisher| publisher.covers(app_id))
    }

    pub(super) fn publishers(&self) -> &[Publisher] {
        &self.0
    }
}

/// The documents this build pins. In this crate's tests they are the test root in
/// `tests/fixtures/trust`, whose private keys are derived from seeds anyone can read;
/// `the_pinned_documents_trust_no_test_key` keeps those out of the build.
#[cfg(not(test))]
const PINNED_ROOT: &[u8] = include_bytes!("trust/root.json");
#[cfg(not(test))]
const PINNED_PUBLISHERS: &[u8] = include_bytes!("trust/publishers.json");
#[cfg(test)]
const PINNED_ROOT: &[u8] = include_bytes!("../../tests/fixtures/trust/root.json");
#[cfg(test)]
const PINNED_PUBLISHERS: &[u8] = include_bytes!("../../tests/fixtures/trust/publishers.json");

/// What the pinned files hold until the key ceremony replaces them
/// (docs/extensions/trust.md).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placeholder {
    #[allow(dead_code)]
    placeholder: String,
}

const NO_PINNED_ROOT: &str = "This build pins no srelens root key, so it trusts no catalog and no publisher signature. The root and publisher documents are created by the key ceremony in docs/extensions/trust.md";

/// The keys this host trusts extension distribution to: a verified root, and the
/// publisher delegations shipped beside it.
///
/// The shipped delegations let apps installed before #559 be verified with no catalog at
/// hand: their proofs name no delegation, and those were all srelens releases.
#[derive(Clone)]
pub struct TrustRoot(Arc<Result<Anchors, String>>);

struct Anchors {
    root: Root,
    shipped: Delegations,
}

impl std::fmt::Debug for TrustRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.as_ref() {
            Ok(anchors) => write!(f, "TrustRoot(version {})", anchors.root.version),
            Err(reason) => write!(f, "TrustRoot(unavailable: {reason})"),
        }
    }
}

impl TrustRoot {
    /// The root and publisher delegations this build pins. When they are missing or do
    /// not verify, every catalog and publisher signature is refused, with why.
    pub fn pinned() -> Self {
        static PINNED: OnceLock<TrustRoot> = OnceLock::new();
        PINNED
            .get_or_init(|| Self::open(PINNED_ROOT, Some(PINNED_PUBLISHERS)))
            .clone()
    }

    /// A root other than the pinned one, such as a test's: a signed root document,
    /// trusted once its own root role has signed it. It ships no publisher delegations.
    pub fn from_signed_root(raw: &[u8]) -> Result<Self, String> {
        let root = Self::open(raw, None);
        root.anchors()?;
        Ok(root)
    }

    /// A root and the publisher delegations shipped with it, as a build pins them: a test's,
    /// whose shipped delegations reserve their namespaces before any catalog is fetched.
    /// `publishers` is a JSON array of signed delegations the root's catalog role signed.
    pub fn from_signed_documents(root: &[u8], publishers: &[u8]) -> Result<Self, String> {
        let root = Self::open(root, Some(publishers));
        root.anchors()?;
        Ok(root)
    }

    fn open(root: &[u8], publishers: Option<&[u8]>) -> Self {
        let opened = (|| {
            if serde_json::from_slice::<Placeholder>(root).is_ok() {
                return Err(NO_PINNED_ROOT.to_owned());
            }
            let root = open_root(root)?;
            let shipped = match publishers {
                None => Delegations::default(),
                Some(raw) => {
                    if serde_json::from_slice::<Placeholder>(raw).is_ok() {
                        return Err(NO_PINNED_ROOT.to_owned());
                    }
                    let envelopes: Vec<Envelope> = serde_json::from_slice(raw)
                        .map_err(|e| format!("Invalid pinned publisher delegations: {e}"))?;
                    let publishers = envelopes
                        .iter()
                        .map(|envelope| open_publisher(&root.catalog, envelope))
                        .collect::<Result<Vec<_>, _>>()?;
                    Delegations::new(publishers)?
                }
            };
            Ok(Anchors { root, shipped })
        })();
        Self(Arc::new(opened))
    }

    fn anchors(&self) -> Result<&Anchors, String> {
        self.0.as_ref().as_ref().map_err(Clone::clone)
    }

    /// The payload of a `payload_type` document the catalog role signed.
    pub(super) fn open_catalog_signed(
        &self,
        envelope: &Envelope,
        payload_type: &str,
        max: usize,
    ) -> Result<Vec<u8>, String> {
        self.anchors()?
            .root
            .catalog
            .open(envelope, payload_type, max)
    }

    /// A publisher delegation the catalog role signed.
    pub(super) fn publisher(&self, envelope: &Envelope) -> Result<Publisher, String> {
        open_publisher(&self.anchors()?.root.catalog, envelope)
    }

    /// The delegations this build shipped; none when the root is unavailable.
    pub(super) fn shipped(&self) -> Delegations {
        self.anchors()
            .map(|anchors| anchors.shipped.clone())
            .unwrap_or_default()
    }

    /// Why nothing verifies under this root, when nothing does.
    pub(super) fn unavailable(&self) -> Option<String> {
        self.anchors().err()
    }
}

fn open_publisher(catalog: &Role, envelope: &Envelope) -> Result<Publisher, String> {
    let payload = catalog.open(envelope, PUBLISHER_TYPE, MAX_PUBLISHER_BYTES)?;
    parse_publisher(&payload, envelope)
}

/// Signing, for the tests of this crate and the fuzz targets. The host never signs: the
/// keys here come from seeds anyone can read, and sign only what tests trust.
#[cfg(any(test, feature = "fuzzing"))]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) mod testing {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::{json, Value};

    /// The seeds of the test root in `tests/fixtures/trust`, as
    /// `scripts/extensions/trust.mjs` derives them with `seed:<byte repeated 32 times>`.
    pub(crate) const ROOT_SEEDS: [u8; 2] = [0xa1, 0xa2];
    pub(crate) const CATALOG_SEED: u8 = 0xc1;
    /// The key of the test publisher Example Labs, delegated `com.example-labs`: not
    /// `org.example`, which the tests use for unsigned local apps.
    pub(crate) const EXAMPLE_SEED: u8 = 0xe1;
    /// The key of the package tests' publisher (#562), which this build's test root ships a
    /// delegation of `test.signed` for.
    pub(crate) const TEST_PUBLISHER_SEED: u8 = 0x62;

    pub(crate) fn key(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("a test key")
    }

    pub(crate) fn public(key: &Ed25519KeyPair) -> Vec<u8> {
        key.public_key().as_ref().to_vec()
    }

    pub(crate) fn id(key: &Ed25519KeyPair) -> String {
        key_id(key.public_key().as_ref())
    }

    /// `payload` as a `payload_type` document signed by each of `keys`.
    pub(crate) fn sign(payload_type: &str, payload: &[u8], keys: &[&Ed25519KeyPair]) -> Envelope {
        let message = pae(payload_type, payload);
        Envelope {
            payload_type: payload_type.into(),
            payload: encode_base64(payload),
            signatures: keys
                .iter()
                .map(|key| EnvelopeSignature {
                    keyid: id(key),
                    sig: encode_base64(key.sign(&message).as_ref()),
                })
                .collect(),
        }
    }

    pub(crate) fn sign_json(
        payload_type: &str,
        payload: &Value,
        keys: &[&Ed25519KeyPair],
    ) -> Envelope {
        sign(payload_type, &serde_json::to_vec(payload).unwrap(), keys)
    }

    /// A publisher delegation the test catalog key signs.
    pub(crate) fn publisher(
        id: &str,
        name: &str,
        keys: &[Vec<u8>],
        namespaces: &[&str],
    ) -> Envelope {
        sign_json(
            PUBLISHER_TYPE,
            &json!({
                "_type": "publisher",
                "version": 1,
                "id": id,
                "name": name,
                "keys": keys.iter().map(|key| KeySpec::ed25519(key)).collect::<Vec<_>>(),
                "namespaces": namespaces,
            }),
            &[&key(CATALOG_SEED)],
        )
    }

    /// The published srelens release key, delegated `org.srelens`: the delegation every
    /// release signed before #559 is verified under.
    pub(crate) fn srelens_publisher() -> Envelope {
        publisher(
            "srelens",
            "srelens",
            &[include_bytes!("../../tests/fixtures/trust/srelens-apps.pub").to_vec()],
            &["org.srelens"],
        )
    }

    pub(crate) fn example_publisher() -> Envelope {
        publisher(
            "example",
            "Example Labs",
            &[public(&key(EXAMPLE_SEED))],
            &["com.example-labs"],
        )
    }

    /// The test root, as this crate's tests pin it.
    pub(crate) fn root() -> TrustRoot {
        TrustRoot::open(
            include_bytes!("../../tests/fixtures/trust/root.json"),
            Some(include_bytes!("../../tests/fixtures/trust/publishers.json")),
        )
    }

    /// [`publisher`] at another version.
    pub(crate) fn publisher_at(
        version: u64,
        id: &str,
        name: &str,
        keys: &[Vec<u8>],
        namespaces: &[&str],
    ) -> Envelope {
        sign_json(
            PUBLISHER_TYPE,
            &json!({
                "_type": "publisher",
                "version": version,
                "id": id,
                "name": name,
                "keys": keys.iter().map(|key| KeySpec::ed25519(key)).collect::<Vec<_>>(),
                "namespaces": namespaces,
            }),
            &[&key(CATALOG_SEED)],
        )
    }

    /// The test root, shipping `delegations` instead of the committed ones: a later build.
    pub(crate) fn root_shipping(delegations: &[Envelope]) -> TrustRoot {
        TrustRoot::open(
            include_bytes!("../../tests/fixtures/trust/root.json"),
            Some(&serde_json::to_vec(delegations).unwrap()),
        )
    }

    /// A build whose pinned files are still the key ceremony's placeholders.
    pub(crate) fn placeholder_root() -> TrustRoot {
        TrustRoot::open(br#"{"placeholder": "run the key ceremony"}"#, Some(b"[]"))
    }

    /// The root a release build pins, which this crate's own tests do not: for the checks
    /// that follow the live catalog.
    pub(crate) fn production_root() -> TrustRoot {
        TrustRoot::open(
            include_bytes!("trust/root.json"),
            Some(include_bytes!("trust/publishers.json")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use serde_json::{json, Value};

    fn root_payload(version: u64, root_keys: &[Vec<u8>], threshold: usize) -> Value {
        let catalog = public(&key(CATALOG_SEED));
        let mut keys = serde_json::Map::new();
        for key in root_keys.iter().chain([&catalog]) {
            keys.insert(key_id(key), json!(KeySpec::ed25519(key)));
        }
        json!({
            "_type": "root",
            "version": version,
            "keys": keys,
            "roles": {
                "root": {"keyids": root_keys.iter().map(|k| key_id(k)).collect::<Vec<_>>(), "threshold": threshold},
                "catalog": {"keyids": [key_id(&catalog)], "threshold": 1},
            },
        })
    }

    fn two_of_two() -> (Value, Vec<ring::signature::Ed25519KeyPair>) {
        let keys: Vec<_> = ROOT_SEEDS.iter().map(|seed| key(*seed)).collect();
        let publics: Vec<_> = keys.iter().map(public).collect();
        (root_payload(1, &publics, 2), keys)
    }

    #[test]
    fn a_root_is_trusted_only_at_its_own_threshold() {
        let (payload, keys) = two_of_two();
        let both = sign_json(ROOT_TYPE, &payload, &[&keys[0], &keys[1]]);
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&both).unwrap()).is_ok());
        // One of the two it needs.
        let one = sign_json(ROOT_TYPE, &payload, &[&keys[0]]);
        let refused = TrustRoot::from_signed_root(&serde_json::to_vec(&one).unwrap()).unwrap_err();
        assert!(refused.contains("1 of the 2"), "{refused}");
        // The same key twice is still one key.
        let mut twice = one.clone();
        twice.signatures.push(one.signatures[0].clone());
        let refused =
            TrustRoot::from_signed_root(&serde_json::to_vec(&twice).unwrap()).unwrap_err();
        assert!(refused.contains("1 of the 2"), "{refused}");
        // A key the root does not list counts for nothing, whatever ID it claims.
        let stranger = key(0x99);
        let mut forged = sign_json(ROOT_TYPE, &payload, &[&keys[0], &stranger]);
        forged.signatures[1].keyid = id(&keys[1]);
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&forged).unwrap()).is_err());
        // DSSE's keyid is optional: signatures that leave it out are tried against each key.
        let mut unnamed = serde_json::to_value(&both).unwrap();
        for signature in unnamed["signatures"].as_array_mut().unwrap() {
            signature.as_object_mut().unwrap().remove("keyid");
        }
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&unnamed).unwrap()).is_ok());
        // And each key still counts once, and a key the root does not list for nothing.
        let unnamed_as = |mut envelope: Envelope| {
            for signature in &mut envelope.signatures {
                signature.keyid.clear();
            }
            TrustRoot::from_signed_root(&serde_json::to_vec(&envelope).unwrap())
        };
        let refused = unnamed_as(twice).unwrap_err();
        assert!(refused.contains("1 of the 2"), "{refused}");
        let refused =
            unnamed_as(sign_json(ROOT_TYPE, &payload, &[&keys[0], &stranger])).unwrap_err();
        assert!(refused.contains("1 of the 2"), "{refused}");
    }

    #[test]
    fn a_signature_covers_the_payload_type_and_exact_bytes() {
        let (payload, keys) = two_of_two();
        let signed = sign_json(ROOT_TYPE, &payload, &[&keys[0], &keys[1]]);
        // The same bytes and signatures presented as a catalog.
        let mut retyped = signed.clone();
        retyped.payload_type = CATALOG_TYPE.into();
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&retyped).unwrap()).is_err());
        // One byte changed: here, an added space the JSON parser would not notice.
        let mut bytes = serde_json::to_vec(&payload).unwrap();
        bytes.push(b' ');
        let mut changed = signed.clone();
        changed.payload = encode_base64(&bytes);
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&changed).unwrap()).is_err());
        // Either base64 alphabet, padded or not, decodes to the same signed bytes.
        let raw = serde_json::to_vec(&payload).unwrap();
        let mut url_safe = signed.clone();
        url_safe.payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&raw);
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&url_safe).unwrap()).is_ok());
    }

    #[test]
    fn a_root_must_describe_its_roles_consistently() {
        let (valid, keys) = two_of_two();
        for (pointer, value) in [
            ("/_type", json!("catalog")),
            ("/version", json!(0)),
            ("/roles/root/threshold", json!(0)),
            ("/roles/root/threshold", json!(3)),
            ("/roles/catalog", Value::Null),
        ] {
            let mut payload = valid.clone();
            if value.is_null() {
                payload["roles"].as_object_mut().unwrap().remove("catalog");
            } else {
                *payload.pointer_mut(pointer).unwrap() = value;
            }
            let signed = sign_json(ROOT_TYPE, &payload, &[&keys[0], &keys[1]]);
            assert!(
                TrustRoot::from_signed_root(&serde_json::to_vec(&signed).unwrap()).is_err(),
                "{pointer}"
            );
        }
        // A key listed under an ID that is not its own.
        let mut payload = valid.clone();
        let keys_map = payload["keys"].as_object_mut().unwrap();
        let (first, spec) = keys_map
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), v.clone()))
            .unwrap();
        keys_map.remove(&first);
        keys_map.insert("0".repeat(64), spec);
        let signed = sign_json(ROOT_TYPE, &payload, &[&keys[0], &keys[1]]);
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&signed).unwrap()).is_err());
        // Roles and fields a later root adds are ignored.
        let mut later = valid.clone();
        later["roles"]["timestamp"] = json!({"keyids": [], "threshold": 1});
        later["expires"] = json!("2030-01-01T00:00:00Z");
        let signed = sign_json(ROOT_TYPE, &later, &[&keys[0], &keys[1]]);
        assert!(TrustRoot::from_signed_root(&serde_json::to_vec(&signed).unwrap()).is_ok());
    }

    #[test]
    fn the_test_root_verifies_and_ships_the_srelens_delegation() {
        let root = root();
        assert!(root.unavailable().is_none(), "{root:?}");
        let shipped = root.shipped();
        let srelens = shipped.owner("org.srelens.flux").unwrap();
        assert_eq!(
            srelens.signer(),
            Signer {
                id: "srelens".into(),
                name: "srelens".into()
            }
        );
        assert!(shipped.owner("com.example-labs.app").is_none());
        // The Node script wrote the committed fixture and these helpers sign in Rust; both
        // verify, and name the same key for the same namespace.
        let signed_here = root.publisher(&srelens_publisher()).unwrap();
        assert_eq!(signed_here.keys, srelens.keys);
        assert_eq!(signed_here.namespaces, srelens.namespaces);
    }

    #[test]
    fn a_publisher_is_delegated_only_what_the_catalog_role_signed() {
        let root = root();
        assert!(root.publisher(&example_publisher()).is_ok());
        // Signed by the publisher's own key rather than the catalog role.
        let own = key(EXAMPLE_SEED);
        let payload = decode_base64(&example_publisher().payload, 4096).unwrap();
        let self_signed = sign(PUBLISHER_TYPE, &payload, &[&own]);
        assert!(root
            .publisher(&self_signed)
            .unwrap_err()
            .contains("catalog role"));
        for (field, value) in [
            ("namespaces", json!(["org"])),
            ("namespaces", json!(["Com.Example-Labs"])),
            (
                "namespaces",
                json!(["com.example-labs", "com.example-labs.sub"]),
            ),
            ("namespaces", json!([])),
            ("name", json!("Example\u{202E}Labs")),
            ("name", json!("")),
            ("id", json!("example.labs")),
            ("keys", json!([])),
            ("version", json!(0)),
            ("_type", json!("catalog")),
        ] {
            let mut document: Value = serde_json::from_slice(&payload).unwrap();
            document[field] = value;
            let signed = sign_json(PUBLISHER_TYPE, &document, &[&key(CATALOG_SEED)]);
            assert!(root.publisher(&signed).is_err(), "{field}");
        }
    }

    #[test]
    fn delegations_give_each_app_id_at_most_one_publisher() {
        let root = root();
        let example = root.publisher(&example_publisher()).unwrap();
        let srelens = root.publisher(&srelens_publisher()).unwrap();
        let both = Delegations::new(vec![srelens.clone(), example.clone()]).unwrap();
        assert_eq!(both.owner("com.example-labs.app").unwrap().id, "example");
        assert_eq!(both.owner("org.srelens.argocd").unwrap().id, "srelens");
        assert!(both.owner("org.srelensx.argocd").is_none());
        assert!(
            both.owner("com.example-labs").is_some(),
            "a namespace covers its own name"
        );
        // A second publisher claiming a namespace inside another's.
        let inside = root
            .publisher(&publisher(
                "squatter",
                "Squatter",
                &[public(&key(0x55))],
                &["org.srelens.flux"],
            ))
            .unwrap();
        assert!(Delegations::new(vec![srelens.clone(), inside])
            .unwrap_err()
            .contains("overlaps"));
        // A key shared between publishers.
        let shared = root
            .publisher(&publisher(
                "other",
                "Other",
                &[public(&key(EXAMPLE_SEED))],
                &["com.other"],
            ))
            .unwrap();
        assert!(Delegations::new(vec![example.clone(), shared]).is_err());
        // A catalog moves a shipped publisher on only with a later version of its
        // delegation: at the same version the shipped one stands, so an install and every
        // later load agree on which keys sign for it.
        let rotated_at = |version| {
            Delegations::new(vec![root
                .publisher(&publisher_at(
                    version,
                    "srelens",
                    "srelens",
                    &[public(&key(0x77))],
                    &["org.srelens"],
                ))
                .unwrap()])
            .unwrap()
        };
        let shipped_key = root.shipped().owner("org.srelens.flux").unwrap().keys[0].clone();
        let rotated_key = PublicKey::parse(&KeySpec::ed25519(&public(&key(0x77)))).unwrap();
        let merged = Delegations::merged(&root.shipped(), &rotated_at(1));
        assert_eq!(
            merged.owner("org.srelens.flux").unwrap().keys,
            [shipped_key]
        );
        let merged = Delegations::merged(&root.shipped(), &rotated_at(2));
        // The rotated srelens delegation, and the shipped ones no catalog delegation touches.
        assert_eq!(
            merged
                .publishers()
                .iter()
                .filter(|p| p.id == "srelens")
                .count(),
            1
        );
        assert_eq!(
            merged.owner("org.srelens.flux").unwrap().keys,
            [rotated_key]
        );
        assert!(merged.owner("test.signed.packaged").is_some());
        // Nor can a catalog hand back a delegation the build has already moved past.
        let mut later = root.shipped().publishers()[0].clone();
        later.version = 3;
        let shipped_later = Delegations::new(vec![later]).unwrap();
        let merged = Delegations::merged(&shipped_later, &rotated_at(2));
        assert_eq!(merged.owner("org.srelens.flux").unwrap().version, 3);
        // Nor give a namespace the build ships to another publisher.
        let usurper = Delegations::new(vec![root
            .publisher(&publisher(
                "usurper",
                "Usurper",
                &[public(&key(0x78))],
                &["org.srelens"],
            ))
            .unwrap()])
        .unwrap();
        let merged = Delegations::merged(&root.shipped(), &usurper);
        assert_eq!(merged.owner("org.srelens.flux").unwrap().id, "srelens");
        assert!(merged.publishers().iter().all(|p| p.id != "usurper"));
    }

    #[test]
    fn a_placeholder_root_refuses_everything_and_says_why() {
        let root = TrustRoot::open(br#"{"placeholder": "run the key ceremony"}"#, Some(b"[]"));
        let reason = root.unavailable().unwrap();
        assert!(reason.contains("key ceremony"), "{reason}");
        assert!(root
            .publisher(&example_publisher())
            .unwrap_err()
            .contains("key ceremony"));
        assert!(root.shipped().publishers().is_empty());
    }

    /// The documents a release build pins. Until the key ceremony runs they are
    /// placeholders, and the build trusts no catalog; either way they must never trust a
    /// key whose seed is in this repository.
    #[test]
    fn the_pinned_documents_trust_no_test_key() {
        let production: &[&[u8]] = &[
            include_bytes!("trust/root.json"),
            include_bytes!("trust/publishers.json"),
        ];
        for raw in production {
            for text in readable(raw) {
                for key in test_keys() {
                    assert!(
                        !text.contains(&key),
                        "a pinned document names test key {key}"
                    );
                }
            }
        }
        let pinned = TrustRoot::open(production[0], Some(production[1]));
        if let Some(reason) = pinned.unavailable() {
            assert_eq!(
                reason, NO_PINNED_ROOT,
                "the pinned root is neither a placeholder nor valid"
            );
        }
        // The check sees inside payloads: the test root names its catalog key only there.
        let catalog = id(&key(CATALOG_SEED));
        let test_root = include_bytes!("../../tests/fixtures/trust/root.json");
        assert!(!std::str::from_utf8(test_root).unwrap().contains(&catalog));
        assert!(readable(test_root)
            .iter()
            .any(|text| text.contains(&catalog)));
    }

    /// Every test key's ID and public key in hex: what must never appear in a pinned file.
    fn test_keys() -> Vec<String> {
        ROOT_SEEDS
            .iter()
            .chain([&CATALOG_SEED, &EXAMPLE_SEED, &TEST_PUBLISHER_SEED])
            .flat_map(|seed| {
                let key = key(*seed);
                [id(&key), KeySpec::ed25519(&public(&key)).keyval.public]
            })
            .collect()
    }

    /// A pinned file's text, and the decoded payload of each signed document in it: the
    /// keys a root lists and a delegation holds are inside those payloads, in base64.
    fn readable(raw: &[u8]) -> Vec<String> {
        let envelopes = serde_json::from_slice::<Envelope>(raw)
            .map(|envelope| vec![envelope])
            .or_else(|_| serde_json::from_slice::<Vec<Envelope>>(raw))
            .unwrap_or_default();
        std::iter::once(String::from_utf8_lossy(raw).into_owned())
            .chain(envelopes.iter().filter_map(|envelope| {
                decode_base64(&envelope.payload, MAX_ROOT_BYTES)
                    .ok()
                    .map(|payload| String::from_utf8_lossy(&payload).into_owned())
            }))
            .collect()
    }

    /// Run once the key ceremony has replaced the placeholders: the pinned root verifies,
    /// and its shipped srelens delegation holds the key every published release was
    /// signed with, so apps installed before #559 go on verifying, and the key that
    /// replaced it (#582).
    #[test]
    fn the_pinned_root_verifies_and_delegates_org_srelens_to_the_release_key() {
        let pinned = TrustRoot::open(
            include_bytes!("trust/root.json"),
            Some(include_bytes!("trust/publishers.json")),
        );
        assert!(pinned.unavailable().is_none(), "{pinned:?}");
        let srelens = pinned.shipped();
        let srelens = srelens
            .owner("org.srelens.flux")
            .expect("org.srelens is delegated");
        for release_key in [
            key_id(include_bytes!("../../tests/fixtures/trust/srelens-apps.pub")),
            key_id(include_bytes!("../../tests/fixtures/trust/srelens-publisher.pub")),
        ] {
            assert!(srelens.keys.iter().any(|key| key.id == release_key));
        }
    }
}
