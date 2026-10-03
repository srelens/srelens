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

/// Whether `path` is a file this user could run. On Unix that is the
/// question `which` asks `access(2)`: whether THIS user may execute it. A
/// data file named `kubectl` earlier on `PATH` would otherwise be reported as
/// the install, and so would a file only other users may execute, which an
/// execute bit somewhere in the mode does not rule out. Windows decides by
/// extension, which the candidate names already carry.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // `is_file` first: `access` grants X_OK on a searchable directory.
        // SAFETY: `c_path` is a NUL-terminated string that outlives the call.
        path.is_file() && unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } == 0
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
/// Windows it is `PATHEXT`, and only names ending in an extension a process
/// can be started from are candidates. The bare name never is: Windows cannot
/// run an extensionless file, and npm installs exactly that, a POSIX shell
/// script, beside the `.cmd` shim Windows does run. A name that already
/// carries such an extension is tried as written first, then with each
/// extension appended, which is how Go's `exec.LookPath` (and so client-go's
/// exec plugins) resolves a command.
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

/// The extensions a process can be started from directly, which is how every
/// caller here runs what it finds, and how client-go runs an exec plugin.
/// Windows' own default `PATHEXT` also lists scripts (`.VBS`, `.JS`, …) that
/// only the shell knows to hand to an interpreter.
const STARTABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

fn startable(extension: &str) -> bool {
    STARTABLE
        .iter()
        .any(|startable| startable.eq_ignore_ascii_case(extension))
}

/// The file names `program` may have in one directory, in the order tried.
fn candidates(program: &str, pathext: Option<&str>) -> Vec<String> {
    let Some(pathext) = pathext else {
        return vec![program.to_string()];
    };
    let mut names = Vec::new();
    let written = Path::new(program).extension();
    if written.is_some_and(|ext| startable(&format!(".{}", ext.to_string_lossy()))) {
        names.push(program.to_string());
    }
    names.extend(
        pathext
            .split(';')
            .map(str::trim)
            // Only what can be started. That also drops an empty entry, which
            // a trailing `;` makes and which would put the bare name back.
            .filter(|ext| startable(ext))
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

    /// Windows' own default PATHEXT also names scripts (`.VBS`, `.JS`, …), and
    /// people add `.PS1`. The shell hands those to an interpreter, but a
    /// process started directly, as every caller here starts one, can only
    /// be a `.com`, `.exe`, `.bat` or `.cmd`. A script earlier on PATH must
    /// not hide a program later on it.
    #[test]
    fn a_script_windows_cannot_start_does_not_hide_a_later_program() {
        let found = resolve_on_path(
            "helm",
            &path_var(&["/first", "/second"]),
            Some(".COM;.EXE;.BAT;.CMD;.VBS;.JS;.PS1"),
            fs(&[
                file("/first", "helm.js"),
                file("/first", "helm.ps1"),
                file("/second", "helm.exe"),
            ]),
        );
        assert_eq!(found, Some(file("/second", "helm.exe")));
    }

    #[test]
    fn a_name_written_with_an_extension_windows_cannot_start_is_not_a_program() {
        let found = resolve_on_path(
            "tool.v2",
            &path_var(&["/bin"]),
            WINDOWS,
            fs(&[file("/bin", "tool.v2")]),
        );
        assert_eq!(found, None);
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

    /// An execute bit is not the same as "this user may execute it": a file
    /// mode `001` owned by the user has one, and its owner still cannot run
    /// it. `which` asks `access(2)`, so must we.
    #[cfg(unix)]
    #[test]
    fn a_file_this_user_may_not_execute_is_not_a_program() {
        use std::os::unix::fs::PermissionsExt;
        // Root may execute anything with any execute bit set, so the case
        // does not arise for it; CI runs these as an ordinary user.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        // Executable by "others" only, and owned by us: not by us.
        std::fs::write(first.path().join("kubectl"), "").unwrap();
        std::fs::set_permissions(
            first.path().join("kubectl"),
            std::fs::Permissions::from_mode(0o001),
        )
        .unwrap();
        let real = second.path().join("kubectl");
        std::fs::write(&real, "").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();

        assert_eq!(find_executable("kubectl", &path), Some(real));
    }
}
