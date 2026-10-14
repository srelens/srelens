//! The TUI finds installed tools the way the platform runs them (#445).
//!
//! Both lookups used to shell out to `which`, which stock Windows does not
//! have, so every tool read as missing there. Each test points `PATH` at a
//! scratch directory holding the tool, through the shared environment guard.

mod common;

use std::path::{Path, PathBuf};

use srectl::ai_config::find_cursor_binary;
use srectl::views::toolbox_view::ToolboxViewState;

/// Put `file` in `dir` as an executable: runnable on Unix, and named however
/// the caller names it.
fn install(dir: &Path, file: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[test]
fn the_toolbox_lists_a_kubectl_that_is_on_path() {
    let bin = tempfile::tempdir().unwrap();
    // `kubectl.exe` on Windows.
    let kubectl = install(
        bin.path(),
        &format!("kubectl{}", std::env::consts::EXE_SUFFIX),
    );
    let mut env = common::env::lock();
    env.set("PATH", bin.path());

    let state = ToolboxViewState::new();

    let tool = &state.tools[0];
    assert_eq!(tool.name, "kubectl");
    assert!(tool.installed, "kubectl is on PATH, so it is installed");
    assert_eq!(tool.path.as_deref().map(Path::new), Some(kubectl.as_path()));
}

#[test]
fn the_cursor_provider_finds_the_cursor_agent_windows_would_run() {
    let bin = tempfile::tempdir().unwrap();
    // What npm installs on Windows: a shell script Windows cannot run, and
    // the `.cmd` shim it does run. Elsewhere the script is the program.
    let script = install(bin.path(), "cursor-agent");
    let expected = if cfg!(windows) {
        install(bin.path(), "cursor-agent.cmd")
    } else {
        script
    };
    let mut env = common::env::lock();
    env.set("PATH", bin.path());

    assert_eq!(
        find_cursor_binary().as_deref().map(Path::new),
        Some(expected.as_path())
    );
}
