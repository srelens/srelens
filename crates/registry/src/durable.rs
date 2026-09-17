//! The one way a durable store in this crate replaces its file.
//!
//! The extension inventory, the extension catalog cache and the settings
//! document all save through [`replace`], so a save behaves the same wherever it
//! happens: a reader, a concurrent writer or a crash sees the old file or the new
//! one, never a torn one, and a save that returned `Ok` survives a power loss
//! unless a warning said otherwise.
//!
//! The contract callers rely on:
//! - `Err` means the target is unchanged, so reporting a failed save is true.
//! - `Ok` means the new contents are in place. They are durable across a power
//!   loss unless the directory sync after the rename failed, in which case a
//!   warning names the path and the error. That failure is not an error: the
//!   save has happened, and reporting it as failed would make a caller retry a
//!   change that is already applied.

use std::io::{self, Write};
use std::path::Path;

/// A directory handle opened before a change to it, synced after.
#[cfg(unix)]
type Directory = std::fs::File;
#[cfg(not(unix))]
type Directory = UnsyncedDirectory;

/// Atomically and durably replace `path` with `contents`.
///
/// 1. Write `contents` to a private temporary file in `path`'s own directory
///    (owner-only permissions on Unix), so the final rename never crosses a
///    filesystem.
/// 2. Sync the temporary file, so the new bytes are on disk before any name
///    points at them.
/// 3. Rename it over `path`.
/// 4. Make the rename itself durable:
///    - **Unix**: open `path`'s parent directory and `fsync` it. A rename is a
///      change to the directory, and without this a power loss or kernel crash
///      just after the save can bring back the previous file, or no file at all
///      after a first save.
///    - **Windows**: the rename is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING
///      | MOVEFILE_WRITE_THROUGH`, which does not return until the move is
///      flushed. Windows offers no way to flush a directory without
///      administrator rights, so on NTFS this relies on the file system's
///      metadata journal for the rename, which is the platform's limit.
///
/// Everything that can fail for an ordinary reason (permissions, a missing
/// directory, a full disk) happens before or at the rename, including opening
/// the parent directory. Any such failure returns `Err`, removes the temporary
/// file and leaves `path` as it was.
///
/// Once the rename has succeeded the save has happened, so a failure of the
/// directory sync after it is logged as a warning, naming the path and the
/// error, and `replace` still returns `Ok`: the new contents are in place, but
/// that save is not guaranteed to survive a power loss.
///
/// The parent directory must already exist; create it with [`create_dir_all`]
/// so that it, too, survives a power loss.
///
/// On Windows `path` is first made absolute and extended-length (`\\?\`), so
/// the raw Win32 calls here and in tempfile accept a path longer than
/// `MAX_PATH`, as `std::fs` does.
pub(crate) fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    replace_with(path, contents, sync_opened)
}

/// [`replace`], with the post-rename directory sync supplied by the caller so a
/// test can make it fail.
fn replace_with(
    path: &Path,
    contents: &[u8],
    sync: impl FnOnce(Directory) -> io::Result<()>,
) -> io::Result<()> {
    #[cfg(windows)]
    let path = &extended_length(path)?;
    let parent = parent_of(path);
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    let directory = open_directory(parent)?;
    rename_over(file, path)?;
    if let Err(error) = sync(directory) {
        log::warn!(
            "saved {} but could not sync {}: {error}; this save may not survive a power loss",
            path.display(),
            parent.display(),
        );
    }
    Ok(())
}

/// `std::fs::create_dir_all`, but durable: every directory it creates has its
/// own entry synced into its parent, so a store's first save cannot vanish
/// with the directory it was saved in after a power loss.
///
/// Missing ancestors are created one at a time, outermost first. A directory
/// another process created in the meantime is accepted, and its parent is
/// synced all the same, because that process may not have synced it yet.
/// Directories that already existed are left alone.
///
/// The same contract as [`replace`]: the parent is opened before a directory is
/// created, and a failure to sync it afterwards is logged as a warning rather
/// than returned, because the directory exists and a save into it can go on.
pub(crate) fn create_dir_all(dir: &Path) -> io::Result<()> {
    create_dir_all_with(dir, &sync_opened)
}

/// [`create_dir_all`], with the post-create directory sync supplied by the
/// caller so a test can make it fail.
fn create_dir_all_with(dir: &Path, sync: &dyn Fn(Directory) -> io::Result<()>) -> io::Result<()> {
    if dir.as_os_str().is_empty() || dir.is_dir() {
        return Ok(());
    }
    let parent = parent_of(dir);
    create_dir_all_with(parent, sync)?;
    let directory = open_directory(parent)?;
    match std::fs::create_dir(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => {}
        Err(error) => return Err(error),
    }
    if let Err(error) = sync(directory) {
        log::warn!(
            "created {} but could not sync {}: {error}; it may not survive a power loss",
            dir.display(),
            parent.display(),
        );
    }
    Ok(())
}

