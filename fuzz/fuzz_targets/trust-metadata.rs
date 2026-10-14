//! The trust metadata readers on arbitrary input (#559): a signed root, a publisher
//! delegation and a release signature file. The properties live in
//! `srelens_registry::fuzzing::trust_metadata`, which `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_registry::fuzzing::trust_metadata(data));
