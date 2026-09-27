//! A signed catalog on arbitrary input, verified as a download is (#559): the envelope, the
//! catalog role's signature, the document and its publisher delegations. The properties live
//! in `srelens_registry::fuzzing::signed_catalog`, which `cargo test` holds as well.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| srelens_registry::fuzzing::signed_catalog(data));
