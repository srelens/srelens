//! Embedded frontend assets: serves the Vite build of `apps/desktop/dist`
//! with an SPA fallback to index.html for client-side routes. In debug builds
//! rust-embed reads the folder from disk at runtime; release builds embed the
//! files into the binary.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../apps/desktop/dist"]
struct Assets;

/// Decide which embedded file a GET path resolves to: the file itself when it
/// exists, `index.html` for extension-less SPA routes, `None` when neither
/// exists (frontend not built, or a real missing asset like `/app.js`).
fn resolve(path: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };
    if exists(candidate) {
        return Some(candidate.to_string());
    }
    if !candidate.contains('.') && exists("index.html") {
        return Some("index.html".to_string());
    }
    None
}

/// Router fallback handler: serve the embedded frontend.
pub async fn serve_asset(uri: Uri) -> Response {
    match resolve(uri.path(), |p| Assets::get(p).is_some()) {
        Some(path) => {
            let file = Assets::get(&path).expect("resolved asset exists");
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            "frontend not built — run `pnpm --filter @srelens/desktop build`",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    fn dist<'a>(files: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |p: &str| files.contains(&p)
    }

    #[test]
    fn root_serves_index() {
        assert_eq!(
            resolve("/", dist(&["index.html"])),
            Some("index.html".to_string())
        );
    }

    #[test]
    fn existing_file_is_served_directly() {
        assert_eq!(
            resolve("/assets/app.js", dist(&["index.html", "assets/app.js"])),
            Some("assets/app.js".to_string())
        );
    }

    #[test]
    fn spa_route_falls_back_to_index() {
        assert_eq!(
            resolve("/settings/clusters", dist(&["index.html"])),
            Some("index.html".to_string())
        );
    }

    #[test]
    fn missing_file_with_extension_is_none() {
        assert_eq!(resolve("/missing.js", dist(&["index.html"])), None);
    }

    #[test]
    fn unbuilt_frontend_is_none() {
        assert_eq!(resolve("/", dist(&[])), None);
        assert_eq!(resolve("/settings", dist(&[])), None);
    }
}
