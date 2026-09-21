//! The portable setup bundle (issue #654): one encrypted file that carries a
//! reader's whole srelens setup to another machine — settings, cluster
//! kubeconfigs, assistant skills, MCP prompts, and (only if they ask) the
//! provider API keys and MCP token from the vault.
//!
//! **Kubeconfig contents, never kubeconfig paths.** The settings document
//! records `srelens.kubeconfigFiles` as absolute paths, which mean nothing on
//! the next machine, so that one key is dropped from the bundle and the files
//! themselves ride inside it. On import they land in the managed kubeconfig
//! directory, which `managed_kubeconfig_files()` already enumerates on every
//! listing — so an imported cluster appears with no path rewriting at all.
//!
//! **Always encrypted, and the cleartext says nothing.** A bundle worth
//! carrying holds cluster credentials, so there is no useful unencrypted form.
//! The file is
//!
//! ```text
//! "SRELENS-BUNDLE\n"  magic
//! version: u8 = 1
//! header length: u32, big endian
//! header: JSON `vault::VaultMeta` — argon2id parameters, salt, verifier
//! payload: XChaCha20-Poly1305, exactly `vault::seal_bytes`'s framing
//! ```
//!
//! The header carries only what deriving the key needs. No hostname, no
//! context names, no counts: a bundle sitting in a Downloads folder must not
//! enumerate its owner's clusters. Everything descriptive — the manifest the
//! import preview renders — lives inside the ciphertext, which is why
//! `preview` takes the passphrase too.
//!
//! The KDF is the vault's own (`vault::build_meta` / `unlock_key_for`), so a
//! wrong passphrase is rejected by the sealed verifier before the payload is
//! touched, and bundle and vault never drift apart on argon2 parameters.
//!
//! Import is additive and idempotent. A bundled kubeconfig whose bytes already
//! exist in any file the app reads is skipped rather than duplicated; a skill
//! or prompt whose name is taken is reported, not overwritten. Nothing here
//! deletes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::vault::{self, Secrets, VaultMeta};

/// Leading bytes of every bundle. A file that does not start with these is
/// reported as "not a srelens bundle" rather than as a bad passphrase — the
/// two failures have entirely different fixes.
const MAGIC: &[u8] = b"SRELENS-BUNDLE\n";
const FORMAT_VERSION: u8 = 1;
const SCHEMA_VERSION: u32 = 1;

/// Refuse to read a file larger than this before allocating for it.
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
/// Cap on the cleartext header, so a corrupt length cannot drive a huge read.
const MAX_HEADER_BYTES: usize = 8 * 1024;
/// Per-member cap, matching the 1 MB gate `files.rs` already applies to a
/// pasted kubeconfig.
const MAX_MEMBER_BYTES: usize = 1024 * 1024;
/// Cap per group, so a hostile bundle cannot fill the config directory.
const MAX_MEMBERS: usize = 512;

/// The settings key holding absolute kubeconfig paths — meaningless on another
/// machine, so it is dropped on export and never written on import.
const KUBECONFIG_FILES_KEY: &str = "srelens.kubeconfigFiles";

/// The directory, relative to the app config dir, that each file group lives
/// in. Import writes only inside these.
fn kubeconfigs_dir(base: &Path) -> PathBuf {
    base.join("kubeconfigs")
}
fn skills_dir(base: &Path) -> PathBuf {
    base.join("assistant").join("skills")
}
fn prompts_dir(base: &Path) -> PathBuf {
    base.join("mcp").join("prompts")
}
fn extensions_inventory_path(base: &Path) -> PathBuf {
    base.join("settings.extensions.json")
}

/// One file carried by name and content. The name is a bare file name — never
/// a path — and `sanitized_name` is what actually reaches the filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundledFile {
    pub name: String,
    pub content: String,
}

/// An installed app, recorded so the reader knows what to reinstall. Apps are
/// code with capability grants: they are listed, never installed by an import.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BundledExtension {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// The bundle's plaintext. Every field is `default` so a file written by an
/// older build still decodes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bundle {
    pub schema_version: u32,
    /// RFC 3339, for the import preview. Purely descriptive.
    pub created: String,
    pub app_version: String,
    pub settings: BTreeMap<String, Value>,
    pub kubeconfigs: Vec<BundledFile>,
    pub skills: Vec<BundledFile>,
    pub prompts: Vec<BundledFile>,
    pub extensions: Vec<BundledExtension>,
    pub secrets: Option<Secrets>,
}

