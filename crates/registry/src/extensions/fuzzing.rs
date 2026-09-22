//! Test support for the cargo-fuzz targets in `fuzz/` and the property tests below: the
//! catalog, publisher signature and inventory readers on arbitrary bytes. An entry point
//! panics only when a reader breaks its contract, so the fuzzer and `cargo test` hold the
//! same properties. Compiled for this crate's tests and under the `fuzzing` feature; not an
//! API.
use super::{read, saved_form, signing, MAX_INVENTORY_BYTES};
use serde_json::Value;
use std::{collections::BTreeSet, fs};

/// The one release this suite holds a publisher signature for.
const SIGNED: &[u8] = include_bytes!("../../tests/fixtures/argocd-manifest.json");
const SIGNATURE: &[u8] = include_bytes!("../../tests/fixtures/argocd-manifest.sig");

/// `parse_catalog` on arbitrary bytes. It does not panic; a catalog it accepts is within
/// 1 MiB, is schema version 1 and lists each ID once; and it accepts that catalog again in
/// the form the cache writes, unchanged.
pub fn catalog(data: &[u8]) {
    let catalog = match super::catalog::parse_catalog(data) {
        Ok(catalog) => catalog,
        Err(reason) => {
            assert!(!reason.is_empty(), "a refused catalog gives no reason");
            return;
        }
    };
    assert!(
        data.len() <= super::catalog::MAX_CATALOG,
        "a {}-byte catalog was accepted",
        data.len()
    );
    let value = serde_json::to_value(&catalog).expect("an accepted catalog serializes");
    assert_eq!(value["schemaVersion"], 1);
    let entries = value["extensions"]
        .as_array()
        .expect("a catalog lists extensions");
    let ids: BTreeSet<_> = entries
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .collect();
    assert_eq!(ids.len(), entries.len(), "a catalog lists an ID twice");
    // The cache holds the catalog as serialized here, and parses it again on every load.
    let cached = serde_json::to_vec(&catalog).expect("an accepted catalog serializes");
    let again = super::catalog::parse_catalog(&cached).unwrap_or_else(|reason| {
        panic!("the cached form of an accepted catalog is refused: {reason}")
    });
    assert_eq!(
        serde_json::to_value(&again).expect("a catalog serializes"),
        value,
        "a catalog changed in the cache"
    );
}

/// Publisher signature checks on arbitrary input, read as one byte giving the signature's
/// length, the signature, then the manifest. Neither `verify` nor `verify_for` panics, and
/// neither accepts anything but the release this suite holds the signature for: a fuzzer
/// cannot sign, so anything else it got accepted would be a forgery.
pub fn signed_manifest(data: &[u8]) {
    let (length, rest) = match data.split_first() {
        Some((length, rest)) => (usize::from(*length), rest),
        None => (0, data),
    };
    let (signature, raw) = rest.split_at(length.min(rest.len()));
    let published = raw == SIGNED && signature == SIGNATURE;
    let verified = signing::verify(raw, signature);
    match &verified {
        Ok(()) => assert!(published, "a signature verified over bytes nobody signed"),
        Err(reason) => assert!(!reason.is_empty(), "a refused signature gives no reason"),
    }
    // Installation checks the signature against the manifest's ID even when the manifest
    // breaks its rules, so `verify_for` sees manifests `verify` never gets past parsing.
    let id = serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|value| value.get("id")?.as_str().map(str::to_owned));
    if let Some(id) = id {
        match signing::verify_for(&id, raw, signature) {
            Ok(()) => assert!(
                published,
                "a signature verified for {id} over bytes nobody signed"
            ),
            Err(reason) => {
                assert!(!reason.is_empty(), "a refused signature gives no reason");
                assert!(verified.is_err(), "verify accepted what verify_for refused");
            }
        }
    }
}

