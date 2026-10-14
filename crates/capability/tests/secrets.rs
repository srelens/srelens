//! Where a host capability lets the host's secret store (#543) put a secret.
//!
//! The mirror of `settable`: a setting is interpolated into an argument the
//! capability marks, and a secret is injected — by the host, never by the
//! manifest — only into an argument the capability marks as a secret slot.

use serde_json::{json, Value};
use srelens_capability::Capability;

#[test]
fn no_capability_takes_a_secret_unless_it_declares_the_slot() {
    let plain = Capability::read_only("test.read", "read", |_: Value| async { Ok(json!({})) });
    assert!(plain.secret_slots.is_empty(), "no slot by default");
    assert!(!plain.takes_secret("headers"));

    let marked = Capability::read_only("test.read", "read", |_: Value| async { Ok(json!({})) })
        .with_secret_slot("headers");
    assert!(marked.takes_secret("headers"));
    assert!(!marked.takes_secret("url"), "only the argument it names");
    assert!(
        marked.settable.is_empty(),
        "a secret slot does not make the argument settable"
    );
}
