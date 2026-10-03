//! Finding a program on a `PATH` the way the operating system would run it.
//!
//! Every "is this tool installed, and where" question goes through here: Helm
//! operations, the toolbox diagnosis, and the TUI's toolbox screen and Cursor
//! lookup. They used to answer it two ways, joining the bare name onto each
//! `PATH` entry or shelling out to `which`, and both were Unix-only: stock
//! Windows has no `which`, and the file there is `kubectl.exe` (#445).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The extensions Windows tries when `PATHEXT` is not set.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Find `program` on this process's `PATH`, named as this platform names
/// programs.
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    find_executable(program, std::env::var_os("PATH").unwrap_or_default())
}

/// Find `program` in the directories of the `PATH`-style `path_var`, on the
/// real filesystem, named as this platform names programs.
pub fn find_executable(program: &str, path_var: impl AsRef<OsStr>) -> Option<PathBuf> {
    resolve_on_path(
        program,
        path_var,
        platform_pathext().as_deref(),
        is_executable,
    )
}

/// Whether `path` is a file this platform would run. On Unix that takes an
/// execute bit, as `which` requires: a data file that happens to be named
/// `kubectl` earlier on `PATH` would otherwise be reported as the install.
/// Windows decides by extension, which the candidate names already carry.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Find `program` on the `PATH`-style `path_var`, as `pathext` says programs
/// are named. The first directory holding a candidate wins, so `PATH` order
/// beats extension order, as it does for the Windows shell.
///
/// `pathext` is `None` off Windows, where a program is its bare name. On
/// Windows it is `PATHEXT`, and the bare name is never a candidate: Windows
/// cannot run an extensionless file, and npm installs exactly that, a POSIX
/// shell script, beside the `.cmd` shim Windows does run. A name that already
/// carries an extension is tried as written first, then with each extension
/// appended, which is how Go's `exec.LookPath` (and so client-go's exec
/// plugins) resolves a command.
///
/// Everything is passed in, so this is unit-testable on any host, Windows
/// rules included, without touching the disk.
pub fn resolve_on_path(
    program: &str,
    path_var: impl AsRef<OsStr>,
    pathext: Option<&str>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let names = candidates(program, pathext);
    std::env::split_paths(&path_var)
        .find_map(|dir| names.iter().map(|name| dir.join(name)).find(|c| is_file(c)))
}

/// The file names `program` may have in one directory, in the order tried.
fn candidates(program: &str, pathext: Option<&str>) -> Vec<String> {
    let Some(pathext) = pathext else {
        return vec![program.to_string()];
    };
    let mut names = Vec::new();
    if Path::new(program).extension().is_some() {
        names.push(program.to_string());
    }
    names.extend(
        pathext
            .split(';')
            .map(str::trim)
            // A trailing `;` read as an empty extension would put the bare
            // name back.
            .filter(|ext| !ext.is_empty())
            // `PATHEXT` spells them `.EXE` and the files are `kubectl.exe`.
            // Windows matches either; the lower-case one is what people see.
            .map(|ext| format!("{program}{}", ext.to_ascii_lowercase())),
    );
    names
}

/// The `PATHEXT` this platform resolves programs with: Windows' own, or
/// `None` elsewhere.
pub fn platform_pathext() -> Option<String> {
    cfg!(windows).then(|| windows_pathext(std::env::var("PATHEXT").ok()))
}

