//! A fake Kubernetes API server, so a capability can be driven end to end without a
//! cluster: this crate's own tests, and, through the `test-support` feature, the
//! registry's (#728). Test support, not an API.

/// The client [`fake_api`] returns, named for crates that do not depend on kube.
pub use kube::Client;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// One request the fake API server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// The URI path, e.g. `/api/v1/namespaces/team/secrets`.
    pub path: String,
    /// The query string, empty when there is none.
    pub query: String,
    /// The `Accept` header, which says whether a list asked for metadata only.
    pub accept: String,
}

/// A client whose API server answers every request with `respond(request)` as a 200,
/// and records each request it answered.
pub fn fake_api<F>(respond: F) -> (Client, Arc<Mutex<Vec<Seen>>>)
where
    F: Fn(&Seen) -> Value + Send + Sync + 'static,
{
    let respond = Arc::new(respond);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
        let seen = Seen {
            path: request.uri().path().to_owned(),
            query: request.uri().query().unwrap_or("").to_owned(),
            accept: request
                .headers()
                .get(http::header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_owned(),
        };
        let body = respond(&seen);
        recorded.lock().unwrap().push(seen);
        async move {
            Ok::<_, std::convert::Infallible>(
                http::Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(kube::client::Body::from(body.to_string().into_bytes()))
                    .unwrap(),
            )
        }
    });
    (Client::new(service, "default"), requests)
}