/// The groups a reader can include on export and select on import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Settings,
    Kubeconfigs,
    Skills,
    Prompts,
    Secrets,
}

/// What an opened bundle contains, for the import preview. Secrets are
/// described by name only — a provider slug and whether a token is present —
/// never by value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummary {
    pub created: String,
    pub app_version: String,
    pub settings_keys: usize,
    pub kubeconfigs: Vec<String>,
    pub skills: Vec<String>,
    pub prompts: Vec<String>,
    pub extensions: Vec<BundledExtension>,
    /// Provider slugs whose API key is in the bundle.
    pub secret_keys: Vec<String>,
    pub has_mcp_token: bool,
}

/// Exactly what an import wrote, and what it left alone and why. Rendered
/// verbatim: an import that silently did nothing is indistinguishable from one
/// that worked unless it says so.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub settings_written: Vec<String>,
    pub kubeconfigs_added: Vec<String>,
    /// Bundled configs whose bytes the app already reads from somewhere.
    pub kubeconfigs_already_present: Vec<String>,
    /// Bundled configs that do not parse as a kubeconfig, so were not written.
    /// Kept apart from `kubeconfigs_already_present` because they are opposite
    /// facts: one says the cluster is here, the other that the file was not
    /// usable. Reporting a rejection as "already here" is a claim about this
    /// machine that nothing checked.
    pub kubeconfigs_rejected: Vec<String>,
    pub skills_added: Vec<String>,
    /// Names already taken locally; the local file is left as it is.
    pub skills_kept_local: Vec<String>,
    pub prompts_added: Vec<String>,
    pub prompts_kept_local: Vec<String>,
    pub secrets_written: Vec<String>,
}

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

/// Encrypt `bundle` under a key derived from `passphrase`.
pub fn seal(passphrase: &str, bundle: &Bundle) -> Result<Vec<u8>, String> {
    let (meta, key) = vault::build_meta(passphrase)?;
    let header = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    if header.len() > MAX_HEADER_BYTES {
        return Err("bundle header is implausibly large".into());
    }
    let plaintext = serde_json::to_vec(bundle).map_err(|e| e.to_string())?;
    let payload = vault::seal_bytes(&key, &plaintext).map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(MAGIC.len() + 5 + header.len() + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Read a bundle's cleartext header, without deriving anything. Split out so
/// "this is not a bundle" and "that passphrase does not open this bundle" stay
/// two distinct answers.
fn split(raw: &[u8]) -> Result<(VaultMeta, &[u8]), String> {
    const NOT_A_BUNDLE: &str = "this file is not a srelens setup bundle";
    if raw.len() < MAGIC.len() + 5 || &raw[..MAGIC.len()] != MAGIC {
        return Err(NOT_A_BUNDLE.into());
    }
    let version = raw[MAGIC.len()];
    if version != FORMAT_VERSION {
        return Err(format!(
            "this bundle is format version {version}; this srelens reads version {FORMAT_VERSION}"
        ));
    }
    let at = MAGIC.len() + 1;
    let header_len = u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]) as usize;
    if header_len > MAX_HEADER_BYTES || raw.len() < at + 4 + header_len {
        return Err(NOT_A_BUNDLE.into());
    }
    let (header, payload) = raw[at + 4..].split_at(header_len);
    let meta: VaultMeta = serde_json::from_slice(header).map_err(|_| NOT_A_BUNDLE.to_string())?;
    Ok((meta, payload))
}

/// Decrypt a bundle. A wrong passphrase is refused by the header's sealed
/// verifier before the payload is decrypted, and a tampered or truncated
/// payload by the AEAD tag — neither can produce a partly-read bundle.
pub fn open(passphrase: &str, raw: &[u8]) -> Result<Bundle, String> {
    let (meta, payload) = split(raw)?;
    let key = vault::unlock_key_for(&meta, passphrase)
        .map_err(|_| "that passphrase does not open this bundle".to_string())?;
    let plaintext =
        vault::open_bytes(&key, payload).ok_or("this bundle is damaged and cannot be read")?;
    let bundle: Bundle = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("this bundle's contents could not be read: {e}"))?;
    if bundle.schema_version > SCHEMA_VERSION {
        return Err(format!(
            "this bundle was written by a newer srelens (contents version {}); update to import it",
            bundle.schema_version
        ));
    }
    Ok(bundle)
}

