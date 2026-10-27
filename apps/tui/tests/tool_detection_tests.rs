//! The TUI finds installed tools the way the platform runs them (#445).
//!
//! Both lookups used to shell out to `which`, which stock Windows does not
//! have, so every tool read as missing there. Each test points `PATH` at a
//! scratch directory holding the tool, through the shared environment guard.

mod common;

use std::path::{Path, PathBuf};

use srectl::ai_config::{find_agent, find_agent_in};
use srectl::views::toolbox_view::ToolboxViewState;
use srelens_agent::adapter::AgentKind;
use srelens_kube::toolbox::SearchPaths;

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
fn the_cursor_agent_is_found_as_windows_would_run_it() {
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

    assert_eq!(find_agent(AgentKind::Cursor), Some(expected));
    assert_eq!(
        find_agent(AgentKind::Srelens),
        None,
        "srelens's own agent has no binary"
    );
}

/// Claude Code's program, named as this platform runs an npm install of it.
const CLAUDE: &str = if cfg!(windows) {
    "claude.cmd"
} else {
    "claude"
};

#[test]
fn an_agent_is_looked_for_only_where_the_search_paths_say() {
    let on_path = tempfile::tempdir().unwrap();
    let claude = install(on_path.path(), CLAUDE);
    let mut env = common::env::lock();
    env.set("PATH", on_path.path());
    env.set("HOME", on_path.path());
    assert_eq!(
        find_agent(AgentKind::Claude),
        Some(claude),
        "the process PATH has one"
    );

    let empty = tempfile::tempdir().unwrap();
    let only_empty = SearchPaths {
        app_path: empty.path().to_string_lossy().into_owned(),
        system_path: String::new(),
    };
    assert_eq!(find_agent_in(AgentKind::Claude, &only_empty), None);
}

/// A search directory that is not absolute is what `HOME/.srelens/bin`
/// becomes when `HOME` is unset (usual on Windows), and what an empty `PATH`
/// entry is. It would be read against the working directory, where a cloned
/// repository could have put a `claude.cmd` of its own.
#[test]
fn a_search_dir_that_is_not_absolute_is_never_searched() {
    let cwd = std::env::current_dir().unwrap();
    let planted = tempfile::Builder::new()
        .prefix(".srectl-planted-")
        .tempdir_in(&cwd)
        .unwrap();
    let claude = install(planted.path(), CLAUDE);
    let relative = planted
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(Path::new(&relative).is_relative());

    let absolute = SearchPaths {
        app_path: planted.path().to_string_lossy().into_owned(),
        system_path: String::new(),
    };
    assert_eq!(
        find_agent_in(AgentKind::Claude, &absolute),
        Some(claude),
        "the same directory, named absolutely, is searched"
    );
    for paths in [
        SearchPaths {
            app_path: relative.clone(),
            system_path: String::new(),
        },
        SearchPaths {
            app_path: String::new(),
            system_path: relative.clone(),
        },
    ] {
        assert_eq!(
            find_agent_in(AgentKind::Claude, &paths),
            None,
            "app_path {:?}, system_path {:?}",
            paths.app_path,
            paths.system_path
        );
    }
}
