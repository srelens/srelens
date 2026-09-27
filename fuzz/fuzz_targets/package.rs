//! The `.srelens-extension` package reader (#562) on arbitrary input: a mode byte, then an
//! uncompressed tar (odd) or the package file as it is (even). The properties live in
//! `srelens_registry::fuzzing::package`, which `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_registry::fuzzing::package(data));
