//! The rules every URL the extension host fetches is held to: the catalog's own
//! downloads, and each request an app makes through `network.http` (#568).
//!
//! Lifted out of the catalog, which was the first host code to fetch a URL it
//! had not written, so that both callers share one reading of "which URL may be
//! fetched, where may it redirect, and how much may come back":
//!
//! - HTTPS, with no credentials or fragment in the URL;
//! - plain HTTP only to this computer (loopback), and only where a caller
//!   allows it — the catalog never does;
//! - every redirect is checked again by the caller's own rule, at most
//!   [`MAX_REDIRECTS`] times;
//! - bounded time and a bounded body.
use reqwest::Url;
use std::{error::Error as StdError, fmt, io::Read as _, net::Ipv6Addr, time::Duration};

/// How long a connection may take to open.
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a whole request may take, body included.
pub(super) const TIMEOUT: Duration = Duration::from_secs(20);
/// How many redirects one request may follow.
pub(super) const MAX_REDIRECTS: usize = 4;

/// What a URL may be, beyond HTTPS without credentials or a fragment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct UrlRules {
    /// Whether a port may be named. The catalog's GitHub assets never need one.
    pub ports: bool,
    /// Whether plain `http` may reach a loopback host.
    pub loopback_http: bool,
}

/// This computer: `localhost`, `127.0.0.0/8` or `::1`. Any other name is not
/// taken on trust, whatever it resolves to.
pub(super) fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address == Ipv6Addr::LOCALHOST,
        None => false,
    }
}

/// Why `url` may not be fetched under `rules`. The reason never names the URL:
/// it may be a setting's value, and it is written to errors a log or an MCP
/// client may keep.
pub(super) fn check_url(url: &Url, rules: UrlRules) -> Result<(), &'static str> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("A fetched URL carries no user name, password or fragment");
    }
    if url.host().is_none() {
        return Err("A fetched URL names a host");
    }
    if url.port().is_some() && !rules.ports {
        return Err("This URL may not name a port");
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if rules.loopback_http && is_loopback(url) => Ok(()),
        "http" if is_loopback(url) => Err(
            "Plain HTTP to this computer is off for this app; turn on \"Allow plain HTTP to this computer\" in its details, or use https",
        ),
        "http" => Err("Plain HTTP reaches only this computer (loopback); use https"),
        _ => Err("A fetched URL uses https"),
    }
}

/// A redirect refused by the caller's rule, carrying the caller's reason.
#[derive(Debug)]
pub(super) struct RedirectRefused(pub &'static str);

impl fmt::Display for RedirectRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl StdError for RedirectRefused {}

/// A redirect policy that follows a hop only when `allowed` accepts its URL,
/// and at most [`MAX_REDIRECTS`] of them. `allowed` says why not, and that is
/// what the caller is told.
pub(super) fn redirects(
    allowed: impl Fn(&Url) -> Result<(), &'static str> + Send + Sync + 'static,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        // `previous` holds the first URL too, so this follows `MAX_REDIRECTS` hops.
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error(RedirectRefused("The server redirected too many times"));
        }
        match allowed(attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(why) => attempt.error(RedirectRefused(why)),
        }
    })
}

/// The reason a redirect was refused, when `error` is one.
pub(super) fn redirect_refusal(error: &reqwest::Error) -> Option<&'static str> {
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(refused) = cause.downcast_ref::<RedirectRefused>() {
            return Some(refused.0);
        }
        source = cause.source();
    }
    None
}

/// Rustls needs one crypto provider. The first network call may come before any
/// kube client exists, and workspace feature unification can link two.
pub(super) fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Reads at most `limit` bytes of a blocking body, refusing one that is longer.
pub(super) fn read_limited_blocking(
    body: impl std::io::Read,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    body.take(limit as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > limit {
        return Err(too_large(limit));
    }
    Ok(raw)
}

/// Reads at most `limit` bytes of `response`, refusing one that says or turns out
/// to be longer. Stops reading at the first byte past the limit.
pub(super) async fn read_limited(
    response: &mut reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(BodyError::TooLarge(too_large(limit)));
    }
    let mut raw = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(BodyError::Read)? {
        if raw.len() + chunk.len() > limit {
            return Err(BodyError::TooLarge(too_large(limit)));
        }
        raw.extend_from_slice(&chunk);
    }
    Ok(raw)
}

/// Why a body could not be read whole.
pub(super) enum BodyError {
    TooLarge(String),
    Read(reqwest::Error),
}

fn too_large(limit: usize) -> String {
    if limit >= 1024 * 1024 && limit.is_multiple_of(1024 * 1024) {
        format!("The response is larger than {} MiB", limit / (1024 * 1024))
    } else {
        format!("The response is larger than {limit} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[test]
    fn https_only_without_credentials_fragments_or_unasked_ports() {
        let catalog = UrlRules::default();
        assert!(check_url(&url("https://github.com/a/b"), catalog).is_ok());
        for refused in [
            "http://github.com/a/b",
            "https://user@github.com/a/b",
            "https://user:pass@github.com/a/b",
            "https://github.com/a/b#top",
            "https://github.com:8443/a/b",
            "ftp://github.com/a/b",
            "file:///etc/passwd",
            "http://localhost/a",
        ] {
            assert!(check_url(&url(refused), catalog).is_err(), "{refused}");
        }
        let app = UrlRules {
            ports: true,
            loopback_http: false,
        };
        assert!(check_url(&url("https://prometheus.internal:9090/api"), app).is_ok());
    }

    #[test]
    fn plain_http_reaches_loopback_only_and_only_when_allowed() {
        let off = UrlRules {
            ports: true,
            loopback_http: false,
        };
        let on = UrlRules {
            ports: true,
            loopback_http: true,
        };
        for loopback in [
            "http://localhost:9090/",
            "http://127.0.0.1:9090/",
            "http://127.1.2.3/",
            "http://[::1]:9090/",
        ] {
            let why = check_url(&url(loopback), off).unwrap_err();
            assert!(why.contains("Allow plain HTTP"), "{loopback}: {why}");
            assert!(check_url(&url(loopback), on).is_ok(), "{loopback}");
        }
        for elsewhere in [
            "http://prometheus.internal/",
            "http://10.0.0.5:9090/",
            "http://[::2]/",
            "http://localhost.evil.example/",
            "http://0.0.0.0/",
        ] {
            let why = check_url(&url(elsewhere), on).unwrap_err();
            assert!(why.contains("only this computer"), "{elsewhere}: {why}");
        }
    }

    #[test]
    fn no_refusal_names_the_url() {
        for raw in [
            "http://prometheus.secret-team.example/",
            "https://user:hunter2@prometheus.secret-team.example/",
        ] {
            let why = check_url(&url(raw), UrlRules::default()).unwrap_err();
            assert!(
                !why.contains("secret-team") && !why.contains("hunter2"),
                "{why}"
            );
        }
    }

    #[test]
    fn a_blocking_body_past_the_limit_is_refused() {
        assert_eq!(read_limited_blocking(&b"abcd"[..], 4).unwrap(), b"abcd");
        assert!(read_limited_blocking(&b"abcde"[..], 4)
            .unwrap_err()
            .contains("larger than 4 bytes"));
        assert!(too_large(4 * 1024 * 1024).contains("4 MiB"));
    }
}
