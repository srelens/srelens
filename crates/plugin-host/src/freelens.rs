//! Audited compatibility packages. Never extract an archive onto the filesystem
//! or run its npm lifecycle scripts. New packages/versions need a runtime audit.
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::io::Read;

pub const FLUX_SHA256: &str = "27b433c2738e6228cd06c79fe98d0141101180679474dd6e81a56f12aacb4ddf";
pub struct FluxArchive {
    pub name: String,
    pub version: String,
    pub renderer: String,
}
impl FluxArchive {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 1024 * 1024 || format!("{:x}", Sha256::digest(bytes)) != FLUX_SHA256 {
            return Err("Unsupported archive. This compatibility runtime supports the original @freelensapp/fluxcd-extension 5.3.1 release .tgz only; package integrity did not match.".into());
        }
        let mut archive = tar::Archive::new(GzDecoder::new(bytes).take(8 * 1024 * 1024));
        let mut package = None;
        let mut renderer = None;
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            if path
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
                || !entry.header().entry_type().is_file()
            {
                return Err("Archive contains an unsafe path or non-file entry".into());
            }
            if path == std::path::Path::new("package/package.json")
                || path == std::path::Path::new("package/out/renderer/index.js")
            {
                let mut text = String::new();
                entry
                    .take(1024 * 1024)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                if path.ends_with("package.json") {
                    package = Some(text);
                } else {
                    renderer = Some(text);
                }
            }
        }
        let metadata: serde_json::Value =
            serde_json::from_str(&package.ok_or("Missing package.json")?)
                .map_err(|e| e.to_string())?;
        Ok(Self {
            name: metadata["name"]
                .as_str()
                .ok_or("Missing package name")?
                .into(),
            version: metadata["version"]
                .as_str()
                .ok_or("Missing package version")?
                .into(),
            renderer: renderer.ok_or("Missing renderer")?,
        })
    }
}
