//! A sidecar's data directory (#573): the one path it may write, private to
//! its app and limited in size.
//!
//! - **One per app**, `<root>/<digest>`: a digest of the exact app ID, so two
//!   IDs that differ only in case never share one on a case-insensitive
//!   filesystem, and the name stays short on Windows. The host chooses the root.
//! - **Private.** Created owner-only (`0700`) on Linux and macOS, and refused
//!   by the supervisor if other users can read it or it is a symbolic link,
//!   which the sandbox would resolve and grant. On Windows it inherits the
//!   user's own directory's ACL, and the sandbox grants it to the app's
//!   AppContainer.
//! - **Limited.** [`check`] holds it to [`Limits::data_bytes`] and
//!   [`Limits::data_entries`]; the supervisor runs it before each start and
//!   every [`Policy::data_check_interval`](super::Policy::data_check_interval)
//!   while the sidecar runs. On Linux and macOS the launcher also caps each
//!   file at the byte limit (`RLIMIT_FSIZE`), which the kernel enforces.
//! - **Gone with the app.** [`prune`] removes the directories of apps no longer
//!   installed, so a later app with the same ID does not inherit what this one
//!   kept.
//!
//! Nothing here follows a symbolic link it finds: not measuring, clearing or
//! pruning. A measurement is a walk, not a snapshot: a sidecar that renames or
//! swaps entries while it is walked can make it count what a swapped-in link
//! points to, or miss a subtree, so one that races the walk on purpose can
//! keep its directory past the limit. Each file is still capped on Linux and
//! macOS. Closing that race is #744's (`docs/extensions/sidecar-protocol.md`,
//! "Data directory").

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use super::Limits;

/// The directory name for `app_id`: the first 16 bytes of its SHA-256, in
/// lowercase hex.
pub fn directory_name(app_id: &str) -> String {
    Sha256::digest(app_id.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `name` is one [`directory_name`] gives.
fn is_directory_name(name: &str) -> bool {
    name.len() == 32
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One app's data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    path: PathBuf,
}

impl DataDir {
    /// The data directory of `app_id` under `root`, created owner-only (with
    /// `root`, if that is missing too) when it does not exist yet. An existing
    /// one is kept, with what it holds, and narrowed to owner-only. A symbolic
    /// link in its place is refused.
    pub fn for_app(root: &Path, app_id: &str) -> io::Result<DataDir> {
        create_private(root, true)?;
        let path = root.join(directory_name(app_id));
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(io::Error::other(format!(
                    "the app's data directory {} is a symbolic link, which srelens does not follow",
                    path.display()
                )))
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(io::Error::other(format!(
                    "the app's data directory {} is not a directory",
                    path.display()
                )))
            }
            Ok(_) => narrow(&path)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => create_private(&path, false)?,
            Err(e) => return Err(e),
        }
        Ok(DataDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Remove everything in it, and keep the directory: what a person does to
    /// an app refused for a full data directory.
    pub fn clear(&self) -> io::Result<()> {
        // The sidecar may have taken its own access away from the directory.
        narrow(&self.path)?;
        let mut failed = None;
        for entry in std::fs::read_dir(&self.path)? {
            if let Err(e) = remove(&entry?.path()) {
                failed.get_or_insert(e);
            }
        }
        failed.map_or(Ok(()), Err)
    }
}

/// Create `path` owner-only; with `parents`, any missing parent too. An
/// existing directory is left as it is.
fn create_private(path: &Path, parents: bool) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(parents);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(path) {
        Err(e) if !parents && e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        other => other,
    }
}

/// Make an existing directory owner-only.
fn narrow(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Remove `path` without following it: a link goes, its target stays.
fn remove(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return std::fs::remove_file(path).or_else(|e| {
            // On Windows a link to a directory is removed as a directory.
            if cfg!(windows) && meta.file_type().is_symlink() {
                std::fs::remove_dir(path)
            } else {
                Err(e)
            }
        });
    }
    // Does not follow links inside it either (CVE-2022-21658 fixed this in
    // the standard library).
    if std::fs::remove_dir_all(path).is_ok() {
        return Ok(());
    }
    // A sidecar owns what it writes, and may have taken its own access away
    // from a directory in it. Give it back, and try again.
    unlock(path);
    std::fs::remove_dir_all(path)
}

/// Give the owner full access to `dir` and every directory below it, and
/// clear the read-only attribute on Windows, following no link. Best effort:
/// what cannot be unlocked is left for the removal to report.
fn unlock(dir: &Path) {
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Checked again right before, which narrows, and does not close,
            // the moment in which a link put in its place would be followed.
            if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                pending.push(entry.path());
                continue;
            }
            #[cfg(windows)]
            if meta.permissions().readonly() && !meta.file_type().is_symlink() {
                let mut writable = meta.permissions();
                writable.set_readonly(false);
                let _ = std::fs::set_permissions(entry.path(), writable);
            }
        }
    }
}

