//! Tauri commands for desktop managed cluster OIDC: open the browser, capture
//! the code on a one-shot 127.0.0.1 listener (RFC 8252 loopback), exchange it,
//! and store the sealed token — mirroring the web /auth/cluster/* routes.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use srelens_kube::client_cache::ClientCache;
use srelens_server::cluster_oidc::{begin_login, exchange_code};
use srelens_server::cluster_tokens::StoredToken;
use srelens_server::unix_now;

use crate::cluster_oidc::DesktopClusterOidc;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterRow {
    pub key: String,
    pub issuer: String,
    pub client_id: String,
    pub contexts: Vec<String>,
    pub signed_in: bool,
    pub expires_at: Option<i64>,
}

/// The `code`/`state` pair captured from the browser's loopback redirect.
type LoopbackResult = Result<(String, String), String>;

/// Loopback listener: bind an ephemeral 127.0.0.1 port, return it + a receiver
/// that yields the `code`/`state` from the single callback GET (or times out).
/// How long the loopback listener waits for the browser to complete sign-in.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// Extract `(code, state)` from an HTTP request line
/// (`GET /auth/cluster/callback?code=...&state=... HTTP/1.1`). Returns None
/// when the line carries no `code` (e.g. a browser preconnect or favicon hit).
fn parse_callback_line(line: &str) -> Option<(String, String)> {
    let query = line.split_whitespace().nth(1)?;
    let qs = query.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in qs.split('&') {
        if let Some(v) = pair.strip_prefix("code=") {
            code = Some(urldecode(v));
        }
        if let Some(v) = pair.strip_prefix("state=") {
            state = Some(urldecode(v));
        }
    }
    Some((code?, state.unwrap_or_default()))
}

fn spawn_loopback() -> Result<(u16, std::sync::mpsc::Receiver<LoopbackResult>), String> {
    let port_env = std::env::var("SRELENS_CLUSTER_LOGIN_PORT")
        .ok()
        .and_then(|s| s.parse().ok());
    let listener =
        TcpListener::bind(("127.0.0.1", port_env.unwrap_or(0))).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    // Poll accept() so the thread can self-terminate on the deadline instead of
    // blocking forever (which would leak the thread and pin the port).
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let deadline = Instant::now() + CALLBACK_TIMEOUT;
        let body = "<html><body style=\"font-family:sans-serif\"><h3>Signed in to your cluster.</h3><p>You can close this tab and return to srelens.</p></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let result = loop {
            if Instant::now() >= deadline {
                break Err("sign-in timed out".to_string());
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // The listener is non-blocking so the accept loop can honor
                    // the deadline, but the accepted socket inherits O_NONBLOCK
                    // on BSD/macOS — a bare read() would then return WouldBlock
                    // before the browser's request bytes land and drop the real
                    // callback. Put this socket back into blocking mode with a
                    // bounded read timeout so we actually wait for the request.
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let line = req.lines().next().unwrap_or("");
                    // Keep looping past connections that carry no auth code (a
                    // browser preconnect or /favicon.ico), still bounded by the
                    // deadline; only a real callback ends the loop.
                    if let Some((code, state)) = parse_callback_line(line) {
                        let _ = stream.write_all(response.as_bytes());
                        break Ok((code, state));
                    }
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => break Err(e.to_string()),
            }
        };
        let _ = tx.send(result);
        // `listener` drops here → the port is released even on timeout.
    });
    Ok((port, rx))
}

/// Minimal percent-decoding for the code/state query values.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
pub async fn cluster_login(
    app: tauri::AppHandle,
    oidc: State<'_, Arc<DesktopClusterOidc>>,
    cache: State<'_, Arc<ClientCache>>,
    key: String,
) -> Result<(), String> {
    let cfg = oidc
        .registry()
        .config_for_key(&key)
        .ok_or_else(|| "unknown cluster".to_string())?;
    let (port, rx) = spawn_loopback()?;
    let redirect = format!("http://127.0.0.1:{port}/auth/cluster/callback");
    let begin = begin_login(&cfg, &redirect).await?;
    app.opener()
        .open_url(begin.auth_url, None::<&str>)
        .map_err(|e| e.to_string())?;
    // Wait for the browser callback. The listener thread self-terminates on its
    // own CALLBACK_TIMEOUT; this recv is a slightly-longer backstop.
    let (code, state) = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(CALLBACK_TIMEOUT + Duration::from_secs(30))
            .map_err(|_| "sign-in timed out".to_string())?
    })
    .await
    .map_err(|e| e.to_string())??;
    if state != begin.state {
        return Err("state mismatch".to_string());
    }
    let refreshed = exchange_code(&cfg, &redirect, &code, &begin.pkce_verifier, unix_now()).await?;
    oidc.db
        .put_cluster_token(
            oidc.user_id,
            &key,
            &oidc.master_key,
            &StoredToken {
                id_token: refreshed.id_token,
                refresh_token: refreshed.refresh_token,
                expires_at: refreshed.expires_at,
            },
        )
        .await?;
    // Rebuild any cached client so the fresh token is picked up.
    cache.clear().await;
    Ok(())
}

#[tauri::command]
pub async fn cluster_logout(
    oidc: State<'_, Arc<DesktopClusterOidc>>,
    cache: State<'_, Arc<ClientCache>>,
    key: String,
) -> Result<(), String> {
    oidc.db.delete_cluster_token(oidc.user_id, &key).await?;
    cache.clear().await;
    Ok(())
}

#[tauri::command]
pub async fn list_clusters(
    oidc: State<'_, Arc<DesktopClusterOidc>>,
) -> Result<Vec<ClusterRow>, String> {
    let now = unix_now();
    let mut out = Vec::new();
    for (key, cfg, contexts) in oidc.registry().oidc_clusters() {
        let token = oidc
            .db
            .get_cluster_token(oidc.user_id, &key, &oidc.master_key)
            .await
            .ok()
            .flatten();
        let (signed_in, expires_at) = match &token {
            Some(t) => (t.expires_at > now, Some(t.expires_at)),
            None => (false, None),
        };
        out.push(ClusterRow {
            key,
            issuer: cfg.issuer,
            client_id: cfg.client_id,
            contexts,
            signed_in,
            expires_at,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urldecode_handles_percent_and_plus() {
        assert_eq!(urldecode("a%3Ab+c"), "a:b c");
    }

    #[test]
    fn parses_callback_query_from_request_line() {
        let line = "GET /auth/cluster/callback?code=abc123&state=xyz%3A789 HTTP/1.1";
        assert_eq!(
            parse_callback_line(line),
            Some(("abc123".to_string(), "xyz:789".to_string())),
        );
    }

    #[test]
    fn callback_line_without_code_is_ignored() {
        // A browser preconnect / favicon hit carries no auth code — the listener
        // must keep waiting, not fail the sign-in.
        assert_eq!(parse_callback_line("GET /favicon.ico HTTP/1.1"), None);
        assert_eq!(parse_callback_line("GET /auth/cluster/callback HTTP/1.1"), None);
    }
}