/// The directory a rename of `path` changes. A bare file name lives in the
/// current directory.
fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// `path` as an absolute, extended-length (`\\?\`) path.
///
/// `std::fs` adds this prefix itself before every Win32 call; `MoveFileExW`,
/// and the `SetFileAttributesW` tempfile calls when a file is kept, take the
/// path as given and fail beyond `MAX_PATH` (260 characters) without it. A
/// verbatim path is passed to the file system untouched, so it must already be
/// absolute, use backslashes and hold no `.` or `..` components, which is
/// exactly what `std::path::absolute` produces.
#[cfg(windows)]
fn extended_length(path: &Path) -> io::Result<std::path::PathBuf> {
    Ok(verbatim(&std::path::absolute(path)?))
}

/// An absolute path with the extended-length prefix for its kind: `\\?\C:\…`
/// for a drive path, `\\?\UNC\server\share\…` for a UNC path. A path that is
/// already verbatim, or a device path (`\\.\…`), is returned unchanged.
#[cfg(windows)]
fn verbatim(absolute: &Path) -> std::path::PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Prefix};

    let prefix: &[u16] = match absolute.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(_) => &[],
            // `\\server\share\…` becomes `\\?\UNC\server\share\…`: drop one leading backslash.
            Prefix::UNC(..) => &[b'U' as u16, b'N' as u16, b'C' as u16],
            _ => return absolute.to_path_buf(),
        },
        _ => return absolute.to_path_buf(),
    };
    let wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let rest = if prefix.is_empty() {
        &wide[..]
    } else {
        &wide[1..]
    };
    let mut out: Vec<u16> = r"\\?\".encode_utf16().collect();
    out.extend_from_slice(prefix);
    out.extend_from_slice(rest);
    OsString::from_wide(&out).into()
}

#[cfg(not(windows))]
fn rename_over(file: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    file.persist(path).map(drop).map_err(|e| e.error)
}