/// Remove the data directory of every app under `root` that `installed` does
/// not name. Anything else under `root`, whose name this module does not give,
/// is left alone. A missing `root` is nothing to do.
pub fn prune(root: &Path, installed: &[&str]) -> io::Result<()> {
    let keep: HashSet<String> = installed.iter().map(|id| directory_name(id)).collect();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    // One that cannot be removed does not keep the others: every stale one
    // is tried, and the first failure is reported after.
    let mut failed = None;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if is_directory_name(name) && !keep.contains(name) {
            if let Err(e) = remove(&entry.path()) {
                failed.get_or_insert(e);
            }
        }
    }
    failed.map_or(Ok(()), Err)
}

/// What a data directory holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Files, directories and links below it, not counting itself. Past the
    /// `max_entries` it was measured with, one more than that: the walk stops.
    pub entries: u64,
    /// For each file, the larger of its length and what the filesystem
    /// allocated to it, and each hard-linked file once; for a directory or a
    /// link, what was allocated to it.
    pub bytes: u64,
}

/// Measure `path` without following links, stopping once it has seen more
/// than `max_entries`: a directory of a million files costs the host no more
/// than one just past the limit.
pub fn measure(path: &Path, max_entries: u64) -> io::Result<Usage> {
    let mut usage = Usage {
        entries: 0,
        bytes: 0,
    };
    #[cfg(unix)]
    let mut linked: HashSet<(u64, u64)> = HashSet::new();
    let mut pending = vec![path.to_owned()];
    while let Some(dir) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // A subdirectory the sidecar removed, or replaced with a file,
            // after it was listed; the directory itself missing is an error.
            Err(e)
                if dir != path
                    && matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
            {
                continue
            }
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let meta = match std::fs::symlink_metadata(entry.path()) {
                Ok(meta) => meta,
                // Removed between the listing and now.
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            usage.entries += 1;
            if usage.entries > max_entries {
                return Ok(usage);
            }
            #[cfg(unix)]
            let (allocated, first) = {
                use std::os::unix::fs::MetadataExt;
                let first =
                    meta.nlink() < 2 || meta.is_dir() || linked.insert((meta.dev(), meta.ino()));
                (meta.blocks().saturating_mul(512), first)
            };
            #[cfg(not(unix))]
            let (allocated, first) = (0u64, true);
            if !first {
                continue;
            }
            let bytes = if meta.is_file() {
                meta.len().max(allocated)
            } else {
                allocated
            };
            usage.bytes = usage.bytes.saturating_add(bytes);
            if meta.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(usage)
}

/// Whether `path` may be a sidecar's data directory under `limits`: a real
/// directory, not a link; owner-only on Linux and macOS; and within both
/// limits. Otherwise why not, as a clause the supervisor ends with ", so
/// srelens did not start it" or ", so srelens stopped it".
pub fn check(path: &Path, limits: &Limits) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| {
        format!(
            "The extension's data directory {} cannot be read: {e}",
            path.display()
        )
    })?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "The extension's data directory {} is a symbolic link, which the sandbox would follow",
            path.display()
        ));
    }
    if !meta.is_dir() {
        return Err(format!(
            "The extension's data directory {} is not a directory",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        if meta.uid() != me {
            return Err(format!(
                "The extension's data directory {} belongs to another account",
                path.display()
            ));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "The extension's data directory {} can be read by other users",
                path.display()
            ));
        }
    }
    let usage = measure(path, limits.data_entries).map_err(|e| {
        format!(
            "The extension's data directory {} cannot be measured: {e}",
            path.display()
        )
    })?;
    if usage.entries > limits.data_entries {
        return Err(format!(
            "The extension's data directory holds more than {} files and directories, the most srelens allows",
            limits.data_entries
        ));
    }
    if usage.bytes > limits.data_bytes {
        return Err(format!(
            "The extension's data directory holds {}, over its {} limit",
            size(usage.bytes),
            size(limits.data_bytes)
        ));
    }
    Ok(())
}