impl Bundle {
    /// The manifest the import preview renders.
    pub fn summary(&self) -> BundleSummary {
        let names = |files: &[BundledFile]| files.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        BundleSummary {
            created: self.created.clone(),
            app_version: self.app_version.clone(),
            settings_keys: self.settings.len(),
            kubeconfigs: names(&self.kubeconfigs),
            skills: names(&self.skills),
            prompts: names(&self.prompts),
            extensions: self.extensions.clone(),
            secret_keys: self
                .secrets
                .as_ref()
                .map(|s| s.llm_keys.keys().cloned().collect())
                .unwrap_or_default(),
            has_mcp_token: self
                .secrets
                .as_ref()
                .is_some_and(|s| s.mcp_token.as_deref().is_some_and(|t| !t.is_empty())),
        }
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Everything the export needs from the host, so `collect` itself is pure over
/// a directory and a path list (the `#28` seam) and testable without Tauri.
pub struct ExportSources<'a> {
    /// The app config directory.
    pub base: &'a Path,
    /// Every kubeconfig the app reads: `all_kubeconfig_paths()` plus whatever
    /// the reader added under `srelens.kubeconfigFiles`.
    pub kubeconfig_paths: &'a [PathBuf],
    /// The settings document's `values` map, verbatim from `settings.get`.
    pub settings: BTreeMap<String, Value>,
    pub secrets: Option<Secrets>,
    pub app_version: String,
    /// RFC 3339 timestamp; passed in so tests are deterministic.
    pub created: String,
}

/// Build a bundle from what is on disk. Unreadable members are skipped, not
/// fatal: a bundle missing one unreadable skill is worth far more than no
/// bundle at all, and `summary()` shows the reader exactly what was captured.
pub fn collect(sources: ExportSources<'_>) -> Bundle {
    let mut settings = sources.settings;
    // Absolute paths from the machine being left behind. The files themselves
    // are in `kubeconfigs` below; carrying the paths would only relocate
    // clusters to directories that do not exist on the new machine.
    settings.remove(KUBECONFIG_FILES_KEY);

    Bundle {
        schema_version: SCHEMA_VERSION,
        created: sources.created,
        app_version: sources.app_version,
        settings,
        kubeconfigs: read_paths(sources.kubeconfig_paths),
        skills: read_dir_files(&skills_dir(sources.base)),
        prompts: read_dir_files(&prompts_dir(sources.base)),
        extensions: read_extensions(&extensions_inventory_path(sources.base)),
        secrets: sources.secrets.filter(|s| !s.llm_keys.is_empty() || s.mcp_token.is_some()),
    }
}

/// Read named files by explicit path, giving each a unique bare name.
fn read_paths(paths: &[PathBuf]) -> Vec<BundledFile> {
    let mut out: Vec<BundledFile> = Vec::new();
    for path in paths {
        if out.len() >= MAX_MEMBERS {
            break;
        }
        let Some(content) = read_member(path) else { continue };
        // Two machines' `~/.kube/config` and `~/work/.kube/config` are both
        // "config"; identical content is one file, different content needs two
        // names or the second silently replaces the first.
        if out.iter().any(|f| f.content == content) {
            continue;
        }
        let stem = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(sanitize_name)
            .unwrap_or_else(|| "kubeconfig".into());
        out.push(BundledFile { name: unique_name(&out, &stem), content });
    }
    out
}

/// Read every non-hidden file directly inside `dir`.
fn read_dir_files(dir: &Path) -> Vec<BundledFile> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<BundledFile> = Vec::new();
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
        })
        .collect();
    paths.sort();
    for path in paths {
        if out.len() >= MAX_MEMBERS {
            break;
        }
        let Some(content) = read_member(&path) else { continue };
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(sanitize_name) else {
            continue;
        };
        out.push(BundledFile { name: unique_name(&out, &name), content });
    }
    out
}