/// `read` on arbitrary file contents, legacy migration included. It does not panic; an
/// inventory it loads is within 1 MiB, lists each app once and has disabled every app it
/// quarantined; and what `write` would save of it loads back unchanged.
pub fn inventory(data: &[u8]) {
    thread_local! {
        static DIRECTORY: tempfile::TempDir =
            tempfile::tempdir().expect("a temporary directory for inventories");
    }
    let path = DIRECTORY.with(|directory| directory.path().join("extensions.json"));
    fs::write(&path, data).expect("write the inventory under test");
    let state = match read(&path) {
        Ok(state) => state,
        Err(reason) => {
            assert!(!reason.is_empty(), "a refused inventory gives no reason");
            return;
        }
    };
    assert!(
        data.len() <= MAX_INVENTORY_BYTES,
        "a {}-byte inventory loaded",
        data.len()
    );
    assert_eq!(state.schema_version, 1);
    let ids: BTreeSet<_> = state
        .plugins
        .iter()
        .map(|plugin| plugin.manifest.id.as_str())
        .collect();
    assert_eq!(
        ids.len(),
        state.plugins.len(),
        "an inventory lists an app twice"
    );
    for plugin in &state.plugins {
        if let Some(reason) = &plugin.quarantined {
            assert!(
                !plugin.enabled,
                "{} is quarantined but enabled",
                plugin.manifest.id
            );
            assert!(
                !reason.is_empty(),
                "{} is quarantined without a reason",
                plugin.manifest.id
            );
        }
    }
    let saved = saved_form(&state).expect("a loaded inventory serializes");
    // `write` refuses a saved form over the limit, which pretty-printing a compact file can
    // reach; the app then reports the failed save rather than losing the inventory.
    if saved.len() > MAX_INVENTORY_BYTES {
        return;
    }
    // What `write` saves, without its sync and rename.
    fs::write(&path, &saved).expect("save the inventory under test");
    let again =
        read(&path).unwrap_or_else(|reason| panic!("a saved inventory does not load: {reason}"));
    assert_eq!(
        serde_json::to_value(&again).expect("an inventory serializes"),
        serde_json::to_value(&state).expect("an inventory serializes"),
        "an inventory changed across a save"
    );
}

#[cfg(test)]
mod tests {
    use super::super::{catalog::parse_catalog, catalog::MAX_CATALOG, Inventory};
    use super::*;
    use proptest::{collection::vec, prelude::*, sample::Index, test_runner::RngSeed};
    use srelens_plugin_host::fuzzing::mutate;

    const CATALOG: &[u8] = include_bytes!("../../tests/fixtures/extension-catalog.json");
    const INVENTORY: &[u8] = include_bytes!("../../tests/fixtures/extension-inventory.json");
    const LEGACY_INVENTORY: &[u8] =
        include_bytes!("../../tests/fixtures/extension-inventory-legacy.json");

