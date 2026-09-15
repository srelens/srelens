//! Publisher signature verification on arbitrary input: a length byte, the signature, then
//! the manifest. The properties live in `srelens_registry::fuzzing::signed_manifest`, which
//! `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_registry::fuzzing::signed_manifest(data));
