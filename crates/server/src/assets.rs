//! The frontend: serves the Vite build of `apps/desktop/dist` with an SPA
//! fallback to index.html for client-side routes. This crate embeds no copy of
//! it. Each binary hands in the one it carries: `srelens-server` embeds its own,
//! and the desktop app's `serve` mode reads the copy Tauri embedded, so the
//! desktop does not ship the bundle twice.

use std::borrow::Cow;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use crate::AppState;

/// A frontend file by its path under the bundle root (`index.html`,
/// `assets/app.js`), or `None` when the bundle has no such file.
pub type AssetSource = Arc<dyn Fn(&str) -> Option<Cow<'static, [u8]>> + Send + Sync>;

/// Decide which file a GET path resolves to, and fetch it: the file itself when
/// it exists, `index.html` for extension-less SPA routes, `None` when neither
/// exists (frontend not built, or a real missing asset like `/app.js`). Each
/// file is fetched once, because the desktop's are decompressed on every fetch.
fn resolve<T>(path: &str, get: impl Fn(&str) -> Option<T>) -> Option<(&str, T)> {
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };
    if let Some(data) = get(candidate) {
        return Some((candidate, data));
    }
    if candidate.contains('.') {
        return None;
    }
    get("index.html").map(|data| ("index.html", data))
}

/// Router fallback handler: serve the frontend the binary handed in.
pub async fn serve_asset(State(state): State<AppState>, uri: Uri) -> Response {
    match resolve(uri.path(), |p| (state.assets)(p)) {
        Some((path, data)) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                data,
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

    /// A bundle of `files`, each holding its own path, so a test sees which
    /// file was fetched as well as which path it resolved to.
    fn dist<'a>(files: &'a [&'a str]) -> impl Fn(&str) -> Option<String> + 'a {
        move |p: &str| files.contains(&p).then(|| p.to_owned())
    }

    fn resolved(path: &str, files: &[&str]) -> Option<(String, String)> {
        resolve(path, dist(files)).map(|(at, data)| (at.to_owned(), data))
    }

    fn file(path: &str) -> Option<(String, String)> {
        Some((path.to_owned(), path.to_owned()))
    }

    #[test]
    fn root_serves_index() {
        assert_eq!(resolved("/", &["index.html"]), file("index.html"));
    }

    #[test]
    fn existing_file_is_served_directly() {
        assert_eq!(
            resolved("/assets/app.js", &["index.html", "assets/app.js"]),
            file("assets/app.js")
        );
    }

    #[test]
    fn spa_route_falls_back_to_index() {
        assert_eq!(
            resolved("/settings/clusters", &["index.html"]),
            file("index.html")
        );
    }

    #[test]
    fn missing_file_with_extension_is_none() {
        assert_eq!(resolved("/missing.js", &["index.html"]), None);
    }

    #[test]
    fn unbuilt_frontend_is_none() {
        assert_eq!(resolved("/", &[]), None);
        assert_eq!(resolved("/settings", &[]), None);
    }
}