    /// The same cases on every run, so a red build points at a change rather than a lucky
    /// draw. `PROPTEST_RNG_SEED` explores others; the fuzz workflow keeps exploring.
    fn config() -> ProptestConfig {
        let mut config = ProptestConfig::default();
        if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
            config.rng_seed = RngSeed::Fixed(580);
        }
        config
    }

    /// The input [`signed_manifest`] reads for this signature and manifest.
    fn signed_input(signature: &[u8], raw: &[u8]) -> Vec<u8> {
        let length = u8::try_from(signature.len()).expect("a signature under 256 bytes");
        [&[length][..], signature, raw].concat()
    }

    fn edited(seed: &[u8], choices: &[u8], pretty: bool) -> Vec<u8> {
        let document = mutate(&serde_json::from_slice(seed).unwrap(), choices);
        if pretty {
            serde_json::to_vec_pretty(&document)
        } else {
            serde_json::to_vec(&document)
        }
        .unwrap()
    }

    fn read_bytes(data: &[u8]) -> Result<Inventory, String> {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("extensions.json");
        fs::write(&path, data).unwrap();
        read(&path)
    }

    /// `seed` without its trailing newline, padded with spaces to exactly `len` bytes.
    fn padded(seed: &[u8], len: usize) -> Vec<u8> {
        let mut data = seed.trim_ascii_end().to_vec();
        data.resize(len, b' ');
        data
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn arbitrary_bytes_are_read_or_refused(data in vec(any::<u8>(), 0..1024)) {
            catalog(&data);
            signed_manifest(&data);
            inventory(&data);
        }

        #[test]
        fn catalogs_a_few_edits_from_the_fixture_are_accepted_or_refused(
            choices in vec(any::<u8>(), 0..96),
            pretty in any::<bool>(),
        ) {
            catalog(&edited(CATALOG, &choices, pretty));
        }

        #[test]
        fn edited_signed_manifests_and_signatures_are_refused(
            edit_manifest in any::<bool>(),
            choices in vec(any::<u8>(), 1..96),
            flips in vec((any::<Index>(), 1..=u8::MAX, any::<bool>()), 0..4),
            length in prop_oneof![Just(SIGNATURE.len()), 0..=255usize],
        ) {
            let mut raw = if edit_manifest {
                edited(SIGNED, &choices, true)
            } else {
                SIGNED.to_vec()
            };
            let mut signature = SIGNATURE.to_vec();
            signature.resize(length, 0);
            for (at, mask, in_signature) in flips {
                let bytes = if in_signature && !signature.is_empty() {
                    &mut signature
                } else {
                    &mut raw
                };
                let index = at.index(bytes.len());
                bytes[index] ^= mask;
            }
            signed_manifest(&signed_input(&signature, &raw));
        }

        #[test]
        fn inventories_a_few_edits_from_the_fixtures_load_or_are_refused(
            legacy in any::<bool>(),
            choices in vec(any::<u8>(), 0..96),
            pretty in any::<bool>(),
        ) {
            let seed = if legacy { LEGACY_INVENTORY } else { INVENTORY };
            inventory(&edited(seed, &choices, pretty));
        }
    }

    #[test]
    fn historic_fixtures_keep_authentic_signatures_but_require_a_retired_api() {
        assert!(parse_catalog(CATALOG).is_ok());
        catalog(CATALOG);
        assert!(signing::verify_for("org.srelens.argocd", SIGNED, SIGNATURE).is_ok());
        assert!(signing::verify(SIGNED, SIGNATURE).unwrap_err().contains("requires API ^0.1"));
        signed_manifest(&signed_input(SIGNATURE, SIGNED));

        let state = read_bytes(INVENTORY).unwrap();
        assert_eq!(state.plugins.len(), 2);
        assert!(state.plugins[0].signature_proof.is_some());
        assert!(state
            .plugins
            .iter()
            .all(|plugin| !plugin.enabled && plugin.quarantined.as_deref().is_some_and(|reason| reason.contains("requires API ^0.1"))));
        inventory(INVENTORY);

        // Migrated: the retired archive is dropped, and with developer mode off nothing that
        // was enabled stays enabled.
        let legacy = read_bytes(LEGACY_INVENTORY).unwrap();
        assert_eq!(legacy.plugins.len(), 1);
        assert!(legacy
            .plugins
            .iter()
            .all(|plugin| !plugin.enabled && plugin.quarantined.as_deref().is_some_and(|reason| reason.contains("requires API ^0.1"))));
        inventory(LEGACY_INVENTORY);
    }

    #[test]
    fn a_setting_survives_a_save_exactly() {
        // serde_json reads this double back one step off unless it parses floats exactly, so
        // a setting holding it changed on every save.
        let fixture = std::str::from_utf8(INVENTORY).unwrap();
        let data = fixture.replacen(
            "\"namespace\": \"argocd\"",
            "\"namespace\": \"argocd\", \"ratio\": 1.0715660391465826e-75",
            1,
        );
        assert_ne!(data, fixture);
        inventory(data.as_bytes());
    }

    #[test]
    fn readers_hold_their_size_limits_to_the_byte() {
        assert!(parse_catalog(&padded(CATALOG, MAX_CATALOG)).is_ok());
        assert!(parse_catalog(&padded(CATALOG, MAX_CATALOG + 1)).is_err());
        catalog(&padded(CATALOG, MAX_CATALOG));
        catalog(&padded(CATALOG, MAX_CATALOG + 1));

        assert!(read_bytes(&padded(INVENTORY, MAX_INVENTORY_BYTES)).is_ok());
        assert!(read_bytes(&padded(INVENTORY, MAX_INVENTORY_BYTES + 1)).is_err());
        inventory(&padded(INVENTORY, MAX_INVENTORY_BYTES));
        inventory(&padded(INVENTORY, MAX_INVENTORY_BYTES + 1));
    }
}
