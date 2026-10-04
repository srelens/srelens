//! The example inside the OS sandbox srelens runs sidecars in: proof that a
//! tokio runtime (threads, the reactor, the blocking stdin reader) works
//! under Linux's Landlock and seccomp layers and in a Windows AppContainer.
//! `#[ignore]`: it needs the sandbox, and on Linux the trusted launcher and a
//! delegated cgroup: the scope srelens asks the systemd user manager for, or,
//! without a user session, the directory `SRELENS_SANDBOX_CGROUP_ROOT` names.
//! The `sandbox-conformance` CI job runs it, from the workspace root, with
//! that variable:
//! ```text
//! cargo build -p srelens-plugin-host --bin srelens-sandbox-launch
//! go build -C sdk/examples/hello-world/go -o "$PWD/target/hello-world-go" .
//! SRELENS_SANDBOX_LAUNCHER="$PWD/target/debug/srelens-sandbox-launch" \
//! SRELENS_HELLO_WORLD_GO="$PWD/target/hello-world-go" \
//!   cargo test -p srelens-sidecar-hello-world --test sandboxed --test supervised -- --ignored --test-threads=1
//! ```
//! On Windows, `SRELENS_SANDBOX_LAUNCHER` and `SRELENS_SANDBOX_CGROUP_ROOT`
//! are not needed, and the Go binary ends `.exe`.

mod common;

use srelens_plugin_host::sidecar::{CgroupRoot, Limits, OsSandbox, SandboxConfig};
use std::path::PathBuf;
use std::sync::Arc;

fn sandbox() -> Arc<OsSandbox> {
    let launcher = std::env::var_os("SRELENS_SANDBOX_LAUNCHER").map(PathBuf::from);
    if cfg!(target_os = "linux") {
        assert!(
            launcher.is_some(),
            "set SRELENS_SANDBOX_LAUNCHER to srelens-sandbox-launch"
        );
    }
    Arc::new(OsSandbox::new(SandboxConfig {
        launcher,
        cgroup: std::env::var_os("SRELENS_SANDBOX_CGROUP_ROOT")
            .map_or(CgroupRoot::SystemdScope, |root| {
                CgroupRoot::Delegated(root.into())
            }),
    }))
}

#[tokio::test]
#[ignore = "needs the OS sandbox; run by the sandbox-conformance CI job"]
async fn the_rust_hello_world_serves_srelens_inside_the_os_sandbox() {
    common::whole_life(
        common::program(common::Language::Rust),
        sandbox(),
        Limits::default(),
    )
    .await;
}

#[tokio::test]
#[ignore = "needs the OS sandbox and the Go example built; run by the sandbox-conformance CI job"]
async fn the_go_hello_world_serves_srelens_inside_the_os_sandbox() {
    common::whole_life(
        common::program(common::Language::Go),
        sandbox(),
        Limits::default(),
    )
    .await;
}