#[cfg(windows)]
fn rename_over(file: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    // Close the handle, then keep the file before moving it. Keeping clears the
    // `FILE_ATTRIBUTE_TEMPORARY` tempfile sets, which a move would otherwise
    // carry onto the saved store, and needs the file still at its temporary name.
    let temp = file.into_temp_path().keep().map_err(|e| e.error)?;
    let (from, to) = (wide(&temp), wide(path));
    // SAFETY: both arguments are NUL-terminated UTF-16 strings that outlive the call.
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = io::Error::last_os_error();
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

/// A handle on a directory, opened so that it can be synced later.
#[cfg(unix)]
fn open_directory(dir: &Path) -> io::Result<Directory> {
    std::fs::File::open(dir)
}

#[cfg(unix)]
fn sync_opened(directory: Directory) -> io::Result<()> {
    directory.sync_all()
}

/// Windows cannot open a directory for flushing without administrator rights;
/// `MOVEFILE_WRITE_THROUGH` in `rename_over` is the durability available there.
#[cfg(not(unix))]
struct UnsyncedDirectory;

#[cfg(not(unix))]
fn open_directory(_dir: &Path) -> io::Result<UnsyncedDirectory> {
    Ok(UnsyncedDirectory)
}

#[cfg(not(unix))]
fn sync_opened(_directory: UnsyncedDirectory) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_a_new_file_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");

        replace(&path, b"first").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(entries(dir.path()), ["store.json"]);
    }

    #[test]
    fn replaces_an_existing_file_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        fs::write(&path, b"a much longer previous version").unwrap();

        replace(&path, b"second").unwrap();
        replace(&path, b"third").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"third");
        assert_eq!(entries(dir.path()), ["store.json"]);
    }

    #[test]
    fn a_failed_replace_keeps_the_target_and_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        // A non-empty directory cannot be replaced by a file on any platform.
        let path = dir.path().join("store.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), b"kept").unwrap();

        assert!(replace(&path, b"new").is_err());

        assert!(path.is_dir());
        assert_eq!(fs::read(path.join("inside")).unwrap(), b"kept");
        assert_eq!(entries(dir.path()), ["store.json"]);
    }

    #[test]
    fn a_missing_parent_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("store.json");

        assert!(replace(&path, b"new").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_bare_file_name_is_replaced_in_the_current_directory() {
        assert_eq!(parent_of(Path::new("store.json")), Path::new("."));
        assert_eq!(parent_of(Path::new("dir/store.json")), Path::new("dir"));
    }

    /// The step that makes the rename durable on Unix: the parent directory is
    /// opened and synced, and a directory that cannot be opened is an error.
    #[cfg(unix)]
    #[test]
    fn the_parent_directory_is_synced_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        sync_opened(open_directory(dir.path()).unwrap()).unwrap();
        assert!(open_directory(&dir.path().join("missing")).is_err());
    }

    /// A directory that can be written but not read cannot be opened for the
    /// sync. That must fail the save before the rename, while the store still
    /// holds its old contents, not after it has already been replaced.
    #[cfg(unix)]
    #[test]
    fn a_parent_that_cannot_be_opened_fails_before_the_rename() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skipped: root reads any directory regardless of its mode");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        fs::create_dir(&store).unwrap();
        let path = store.join("store.json");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&store, fs::Permissions::from_mode(0o300)).unwrap();

        let result = replace(&path, b"new");

        fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(entries(&store), ["store.json"]);
    }

    #[test]
    fn create_dir_all_creates_every_missing_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");

        create_dir_all(&nested).unwrap();
        assert!(nested.is_dir());
        // Existing directories, including the whole chain, are accepted as they are.
        create_dir_all(&nested).unwrap();
        create_dir_all(dir.path()).unwrap();

        replace(&nested.join("store.json"), b"first").unwrap();
        assert_eq!(fs::read(nested.join("store.json")).unwrap(), b"first");
    }

    fn failing_sync(_: Directory) -> io::Result<()> {
        Err(io::Error::other("injected directory sync failure"))
    }

    /// The rename has happened, so a failed directory sync must not report the
    /// save as failed: a caller would show an error for a change that is applied.
    #[test]
    fn a_failed_directory_sync_after_the_rename_still_saves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        fs::write(&path, b"old").unwrap();

        replace_with(&path, b"new", failing_sync).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(entries(dir.path()), ["store.json"]);
    }

    #[test]
    fn a_failed_directory_sync_after_creating_a_directory_still_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");

        create_dir_all_with(&nested, &failing_sync).unwrap();

        assert!(nested.is_dir());
    }

    #[test]
    fn create_dir_all_refuses_a_file_in_the_way() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"not a directory").unwrap();

        assert!(create_dir_all(&dir.path().join("a")).is_err());
        assert!(create_dir_all(&dir.path().join("a").join("b")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefixes_drive_and_unc_paths_and_leaves_verbatim_ones_alone() {
        let cases = [
            (r"C:\Users\me\store.json", r"\\?\C:\Users\me\store.json"),
            (
                r"\\server\share\dir\store.json",
                r"\\?\UNC\server\share\dir\store.json",
            ),
            (r"\\?\C:\Users\me\store.json", r"\\?\C:\Users\me\store.json"),
            (
                r"\\?\UNC\server\share\store.json",
                r"\\?\UNC\server\share\store.json",
            ),
            (r"\\.\pipe\store", r"\\.\pipe\store"),
        ];
        for (input, expected) in cases {
            assert_eq!(verbatim(Path::new(input)), Path::new(expected), "{input}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_normalizes_before_prefixing() {
        let absolute = extended_length(Path::new(r"C:/Users/me/./dir/../store.json")).unwrap();
        assert_eq!(absolute, Path::new(r"\\?\C:\Users\me\store.json"));

        let relative = extended_length(Path::new("store.json")).unwrap();
        let expected = verbatim(&std::env::current_dir().unwrap().join("store.json"));
        assert_eq!(relative, expected);
    }

    /// `std::fs` accepts paths longer than `MAX_PATH`; the raw Win32 calls in
    /// the Windows rename must too, or a store under a deep profile directory
    /// could never be saved.
    #[cfg(windows)]
    #[test]
    fn a_path_longer_than_max_path_is_replaced_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = dir.path().to_path_buf();
        while deep.as_os_str().len() <= 300 {
            deep.push("a-directory-name-of-forty-characters-xx");
        }
        create_dir_all(&deep).unwrap();
        let path = deep.join("store.json");
        assert!(path.as_os_str().len() > 260);

        replace(&path, b"first").unwrap();
        replace(&path, b"second").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(entries(&deep), ["store.json"]);
    }

    #[cfg(windows)]
    #[test]
    fn the_saved_file_is_not_marked_temporary_on_windows() {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");

        replace(&path, b"kept").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().file_attributes() & FILE_ATTRIBUTE_TEMPORARY,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_file_is_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");

        replace(&path, b"secret").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
