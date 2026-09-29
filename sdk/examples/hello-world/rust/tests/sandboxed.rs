//! The example inside the OS sandbox srelens runs sidecars in: proof that a
//! tokio runtime (threads, the reactor, the blocking stdin reader) works
//! under Linux's Landlock and seccomp layers and in a Windows AppContainer.
//! `#[ignore]`: it needs the sandbox, and on Linux the trusted launcher and a
//! delegated cgroup. The `sandbox-conformance` CI job runs it:
//! ```text
//! SRELENS_SANDBOX_LAUNCHER=target/debug/srelens-sandbox-launch \
//! SRELENS_SANDBOX_CGROUP_ROOT=/sys/fs/cgroup/<delegated> \
//!   cargo test -p srelens-sidecar-hello-world --test sandboxed -- --ignored --test-threads=1
//! ```

mod common;

use srelens_plugin_host::sidecar::{Limits, OsSandbox, SandboxConfig};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
#[ignore = "needs the OS sandbox; run by the sandbox-conformance CI job"]
async fn the_hello_world_sidecar_serves_srelens_inside_the_os_sandbox() {
    let launcher = std::env::var_os("SRELENS_SANDBOX_LAUNCHER").map(PathBuf::from);
    if cfg!(target_os = "linux") {
        assert!(
            launcher.is_some(),
            "set SRELENS_SANDBOX_LAUNCHER to srelens-sandbox-launch"
        );
    }
    let sandbox = OsSandbox::new(SandboxConfig {
        launcher,
        cgroup_root: std::env::var_os("SRELENS_SANDBOX_CGROUP_ROOT").map(PathBuf::from),
    });
    common::whole_life(Arc::new(sandbox), Limits::default()).await;
}
