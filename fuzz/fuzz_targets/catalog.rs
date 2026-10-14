//! Extension catalog parsing on arbitrary input. The properties live in
//! `srelens_registry::fuzzing::catalog`, which `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_registry::fuzzing::catalog(data));