/// Read a member, or `None` when it is absent, too large, or not UTF-8.
fn read_member(path: &Path) -> Option<String> {
    let size = fs::metadata(path).ok()?.len();
    if size == 0 || size as usize > MAX_MEMBER_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// List installed apps from the extensions inventory, best-effort. The
/// inventory's own types live in `srelens-registry` and deny unknown fields;
/// re-modelling them here would break on every inventory change for a list
/// that is purely informational, so this reads the three fields it needs and
/// ignores everything it does not recognise.
fn read_extensions(path: &Path) -> Vec<BundledExtension> {
    let Ok(raw) = fs::read(path) else { return Vec::new() };
    let Ok(value) = serde_json::from_slice::<Value>(&raw) else { return Vec::new() };
    let Some(plugins) = value.get("plugins").and_then(Value::as_array) else {
        return Vec::new();
    };
    plugins
        .iter()
        .take(MAX_MEMBERS)
        .filter_map(|plugin| {
            let manifest = plugin.get("manifest")?;
            let text = |key: &str| {
                manifest.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
            };
            let id = text("id");
            if id.is_empty() {
                return None;
            }
            let name = match text("name") {
                empty if empty.is_empty() => id.clone(),
                name => name,
            };
            Some(BundledExtension { id, name, version: text("version") })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Apply the file-backed groups of `bundle` under `base`. Settings and secrets
/// are applied by the caller — they go through the settings capability and the
/// vault respectively, neither of which is a plain file write.
///
/// `existing_kubeconfigs` is every kubeconfig path the app currently reads;
/// a bundled config whose bytes are already among them is skipped, which is
/// what makes a second import of the same bundle a no-op.
pub fn apply_files(
    base: &Path,
    bundle: &Bundle,
    groups: &[Group],
    existing_kubeconfigs: &[PathBuf],
) -> Result<ImportReport, String> {
    let mut report = ImportReport::default();

    if groups.contains(&Group::Kubeconfigs) && !bundle.kubeconfigs.is_empty() {
        let dir = kubeconfigs_dir(base);
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let present: Vec<String> = existing_kubeconfigs.iter().filter_map(|p| read_member(p)).collect();
        for file in bundle.kubeconfigs.iter().take(MAX_MEMBERS) {
            if present.iter().any(|existing| existing == &file.content) {
                report.kubeconfigs_already_present.push(file.name.clone());
                continue;
            }
            // Only a file that parses as a kubeconfig is written — the same
            // gate `save_pasted_kubeconfig` applies, so an import cannot leave
            // a file that breaks every later context listing.
            if srelens_kube::connect::validate_kubeconfig_yaml(&file.content).is_err() {
                report.kubeconfigs_rejected.push(file.name.clone());
                continue;
            }
            let name = free_name(&dir, &sanitize_name(&file.name));
            write_member(&dir.join(&name), &file.content)?;
            report.kubeconfigs_added.push(name);
        }
    }

    if groups.contains(&Group::Skills) {
        let (added, kept) = write_group(&skills_dir(base), &bundle.skills)?;
        report.skills_added = added;
        report.skills_kept_local = kept;
    }

    if groups.contains(&Group::Prompts) {
        let (added, kept) = write_group(&prompts_dir(base), &bundle.prompts)?;
        report.prompts_added = added;
        report.prompts_kept_local = kept;
    }

    Ok(report)
}

/// Write each file into `dir` unless a file of that name is already there.
/// Returns `(written, kept_local)`.
fn write_group(dir: &Path, files: &[BundledFile]) -> Result<(Vec<String>, Vec<String>), String> {
    if files.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let mut added = Vec::new();
    let mut kept = Vec::new();
    for file in files.iter().take(MAX_MEMBERS) {
        let name = sanitize_name(&file.name);
        if name.is_empty() {
            continue;
        }
        let path = dir.join(&name);
        if path.exists() {
            // A skill the reader has edited on this machine outranks the
            // bundle's copy; say so rather than overwriting their work.
            if read_member(&path).as_deref() != Some(file.content.as_str()) {
                kept.push(name);
            }
            continue;
        }
        write_member(&path, &file.content)?;
        added.push(name);
    }
    Ok((added, kept))
}

fn write_member(path: &Path, content: &str) -> Result<(), String> {
    if content.len() > MAX_MEMBER_BYTES {
        return Err(format!("{} is larger than the 1 MB limit", path.display()));
    }
    fs::write(path, content).map_err(|e| format!("write {}: {e}", path.display()))
}

/// The settings to write on import: the bundle's keys, minus the ones that
/// only describe the machine it came from. Separate from `apply_files` because
/// settings go through the settings capability, which holds the cross-process
/// lock that a direct write would bypass.
pub fn importable_settings(bundle: &Bundle) -> BTreeMap<String, Value> {
    let mut values = bundle.settings.clone();
    values.remove(KUBECONFIG_FILES_KEY);
    values
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Reduce a bundled name to a bare, safe file name. Anything that could escape
/// the target directory — separators, `..`, drive letters, control characters
/// — becomes `-`, so a hostile bundle writes inside the group's directory or
/// nowhere.
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '-' || c == '.');
    // `..` survives the character filter above (both are allowed characters),
    // so the trim of leading dots is what actually stops traversal.
    trimmed.chars().take(96).collect()
}

/// A name not already used by `taken`, suffixed `-2`, `-3`, … when it is.
fn unique_name(taken: &[BundledFile], name: &str) -> String {
    let base = if name.is_empty() { "file" } else { name };
    if !taken.iter().any(|f| f.name == base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (base, String::new()),
    };
    (2..)
        .map(|n| format!("{stem}-{n}{ext}"))
        .find(|candidate| !taken.iter().any(|f| &f.name == candidate))
        .unwrap_or_else(|| base.to_string())
}

/// A name that does not exist in `dir`, suffixed the same way.
fn free_name(dir: &Path, name: &str) -> String {
    let base = if name.is_empty() { "kubeconfig.yaml".to_string() } else { name.to_string() };
    if !dir.join(&base).exists() {
        return base;
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (base.clone(), String::new()),
    };
    (2..)
        .map(|n| format!("{stem}-{n}{ext}"))
        .find(|candidate| !dir.join(candidate).exists())
        .unwrap_or(base)
}

/// Read a bundle file from disk, refusing an implausibly large one before
/// allocating for it.
pub fn read_bundle_file(path: &Path) -> Result<Vec<u8>, String> {
    let size = fs::metadata(path).map_err(|e| format!("read {}: {e}", path.display()))?.len();
    if size > MAX_BUNDLE_BYTES {
        return Err("that file is too large to be a srelens setup bundle".into());
    }
    fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(label: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "srelens-bundle-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const KUBECONFIG: &str = "\
apiVersion: v1
kind: Config
clusters:
- name: c
  cluster: { server: 'https://127.0.0.1:1' }
contexts:
- name: ctx
  context: { cluster: c, user: u }
users:
- name: u
  user: {}
current-context: ctx
";

    /// A second cluster, so "different content under the same file name" is a
    /// real case rather than a copy of the first.
    const OTHER_KUBECONFIG: &str = "\
apiVersion: v1
kind: Config
clusters:
- name: d
  cluster: { server: 'https://127.0.0.2:1' }
contexts:
- name: other
  context: { cluster: d, user: v }
users:
- name: v
  user: {}
current-context: other
";

    fn sample() -> Bundle {
        Bundle {
            schema_version: SCHEMA_VERSION,
            created: "2026-09-21T10:00:00Z".into(),
            app_version: "0.15.0".into(),
            settings: BTreeMap::from([("srelens.defaultNamespace".into(), json!("kube-system"))]),
            kubeconfigs: vec![BundledFile { name: "config".into(), content: KUBECONFIG.into() }],
            skills: vec![BundledFile { name: "triage.md".into(), content: "# triage".into() }],
            prompts: vec![],
            extensions: vec![],
            secrets: None,
        }
    }

    // --- sealing -----------------------------------------------------------

    #[test]
    fn a_sealed_bundle_round_trips_through_the_passphrase() {
        let sealed = seal("correct horse battery", &sample()).unwrap();
        assert_eq!(open("correct horse battery", &sealed).unwrap(), sample());
    }

    #[test]
    fn the_cleartext_header_names_nothing_that_is_inside() {
        let mut bundle = sample();
        bundle.settings.insert("srelens.contextProfiles".into(), json!({"prod-eu": {}}));
        let sealed = seal("pw", &bundle).unwrap();
        // Everything after the header is ciphertext, so no context name, file
        // name or setting key may appear anywhere in the file.
        for needle in ["prod-eu", "triage.md", "kube-system", "127.0.0.1", "defaultNamespace"] {
            assert!(
                !sealed.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle} leaked into the bundle's bytes"
            );
        }
        // What IS readable is only the KDF header.
        let (meta, _) = split(&sealed).unwrap();
        assert_eq!(meta.kdf_alg, "argon2id");
    }

    #[test]
    fn a_wrong_passphrase_is_refused_and_named_as_such() {
        let sealed = seal("right", &sample()).unwrap();
        let error = open("wrong", &sealed).unwrap_err();
        assert!(error.contains("passphrase"), "unexpected error: {error}");
    }

    #[test]
    fn a_file_that_is_not_a_bundle_says_so_rather_than_blaming_the_passphrase() {
        let error = open("pw", b"{\"just\": \"json\"}").unwrap_err();
        assert!(error.contains("not a srelens setup bundle"), "unexpected error: {error}");
    }

    #[test]
    fn a_tampered_payload_is_refused_by_the_aead_tag() {
        let mut sealed = seal("pw", &sample()).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        let error = open("pw", &sealed).unwrap_err();
        assert!(error.contains("damaged"), "unexpected error: {error}");
    }

    #[test]
    fn a_truncated_bundle_is_refused_rather_than_partly_read() {
        let sealed = seal("pw", &sample()).unwrap();
        let error = open("pw", &sealed[..sealed.len() - 32]).unwrap_err();
        assert!(!error.is_empty());
        assert!(open("pw", &sealed[..4]).is_err());
    }

    #[test]
    fn a_newer_format_version_is_named_not_mistaken_for_corruption() {
        let mut sealed = seal("pw", &sample()).unwrap();
        sealed[MAGIC.len()] = FORMAT_VERSION + 1;
        let error = open("pw", &sealed).unwrap_err();
        assert!(error.contains("format version"), "unexpected error: {error}");
    }

    #[test]
    fn a_newer_contents_version_asks_for_an_update_instead_of_importing_half() {
        let mut bundle = sample();
        bundle.schema_version = SCHEMA_VERSION + 1;
        let sealed = seal("pw", &bundle).unwrap();
        let error = open("pw", &sealed).unwrap_err();
        assert!(error.contains("newer srelens"), "unexpected error: {error}");
    }

    // --- collect -----------------------------------------------------------

    fn sources<'a>(
        base: &'a Path,
        paths: &'a [PathBuf],
        settings: BTreeMap<String, Value>,
    ) -> ExportSources<'a> {
        ExportSources {
            base,
            kubeconfig_paths: paths,
            settings,
            secrets: None,
            app_version: "test".into(),
            created: "2026-09-21T10:00:00Z".into(),
        }
    }

    #[test]
    fn export_carries_kubeconfig_contents_and_drops_the_machine_local_paths() {
        let base = temp_dir("collect");
        let kube = base.join("a.yaml");
        fs::write(&kube, KUBECONFIG).unwrap();
        let settings = BTreeMap::from([
            (KUBECONFIG_FILES_KEY.to_string(), json!(["/Users/ann/.kube/config"])),
            ("srelens.defaultNamespace".to_string(), json!("default")),
        ]);

        let bundle = collect(sources(&base, &[kube], settings));

        assert!(!bundle.settings.contains_key(KUBECONFIG_FILES_KEY));
        assert_eq!(bundle.settings.len(), 1);
        assert_eq!(bundle.kubeconfigs.len(), 1);
        assert_eq!(bundle.kubeconfigs[0].content, KUBECONFIG);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn two_kubeconfigs_named_config_both_survive_under_distinct_names() {
        let base = temp_dir("names");
        let one = base.join("one");
        let two = base.join("two");
        fs::create_dir_all(&one).unwrap();
        fs::create_dir_all(&two).unwrap();
        fs::write(one.join("config"), KUBECONFIG).unwrap();
        fs::write(two.join("config"), OTHER_KUBECONFIG).unwrap();

        let bundle = collect(sources(
            &base,
            &[one.join("config"), two.join("config")],
            BTreeMap::new(),
        ));

        assert_eq!(bundle.kubeconfigs.len(), 2);
        assert_eq!(bundle.kubeconfigs[0].name, "config");
        assert_eq!(bundle.kubeconfigs[1].name, "config-2");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn the_same_kubeconfig_reached_by_two_paths_is_carried_once() {
        let base = temp_dir("dupe");
        let a = base.join("a.yaml");
        let b = base.join("b.yaml");
        fs::write(&a, KUBECONFIG).unwrap();
        fs::write(&b, KUBECONFIG).unwrap();

        let bundle = collect(sources(&base, &[a, b], BTreeMap::new()));

        assert_eq!(bundle.kubeconfigs.len(), 1);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn skills_and_prompts_come_from_their_directories_and_dotfiles_do_not() {
        let base = temp_dir("groups");
        fs::create_dir_all(skills_dir(&base)).unwrap();
        fs::create_dir_all(prompts_dir(&base)).unwrap();
        fs::write(skills_dir(&base).join("triage.md"), "# triage").unwrap();
        fs::write(skills_dir(&base).join(".DS_Store"), "junk").unwrap();
        fs::write(prompts_dir(&base).join("why-pending.md"), "# why").unwrap();

        let bundle = collect(sources(&base, &[], BTreeMap::new()));

        assert_eq!(bundle.skills.len(), 1, "the dotfile should not be carried");
        assert_eq!(bundle.skills[0].name, "triage.md");
        assert_eq!(bundle.prompts[0].name, "why-pending.md");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn installed_apps_are_listed_from_the_inventory_without_re_modelling_it() {
        let base = temp_dir("apps");
        fs::write(
            extensions_inventory_path(&base),
            // Deliberately carries fields this module knows nothing about:
            // the real inventory grows, and a listing must not break with it.
            json!({
                "schemaVersion": 1,
                "nextRevision": 4,
                "plugins": [
                    {"manifest": {"id": "acme.logs", "name": "Logs", "version": "1.2.0"},
                     "grants": ["k8s.listPods"], "enabled": true},
                    {"manifest": {"version": "9"}},
                ]
            })
            .to_string(),
        )
        .unwrap();

        let bundle = collect(sources(&base, &[], BTreeMap::new()));

        assert_eq!(
            bundle.extensions,
            vec![BundledExtension {
                id: "acme.logs".into(),
                name: "Logs".into(),
                version: "1.2.0".into()
            }],
            "an entry with no id is not an app anyone can reinstall"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn an_empty_vault_does_not_become_an_empty_secrets_group() {
        let base = temp_dir("nosecrets");
        let mut s = sources(&base, &[], BTreeMap::new());
        s.secrets = Some(Secrets::default());
        assert!(collect(s).secrets.is_none());
        let _ = fs::remove_dir_all(&base);
    }

    // --- summary -----------------------------------------------------------

    #[test]
    fn the_summary_names_secrets_without_carrying_their_values() {
        let mut bundle = sample();
        bundle.secrets = Some(Secrets {
            mcp_token: Some("deadbeef".into()),
            llm_keys: BTreeMap::from([("anthropic".into(), "sk-ant-secret".into())]),
        });
        let summary = bundle.summary();
        assert_eq!(summary.secret_keys, vec!["anthropic".to_string()]);
        assert!(summary.has_mcp_token);
        let rendered = serde_json::to_string(&summary).unwrap();
        assert!(!rendered.contains("sk-ant-secret"), "the key value reached the preview");
        assert!(!rendered.contains("deadbeef"), "the token reached the preview");
    }

    #[test]
    fn an_absent_or_blank_mcp_token_does_not_read_as_one_being_present() {
        let mut bundle = sample();
        bundle.secrets = Some(Secrets { mcp_token: Some(String::new()), llm_keys: BTreeMap::new() });
        assert!(!bundle.summary().has_mcp_token);
    }

    // --- import ------------------------------------------------------------

    #[test]
    fn import_writes_kubeconfigs_where_the_app_already_looks_for_them() {
        let base = temp_dir("import");
        let report = apply_files(&base, &sample(), &[Group::Kubeconfigs], &[]).unwrap();

        assert_eq!(report.kubeconfigs_added, vec!["config".to_string()]);
        let written = kubeconfigs_dir(&base).join("config");
        assert_eq!(fs::read_to_string(written).unwrap(), KUBECONFIG);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn importing_the_same_bundle_twice_changes_nothing_the_second_time() {
        let base = temp_dir("idempotent");
        apply_files(&base, &sample(), &[Group::Kubeconfigs, Group::Skills], &[]).unwrap();
        let existing = vec![kubeconfigs_dir(&base).join("config")];

        let second =
            apply_files(&base, &sample(), &[Group::Kubeconfigs, Group::Skills], &existing).unwrap();

        assert!(second.kubeconfigs_added.is_empty());
        assert_eq!(second.kubeconfigs_already_present, vec!["config".to_string()]);
        assert!(second.skills_added.is_empty());
        assert!(
            second.skills_kept_local.is_empty(),
            "an identical local skill is not a conflict worth reporting"
        );
        assert_eq!(fs::read_dir(kubeconfigs_dir(&base)).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_kubeconfig_already_read_from_the_home_directory_is_not_duplicated() {
        let base = temp_dir("present");
        let home = temp_dir("present-home");
        let home_config = home.join("config");
        fs::write(&home_config, KUBECONFIG).unwrap();

        let report = apply_files(&base, &sample(), &[Group::Kubeconfigs], &[home_config]).unwrap();

        assert!(report.kubeconfigs_added.is_empty());
        assert_eq!(report.kubeconfigs_already_present, vec!["config".to_string()]);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_locally_edited_skill_is_kept_and_reported_never_overwritten() {
        let base = temp_dir("conflict");
        fs::create_dir_all(skills_dir(&base)).unwrap();
        fs::write(skills_dir(&base).join("triage.md"), "# my own version").unwrap();

        let report = apply_files(&base, &sample(), &[Group::Skills], &[]).unwrap();

        assert_eq!(report.skills_kept_local, vec!["triage.md".to_string()]);
        assert!(report.skills_added.is_empty());
        assert_eq!(
            fs::read_to_string(skills_dir(&base).join("triage.md")).unwrap(),
            "# my own version"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn an_unselected_group_is_not_written() {
        let base = temp_dir("selection");
        let report = apply_files(&base, &sample(), &[Group::Skills], &[]).unwrap();
        assert!(report.kubeconfigs_added.is_empty());
        assert!(!kubeconfigs_dir(&base).exists());
        assert_eq!(report.skills_added, vec!["triage.md".to_string()]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_bundled_file_cannot_escape_its_directory() {
        let base = temp_dir("traversal");
        let mut bundle = sample();
        bundle.skills = vec![
            BundledFile { name: "../../escaped.md".into(), content: "x".into() },
            BundledFile { name: "..".into(), content: "y".into() },
            BundledFile { name: "C:\\Windows\\evil.md".into(), content: "z".into() },
        ];

        apply_files(&base, &bundle, &[Group::Skills], &[]).unwrap();

        assert!(!base.join("escaped.md").exists());
        assert!(!base.parent().unwrap().join("escaped.md").exists());
        for entry in fs::read_dir(skills_dir(&base)).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(!name.contains(['/', '\\']) && name != "..", "unsafe name written: {name}");
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_bundled_file_that_is_not_a_kubeconfig_is_never_written() {
        let base = temp_dir("invalid");
        let mut bundle = sample();
        bundle.kubeconfigs =
            vec![BundledFile { name: "notes.yaml".into(), content: "just: a mapping".into() }];

        let report = apply_files(&base, &bundle, &[Group::Kubeconfigs], &[]).unwrap();

        assert!(report.kubeconfigs_added.is_empty());
        assert_eq!(report.kubeconfigs_rejected, vec!["notes.yaml".to_string()]);
        // Not "already here": that would be a claim about this machine, and
        // nothing checked it. The two facts are opposite.
        assert!(report.kubeconfigs_already_present.is_empty());
        assert!(!kubeconfigs_dir(&base).join("notes.yaml").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_different_kubeconfig_with_a_taken_name_lands_beside_it_not_over_it() {
        let base = temp_dir("collide");
        fs::create_dir_all(kubeconfigs_dir(&base)).unwrap();
        fs::write(kubeconfigs_dir(&base).join("config"), OTHER_KUBECONFIG).unwrap();

        let report = apply_files(&base, &sample(), &[Group::Kubeconfigs], &[]).unwrap();

        assert_eq!(report.kubeconfigs_added, vec!["config-2".to_string()]);
        assert_eq!(
            fs::read_to_string(kubeconfigs_dir(&base).join("config")).unwrap(),
            OTHER_KUBECONFIG,
            "the existing file was replaced"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn importable_settings_never_relocate_clusters_to_the_old_machines_paths() {
        let mut bundle = sample();
        bundle.settings.insert(KUBECONFIG_FILES_KEY.into(), json!(["/Users/ann/.kube/config"]));
        assert!(!importable_settings(&bundle).contains_key(KUBECONFIG_FILES_KEY));
    }

    #[test]
    fn sanitize_name_reduces_anything_to_a_bare_safe_name() {
        assert_eq!(sanitize_name("../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize_name(".."), "");
        assert_eq!(sanitize_name("/"), "");
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name("prod eu.yaml"), "prod-eu.yaml");
        assert_eq!(sanitize_name(&"a".repeat(200)).len(), 96);
    }

    #[test]
    fn read_bundle_file_reports_a_missing_file_as_a_read_failure() {
        let base = temp_dir("missing");
        let error = read_bundle_file(&base.join("nope.srelens")).unwrap_err();
        assert!(error.starts_with("read "), "unexpected error: {error}");
        let _ = fs::remove_dir_all(&base);
    }
}
