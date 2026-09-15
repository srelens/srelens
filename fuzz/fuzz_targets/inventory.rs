//! The installed-app inventory reader, legacy migration included, on arbitrary file
//! contents. The properties live in `srelens_registry::fuzzing::inventory`, which
//! `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_registry::fuzzing::inventory(data));