/// A size as a person reads it: "900 bytes", "1.5 KiB", "256 MiB".
pub(crate) fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if (value - value.round()).abs() < 0.05 {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::Limits;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    fn limits(bytes: u64, entries: u64) -> Limits {
        Limits {
            data_bytes: bytes,
            data_entries: entries,
            ..Limits::default()
        }
    }

    #[test]
    fn a_data_directory_is_named_by_a_digest_of_the_exact_app_id() {
        let name = directory_name("org.example.scanner");
        assert_eq!(name.len(), 32, "{name}");
        assert!(name
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(name, directory_name("org.example.scanner"), "stable");
        // IDs are case-sensitive, and a case-insensitive filesystem would give
        // two apps one directory if it were named by the ID.
        assert_ne!(name, directory_name("org.Example.scanner"));
    }

    #[test]
    fn each_app_gets_its_own_directory_under_the_root() {
        let root = root();
        let a = DataDir::for_app(root.path(), "org.example.a").unwrap();
        let b = DataDir::for_app(root.path(), "org.example.b").unwrap();
        assert_ne!(a.path(), b.path());
        assert_eq!(a.path(), root.path().join(directory_name("org.example.a")));
        assert!(a.path().is_dir() && b.path().is_dir());
        // Opening it again is the same directory, with what it holds.
        std::fs::write(a.path().join("cache.db"), "x").unwrap();
        let again = DataDir::for_app(root.path(), "org.example.a").unwrap();
        assert!(again.path().join("cache.db").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_and_a_root_it_creates_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let parent = root();
        let root = parent.path().join("data");
        let dir = DataDir::for_app(&root, "org.example.a").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(dir.path()), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_directory_others_could_read_is_made_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let path = root.path().join(directory_name("org.example.a"));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        DataDir::for_app(root.path(), "org.example.a").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_in_place_of_the_directory_is_refused_not_followed() {
        let root = root();
        let elsewhere = tempfile::tempdir().unwrap();
        let path = root.path().join(directory_name("org.example.a"));
        std::os::unix::fs::symlink(elsewhere.path(), &path).unwrap();
        let err = DataDir::for_app(root.path(), "org.example.a").unwrap_err();
        assert!(err.to_string().contains("symbolic link"), "{err}");
        // The sandbox would have granted the link's target.
        assert!(check(&path, &Limits::default())
            .unwrap_err()
            .contains("symbolic link"));
    }

    #[test]
    fn usage_counts_every_file_and_directory_below_it() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        std::fs::create_dir_all(dir.path().join("db/nested")).unwrap();
        std::fs::write(dir.path().join("db/trivy.db"), vec![0u8; 10_000]).unwrap();
        std::fs::write(dir.path().join("db/nested/meta.json"), vec![0u8; 3_000]).unwrap();
        let usage = measure(dir.path(), 1_000).unwrap();
        assert_eq!(usage.entries, 4, "two directories and two files");
        // At least what the files hold; a filesystem may allocate more.
        assert!(usage.bytes >= 13_000, "{usage:?}");
        assert!(usage.bytes < 13_000 + 64 * 1024, "{usage:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_counts_as_itself_and_is_not_followed() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("big"), vec![0u8; 1 << 20]).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("big"), dir.path().join("big")).unwrap();
        let usage = measure(dir.path(), 1_000).unwrap();
        assert_eq!(usage.entries, 2);
        assert!(usage.bytes < 1 << 20, "a link was followed: {usage:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_with_several_hard_links_is_counted_once() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        std::fs::write(dir.path().join("one"), vec![0u8; 100_000]).unwrap();
        for n in 0..5 {
            std::fs::hard_link(dir.path().join("one"), dir.path().join(format!("link{n}")))
                .unwrap();
        }
        let usage = measure(dir.path(), 1_000).unwrap();
        assert_eq!(usage.entries, 6);
        assert!(usage.bytes < 200_000, "counted more than once: {usage:?}");
    }

    #[test]
    fn a_sparse_or_short_file_counts_the_larger_of_its_length_and_its_blocks() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        // A file whose length is far past the blocks it has: counted by its
        // length, so a sparse file cannot hide what it will hold once filled.
        let file = std::fs::File::create(dir.path().join("sparse")).unwrap();
        file.set_len(50 << 20).unwrap();
        let usage = measure(dir.path(), 1_000).unwrap();
        assert!(usage.bytes >= 50 << 20, "{usage:?}");
    }

    #[test]
    fn measuring_stops_past_the_entry_limit() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        for n in 0..50 {
            std::fs::write(dir.path().join(format!("f{n}")), "x").unwrap();
        }
        let usage = measure(dir.path(), 10).unwrap();
        assert_eq!(usage.entries, 11, "one past the limit is enough to know");
    }

    #[test]
    fn a_directory_within_its_limits_passes_the_check() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        std::fs::write(dir.path().join("f"), vec![0u8; 1000]).unwrap();
        assert_eq!(check(dir.path(), &limits(1 << 20, 10)), Ok(()));
    }

    #[test]
    fn a_directory_over_its_size_limit_is_refused_naming_both_sizes() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        std::fs::write(dir.path().join("f"), vec![0u8; 3 << 20]).unwrap();
        let why = check(dir.path(), &limits(2 << 20, 100)).unwrap_err();
        assert!(why.contains("over its 2 MiB limit"), "{why}");
        assert!(why.contains("holds 3 MiB"), "{why}");
    }

    #[test]
    fn a_directory_with_too_many_entries_is_refused() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        for n in 0..20 {
            std::fs::write(dir.path().join(format!("f{n}")), "x").unwrap();
        }
        let why = check(dir.path(), &limits(1 << 30, 10)).unwrap_err();
        assert!(why.contains("more than 10 files and directories"), "{why}");
    }

    #[test]
    fn a_missing_directory_is_refused_by_path() {
        let root = root();
        let path = root.path().join("never-created");
        let why = check(&path, &Limits::default()).unwrap_err();
        assert!(why.contains("never-created"), "{why}");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_other_users_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let why = check(dir.path(), &Limits::default()).unwrap_err();
        assert!(why.contains("other users"), "{why}");
    }

    #[test]
    fn sizes_read_as_a_person_would_say_them() {
        assert_eq!(size(0), "0 bytes");
        assert_eq!(size(900), "900 bytes");
        assert_eq!(size(1024), "1 KiB");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(256 << 20), "256 MiB");
        assert_eq!(size((1 << 30) + (1 << 29)), "1.5 GiB");
    }

    #[test]
    fn pruning_removes_the_directories_of_apps_no_longer_installed() {
        let root = root();
        let kept = DataDir::for_app(root.path(), "org.example.kept").unwrap();
        let gone = DataDir::for_app(root.path(), "org.example.gone").unwrap();
        std::fs::create_dir_all(gone.path().join("db")).unwrap();
        std::fs::write(gone.path().join("db/cache"), "x").unwrap();
        // Not a name this module gives: left alone.
        std::fs::write(root.path().join("README"), "not ours").unwrap();
        prune(root.path(), &["org.example.kept"]).unwrap();
        assert!(kept.path().is_dir());
        assert!(!gone.path().exists());
        assert!(root.path().join("README").is_file());
    }

    #[test]
    fn pruning_a_root_that_does_not_exist_is_nothing_to_do() {
        let root = root();
        assert!(prune(&root.path().join("absent"), &[]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn pruning_and_clearing_remove_a_link_and_never_its_target() {
        let root = root();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("keep-me"), "x").unwrap();
        // A link where an app's directory would be, and one inside a directory.
        let gone = root.path().join(directory_name("org.example.gone"));
        std::os::unix::fs::symlink(target.path(), &gone).unwrap();
        let kept = DataDir::for_app(root.path(), "org.example.kept").unwrap();
        std::os::unix::fs::symlink(target.path(), kept.path().join("link")).unwrap();
        prune(root.path(), &["org.example.kept"]).unwrap();
        kept.clear().unwrap();
        assert!(target.path().join("keep-me").is_file());
        assert!(!gone.exists() && std::fs::symlink_metadata(&gone).is_err());
    }

    /// A sidecar owns what it writes, so it can take its own write access
    /// away from a directory; the host puts it back before removing, or an
    /// uninstalled app's data would outlive it.
    #[cfg(unix)]
    #[test]
    fn a_directory_the_sidecar_locked_is_still_cleared_and_pruned() {
        use std::os::unix::fs::PermissionsExt;
        let lock = |dir: &Path, mode: u32| {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        let root = root();
        for id in ["org.example.gone", "org.example.kept"] {
            let dir = DataDir::for_app(root.path(), id).unwrap();
            std::fs::create_dir_all(dir.path().join("db/deep")).unwrap();
            std::fs::write(dir.path().join("db/deep/f"), "x").unwrap();
            lock(&dir.path().join("db/deep"), 0o500);
            lock(&dir.path().join("db"), 0o000);
        }
        let kept = DataDir::for_app(root.path(), "org.example.kept").unwrap();
        prune(root.path(), &["org.example.kept"]).unwrap();
        assert!(!root
            .path()
            .join(directory_name("org.example.gone"))
            .exists());
        kept.clear().unwrap();
        assert_eq!(std::fs::read_dir(kept.path()).unwrap().count(), 0);
    }

    #[test]
    fn clearing_empties_the_directory_and_keeps_it() {
        let root = root();
        let dir = DataDir::for_app(root.path(), "org.example.a").unwrap();
        std::fs::create_dir_all(dir.path().join("db/nested")).unwrap();
        std::fs::write(dir.path().join("db/nested/f"), "x").unwrap();
        std::fs::write(dir.path().join("top"), "x").unwrap();
        dir.clear().unwrap();
        assert!(dir.path().is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
