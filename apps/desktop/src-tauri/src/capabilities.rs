//! The desktop app's capability registry is assembled in the Tauri-free
//! `srelens-registry` crate (so the headless server binary can build the same
//! registry without linking Tauri). This module re-exports it unchanged.
//!
//! `run_tool` is also re-exported: the streaming Toolbox install path in
//! `toolbox.rs` reuses the same krew-bootstrap helper as the registry's
//! `install_krew` capability.

pub use srelens_registry::{
    build_registry, build_registry_with, build_registry_with_paths, default_kubeconfig_paths,
    run_tool,
};
