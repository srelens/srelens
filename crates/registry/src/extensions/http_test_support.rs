//! A plain HTTP/1.1 server on loopback, standing in for Prometheus, Loki or
//! Tempo, and an unlocked vault: what the `network.http` (#568) and provider
//! (#569) tests drive the broker's real client against.
use serde_json::Value;
use srelens_plugin_host::{HostRule, SecretStore, SecretValue};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use url::Url;

/// What the test server answers one request with.
#[derive(Clone)]
pub(super) enum Reply {
    Json(Value),
    Text(&'static str),
    Status(u16, &'static str),
    Redirect(String),
    /// A body of `len` bytes; with `length`, announced in `Content-Length`,
    /// else streamed until the connection closes.
    Large {
        len: usize,
        length: bool,
    },
    /// Waits this long before answering at all.
    Slow(Duration),
}

/// Headers that carry a credential. The test server keeps only their digest, so a
/// capture never holds a secret in the clear.
pub(super) const SECRET_HEADERS_SEEN: &[&str] = &["authorization", "dd-api-key"];

/// How the test server records a secret-bearing header's value.
pub(super) fn digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

/// A stand-in secret made at run time, so no credential-shaped value is written in
/// the source. It is still handled as a secret everywhere: never printed, and kept
/// by the test server only as a digest.
pub(super) fn stand_in_secret(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{label}-not-a-credential-{nanos:x}")
}

/// One request as the server saw it.
#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub target: String,
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub(super) struct Server {
    pub addr: SocketAddr,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    pub fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{path}", self.addr)).unwrap()
    }
    pub fn rule(&self) -> HostRule {
        HostRule::parse(&self.addr.to_string()).unwrap()
    }
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// A plain HTTP/1.1 server on 127.0.0.1 answering each request with
/// `answer(request target)`, one request per connection.
pub(super) async fn server(answer: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Server {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let answer = Arc::new(answer);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let log = log.clone();
            let answer = answer.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).await.is_err() {
                    return;
                }
                let target = line.split(' ').nth(1).unwrap_or_default().to_owned();
                let mut headers = Vec::new();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).await.unwrap_or(0) == 0 {
                        break;
                    }
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some((key, value)) = header.split_once(':') {
                        let (key, value) = (key.trim(), value.trim());
                        let kept =
                            if SECRET_HEADERS_SEEN.contains(&key.to_ascii_lowercase().as_str()) {
                                digest(value)
                            } else {
                                value.to_owned()
                            };
                        headers.push((key.to_owned(), kept));
                    }
                }
                log.lock().unwrap().push(Seen {
                    target: target.clone(),
                    headers,
                });
                let reply = answer(&target);
                let mut stream = reader.into_inner();
                let (head, body): (String, Vec<u8>) = match reply {
                    Reply::Json(value) => {
                        let body = value.to_string().into_bytes();
                        (
                            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len()),
                            body,
                        )
                    }
                    Reply::Text(text) => (
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n", text.len()),
                        text.as_bytes().to_vec(),
                    ),
                    Reply::Status(code, reason) => (
                        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\n"),
                        Vec::new(),
                    ),
                    Reply::Redirect(location) => (
                        format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n"),
                        Vec::new(),
                    ),
                    Reply::Large { len, length } => (
                        if length {
                            format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {len}\r\n")
                        } else {
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n".to_owned()
                        },
                        vec![b'x'; len],
                    ),
                    Reply::Slow(delay) => {
                        tokio::time::sleep(delay).await;
                        ("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n".to_owned(), Vec::new())
                    }
                };
                let _ = stream
                    .write_all(format!("{head}Connection: close\r\n\r\n").as_bytes())
                    .await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Server { addr, seen }
}

/// The desktop vault as an unlocked one behaves.
#[derive(Default)]
pub(super) struct Vault(Mutex<BTreeMap<String, String>>);

impl SecretStore for Vault {
    fn status(&self) -> Result<(), String> {
        Ok(())
    }
    fn put(&self, key: &str, value: &SecretValue) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(key.into(), value.expose().into());
        Ok(())
    }
    fn contains(&self, key: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().contains_key(key))
    }
    fn retain(&self, keep: &BTreeSet<String>) -> Result<(), String> {
        self.0.lock().unwrap().retain(|key, _| keep.contains(key));
        Ok(())
    }
    fn reveal(&self, key: &str) -> Result<Option<SecretValue>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .map(SecretValue::new))
    }
}
