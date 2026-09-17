//! The one way a durable store in this crate replaces its file.
//!
//! The extension inventory, the extension catalog cache and the settings
//! document all save through [`replace`], so a save behaves the same wherever it
//! happens: a reader, a concurrent writer or a crash sees the old file or the new
//! one, never a torn one, and a save that returned `Ok` survives a power loss.

use std::io::{self, Write};
use std::path::Path;

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
/// On failure the temporary file is removed and `path` is left as it was. The
/// parent directory must already exist.
pub(crate) fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = parent_of(path);
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    rename_over(file, path)?;
    sync_directory(parent)
}

/// The directory a rename of `path` changes. A bare file name lives in the
/// current directory.
fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
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

#[cfg(unix)]
fn sync_directory(dir: &Path) -> io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Windows cannot open a directory for flushing without administrator rights;
/// `MOVEFILE_WRITE_THROUGH` in `rename_over` is the durability available there.
#[cfg(not(unix))]
fn sync_directory(_dir: &Path) -> io::Result<()> {
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
    /// opened and synced, and a failure to do so is reported rather than ignored.
    #[cfg(unix)]
    #[test]
    fn the_parent_directory_is_synced_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        sync_directory(dir.path()).unwrap();
        assert!(sync_directory(&dir.path().join("missing")).is_err());
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
