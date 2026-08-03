//! Bearer-token auth for the HTTP transport. stdio needs none: the client
//! spawned the process and already holds the user's privileges.

use std::path::PathBuf;

use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("system randomness");
        Token(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(Token(s.to_ascii_lowercase()))
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison: a byte-by-byte `==` leaks the shared prefix
    /// length through timing, which is enough to recover a token.
    pub fn matches(&self, presented: &str) -> bool {
        let a = self.0.as_bytes();
        let b = presented.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        a.ct_eq(b).into()
    }
}

/// Deliberately opaque: a token must never reach a log line by accident.
impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Token(<redacted>)")
    }
}

pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<Token>;
    fn save(&self, t: &Token) -> std::io::Result<()>;
    fn clear(&self) -> std::io::Result<()>;
}

/// Fallback store: a 0600 file. Used where no OS keychain is available
/// (headless Linux, minimal window managers).
pub struct FileTokenStore {
    path: PathBuf,
}

impl FileTokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> Option<Token> {
        std::fs::read_to_string(&self.path).ok().and_then(|s| Token::from_hex(&s))
    }

    fn save(&self, t: &Token) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)?;
            f.write_all(t.as_str().as_bytes())?;
            // Belt-and-braces: enforce 0600 even for overwrites of existing files
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.path, t.as_str())?;
        }
        Ok(())
    }

    fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_64_hex_chars_and_unique() {
        let a = Token::generate();
        let b = Token::generate();
        assert_eq!(a.as_str().len(), 64);
        assert!(a.as_str().bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a.as_str(), b.as_str());
    }

    #[test]
    fn matches_accepts_itself_and_rejects_others() {
        let t = Token::generate();
        assert!(t.matches(t.as_str()));
        assert!(!t.matches(""));
        assert!(!t.matches("short"));
        assert!(!t.matches(&format!("{}0", t.as_str())));
    }

    #[test]
    fn debug_never_prints_the_token() {
        let t = Token::generate();
        let shown = format!("{t:?}");
        assert!(!shown.contains(t.as_str()), "token leaked into Debug output");
    }

    #[test]
    fn from_hex_rejects_malformed_values() {
        assert!(Token::from_hex("nope").is_none());
        assert!(Token::from_hex(&"z".repeat(64)).is_none());
        assert!(Token::from_hex(&"a".repeat(64)).is_some());
    }

    #[test]
    fn file_store_round_trips_and_clears() {
        let dir = std::env::temp_dir().join(format!("srelens-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileTokenStore::new(dir.join("token"));
        assert!(store.load().is_none());
        let t = Token::generate();
        store.save(&t).unwrap();
        assert_eq!(store.load().unwrap().as_str(), t.as_str());
        store.clear().unwrap();
        assert!(store.load().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("srelens-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        let store = FileTokenStore::new(path.clone());
        store.save(&Token::generate()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "token file must not be group/world readable");
    }

    #[cfg(unix)]
    #[test]
    fn file_store_tightens_loose_permissions_on_overwrite() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("srelens-perm2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        // Pre-create with loose permissions (0644)
        std::fs::write(&path, "dummy").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        // Now save should tighten to 0600
        let store = FileTokenStore::new(path.clone());
        store.save(&Token::generate()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "save must tighten loose permissions to 0600");
    }
}
