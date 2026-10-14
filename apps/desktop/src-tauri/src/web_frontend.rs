//! The frontend `serve` (the web mode) hands the server: the copy Tauri
//! embedded for the window, rather than a second one linked in through
//! srelens-server.

use std::borrow::Cow;
use std::sync::Arc;

use srelens_server::assets::AssetSource;

/// Tauri's context for this build: its config and the frontend it embedded.
pub(crate) fn context() -> tauri::Context {
    tauri::generate_context!()
}

/// The frontend Tauri embedded in this build. A build without the
/// `custom-protocol` feature embeds none, because its window loads the dev
/// server, so its `serve` answers the frontend's paths with 404; a debug
/// `srelens-server` serves `apps/desktop/dist` from disk instead.
pub fn assets() -> AssetSource {
    asset_source(context().assets)
}

fn asset_source<R: tauri::Runtime>(assets: Box<dyn tauri::Assets<R>>) -> AssetSource {
    let assets: Arc<dyn tauri::Assets<R>> = Arc::from(assets);
    // Tauri keys by the path from the root ("/index.html"); `AssetKey` adds the
    // slash. Its files are compressed, so each fetch is a fresh decompressed copy.
    Arc::new(move |path| {
        assets
            .get(&path.into())
            .map(|data| Cow::Owned(data.into_owned()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};

    /// A bundle as Tauri keys it: paths from the root, with a leading slash.
    struct Bundle;

    impl tauri::Assets<tauri::test::MockRuntime> for Bundle {
        fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
            match key.as_ref() {
                "/index.html" => Some(Cow::Borrowed(b"<p>shell</p>")),
                "/assets/app.js" => Some(Cow::Borrowed(b"app();")),
                _ => None,
            }
        }
        fn iter(&self) -> Box<AssetsIter<'_>> {
            Box::new(std::iter::empty())
        }
        fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
            Box::new(std::iter::empty())
        }
    }

    #[test]
    fn serve_reads_the_files_tauri_embedded_by_the_path_the_server_asks_for() {
        // The server asks by the path under the bundle root, with no leading slash.
        let source = asset_source(Box::new(Bundle));
        assert_eq!(source("index.html").as_deref(), Some(&b"<p>shell</p>"[..]));
        assert_eq!(source("assets/app.js").as_deref(), Some(&b"app();"[..]));
        assert_eq!(source("assets/missing.js"), None);
    }
}
