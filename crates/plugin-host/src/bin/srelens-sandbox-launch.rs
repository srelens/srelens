//! The trusted launcher srelens starts a sidecar through on Linux and macOS.
//! The work is in `srelens_plugin_host::sidecar::sandbox::launch`.

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main() {
    srelens_plugin_host::sidecar::sandbox::launch::main()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("srelens-sandbox-launch is for Linux and macOS");
    std::process::exit(125);
}
