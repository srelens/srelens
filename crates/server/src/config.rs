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

/// How large one log stream's history may grow on the server, in megabytes,
/// unless `SRELENS_LOG_SPOOL_MB` says otherwise (#914). The per-user runtime
/// directory it lives in is tmpfs in the shipped compose file, which is RAM, and
/// the limit is per open log stream, not per user: so the default is a quarter
/// of the desktop's 256 MB, and an operator who wants more sizes the container's
/// memory for it.
pub const DEFAULT_LOG_SPOOL_MB: u64 = 64;

/// Read `SRELENS_LOG_SPOOL_MB`. Unset or empty means the default, and 0 turns
/// spooling off; anything else must be a whole number of megabytes. Pure: the
/// caller passes the env value in.
pub fn log_spool_mb(env: Option<&str>) -> Result<u64, String> {
    match env.map(str::trim) {
        None | Some("") => Ok(DEFAULT_LOG_SPOOL_MB),
        Some(value) => value.parse().map_err(|_| {
            format!("SRELENS_LOG_SPOOL_MB must be a whole number of megabytes, not {value:?}")
        }),
    }
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
    fn log_spool_size_defaults_parses_and_refuses_junk() {
        // 64, not the desktop's 256: on the web this is RAM (see the constant).
        assert_eq!(DEFAULT_LOG_SPOOL_MB, 64);
        assert_eq!(log_spool_mb(None), Ok(64));
        assert_eq!(log_spool_mb(Some("")), Ok(64));
        assert_eq!(log_spool_mb(Some("0")), Ok(0));
        assert_eq!(log_spool_mb(Some(" 64 ")), Ok(64));
        assert!(log_spool_mb(Some("lots"))
            .unwrap_err()
            .contains("SRELENS_LOG_SPOOL_MB"));
    }

    /// The operator's docs name every knob `serve()` reads (#914), and say what
    /// it costs: the history is RAM on the shipped tmpfs runtime directory.
    #[test]
    fn the_web_docs_name_the_log_spool_size() {
        let web = include_str!("../../../docs/WEB.md");
        assert!(
            web.contains("`SRELENS_LOG_SPOOL_MB`"),
            "docs/WEB.md does not document SRELENS_LOG_SPOOL_MB"
        );
        assert!(
            web.contains(&format!("(default `{DEFAULT_LOG_SPOOL_MB}`)")),
            "docs/WEB.md does not state the default {DEFAULT_LOG_SPOOL_MB}"
        );
        let compose = include_str!("../../../docker-compose.yml");
        assert!(
            compose.contains("SRELENS_LOG_SPOOL_MB"),
            "docker-compose.yml does not warn that log history is held in its tmpfs"
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