/// `PATHEXT`, or Windows' default when it is unset or blank. A blank one
/// taken at its word would make every program unfindable.
fn windows_pathext(var: Option<String>) -> String {
    var.filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PATHEXT.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows' own default, spelled the way Windows spells it.
    const WINDOWS: Option<&str> = Some(".COM;.EXE;.BAT;.CMD");

    /// A `PATH` value in this host's syntax (`:` or `;`), so the same test
    /// reads the same way on every platform.
    fn path_var(dirs: &[&str]) -> String {
        std::env::join_paths(dirs).unwrap().into_string().unwrap()
    }

    /// `dir/name`, joined the way this host joins paths.
    fn file(dir: &str, name: &str) -> PathBuf {
        Path::new(dir).join(name)
    }

    /// A fake filesystem in which exactly `files` exist.
    fn fs(files: &[PathBuf]) -> impl Fn(&Path) -> bool {
        let files = files.to_vec();
        move |p: &Path| files.iter().any(|f| f == p)
    }

    #[test]
    fn resolves_program_in_second_path_dir() {
        let found = resolve_on_path(
            "helm",
            &path_var(&["/nope", "/bin"]),
            None,
            fs(&[file("/bin", "helm")]),
        );
        assert_eq!(found, Some(file("/bin", "helm")));
    }

    #[test]
    fn resolve_returns_none_when_absent() {
        assert_eq!(
            resolve_on_path("helm", &path_var(&["/a", "/b"]), None, |_| false),
            None
        );
    }

    #[test]
    fn a_bare_name_resolves_through_pathext_as_windows_runs_it() {
        let found = resolve_on_path(
            "helm",
            &path_var(&["/bin"]),
            WINDOWS,
            fs(&[file("/bin", "helm.exe")]),
        );
        assert_eq!(found, Some(file("/bin", "helm.exe")));
    }

    #[test]
    fn an_extensionless_file_is_not_what_windows_would_run() {
        // npm installs a POSIX shell script with no extension beside the
        // `.cmd` shim. Windows cannot run the script; it runs the shim.
        let found = resolve_on_path(
            "cursor-agent",
            &path_var(&["/npm"]),
            WINDOWS,
            fs(&[
                file("/npm", "cursor-agent"),
                file("/npm", "cursor-agent.cmd"),
            ]),
        );
        assert_eq!(found, Some(file("/npm", "cursor-agent.cmd")));
    }

    #[test]
    fn pathext_order_decides_between_extensions_in_one_directory() {
        let found = resolve_on_path(
            "helm",
            &path_var(&["/bin"]),
            WINDOWS,
            fs(&[file("/bin", "helm.cmd"), file("/bin", "helm.exe")]),
        );
        assert_eq!(found, Some(file("/bin", "helm.exe")));
    }

    #[test]
    fn an_earlier_directory_wins_over_a_preferred_extension() {
        let found = resolve_on_path(
            "helm",
            &path_var(&["/first", "/second"]),
            WINDOWS,
            fs(&[file("/first", "helm.cmd"), file("/second", "helm.exe")]),
        );
        assert_eq!(found, Some(file("/first", "helm.cmd")));
    }

    #[test]
    fn a_name_with_an_extension_is_tried_as_written() {
        let found = resolve_on_path(
            "helm.exe",
            &path_var(&["/bin"]),
            WINDOWS,
            fs(&[file("/bin", "helm.exe")]),
        );
        assert_eq!(found, Some(file("/bin", "helm.exe")));
    }

    #[test]
    fn a_dot_in_the_name_does_not_stop_pathext_being_appended() {
        let found = resolve_on_path(
            "tool.v2",
            &path_var(&["/bin"]),
            WINDOWS,
            fs(&[file("/bin", "tool.v2.exe")]),
        );
        assert_eq!(found, Some(file("/bin", "tool.v2.exe")));
    }

    #[test]
    fn an_empty_pathext_entry_does_not_bring_back_the_bare_name() {
        // A trailing `;` is an easy edit to make by hand. Read as an empty
        // extension it would make the extensionless file a candidate again.
        let found = resolve_on_path(
            "helm",
            &path_var(&["/bin"]),
            Some(".EXE;"),
            fs(&[file("/bin", "helm")]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn windows_falls_back_to_its_default_pathext_when_none_is_set() {
        assert_eq!(windows_pathext(None), ".COM;.EXE;.BAT;.CMD");
        assert_eq!(windows_pathext(Some("  ".into())), ".COM;.EXE;.BAT;.CMD");
        assert_eq!(windows_pathext(Some(".EXE;.PS1".into())), ".EXE;.PS1");
    }

    // Unix only: Windows has no execute bit to leave off.
    #[cfg(unix)]
    #[test]
    fn a_file_without_the_execute_bit_is_not_a_program() {
        use std::os::unix::fs::PermissionsExt;
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        // A plain data file that happens to be named `kubectl`, earlier on
        // PATH than the real one. `which` skips it; so must we.
        std::fs::write(first.path().join("kubectl"), "").unwrap();
        std::fs::set_permissions(
            first.path().join("kubectl"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let real = second.path().join("kubectl");
        std::fs::write(&real, "").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();

        assert_eq!(find_executable("kubectl", &path), Some(real));
    }
}
