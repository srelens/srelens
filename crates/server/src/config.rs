//! Server runtime configuration: resolution of the data directory that holds
//! all persistent state (SQLite database, master key, per-user runtime files).

use std::path::{Path, PathBuf};

/// Resolve the data directory: explicit `--data` flag value wins, else the
/// `SRELENS_DATA` environment value, else `./srelens-data`. Pure — callers
/// pass the env value in so tests never touch process environment.
pub fn resolve_data_dir(flag: Option<&str>, env: Option<&str>) -> PathBuf {
    if let Some(f) = flag {
        return PathBuf::from(f);
    }
    if let Some(e) = env {
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    PathBuf::from("./srelens-data")
}

/// Create the data directory if missing. On unix it is created (or tightened)
/// to mode 0700 — it holds the sealed database and the (plaintext, ideally
/// tmpfs-backed) decrypted runtime files. In server mode the master key comes
/// from `SRELENS_MASTER_KEY` and is never written here.
pub fn ensure_data_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_wins_over_env_and_default() {
        assert_eq!(
            resolve_data_dir(Some("/tmp/x"), Some("/tmp/y")),
            PathBuf::from("/tmp/x")
        );
    }

    #[test]
    fn env_wins_over_default() {
        assert_eq!(
            resolve_data_dir(None, Some("/tmp/y")),
            PathBuf::from("/tmp/y")
        );
        // Empty env value falls through to the default.
        assert_eq!(
            resolve_data_dir(None, Some("")),
            PathBuf::from("./srelens-data")
        );
    }

    #[test]
    fn default_when_nothing_given() {
        assert_eq!(
            resolve_data_dir(None, None),
            PathBuf::from("./srelens-data")
        );
    }

    #[test]
    fn ensure_creates_dir_with_owner_only_perms() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data");
        ensure_data_dir(&dir).unwrap();
        assert!(dir.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }
}
