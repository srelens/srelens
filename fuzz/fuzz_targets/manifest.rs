//! `Manifest::decode`, `validate` and `parse` on arbitrary input. The properties live in
//! `srelens_plugin_host::fuzzing::manifest`, which `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_plugin_host::fuzzing::manifest(data));
