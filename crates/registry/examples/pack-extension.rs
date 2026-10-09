//! Makes a `.srelens-extension` package (#562) from a directory laid out as one
//! (see docs/extensions/packages.md).
//!
//! ```text
//! cargo run -p srelens-registry --example pack-extension -- digests <dir>
//!     writes <dir>/digests.json, the list a publisher signs
//! cargo run -p srelens-registry --example pack-extension -- pack <dir> <out.srelens-extension>
//!     packs <dir>, which holds digests.json and, when it is signed, digests.json.sig
//! ```
//!
//! The signature is the kind a single-file release carries: 64 raw bytes of Ed25519 over
//! the exact bytes of `digests.json`, made with the publisher's key between the two steps.
use srelens_registry::extension_package::{digest_list, pack};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let done = match args.as_slice() {
        ["digests", dir] => digest_list(Path::new(dir)).and_then(|list| {
            let target = Path::new(dir).join("digests.json");
            std::fs::write(&target, list).map_err(|e| format!("{}: {e}", target.display()))
        }),
        ["pack", dir, out] => pack(Path::new(dir))
            .and_then(|archive| std::fs::write(out, archive).map_err(|e| format!("{out}: {e}"))),
        _ => Err(
            "usage: pack-extension digests <dir>\n       pack-extension pack <dir> <out.srelens-extension>"
                .into(),
        ),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            eprintln!("{reason}");
            ExitCode::FAILURE
        }
    }
}
