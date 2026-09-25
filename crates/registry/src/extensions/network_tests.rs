//! `network.http` (#568) against a real socket: a local server on loopback
//! stands in for Prometheus, so the allowlist, every redirect, the limits and
//! the loopback switch are exercised by the client the broker really uses.
use super::*;
use crate::extensions::tests::fake_core;
use srelens_plugin_host::{secret_key, SecretValue, SECRET_STORE_PERMISSION};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// What the test server answers one request with.
#[derive(Clone)]
enum Reply {
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

/// One request as the server saw it.
#[derive(Clone, Debug)]
struct Seen {
    target: String,
    headers: Vec<(String, String)>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

struct Server {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{path}", self.addr)).unwrap()
    }
    fn rule(&self) -> HostRule {
        HostRule::parse(&self.addr.to_string()).unwrap()
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// A plain HTTP/1.1 server on 127.0.0.1 answering each request with
/// `answer(request target)`, one request per connection.
async fn server(answer: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Server {
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
                        headers.push((key.trim().to_owned(), value.trim().to_owned()));
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

fn policy(rules: Vec<HostRule>, loopback_http: bool) -> Policy {
    Policy {
        allowlist: rules,
        loopback_http,
        limits: Limits::default(),
    }
}

fn arguments(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn a_binding_asks_for_a_url_this_host_could_fetch_with_headers_an_app_may_set() {
    for good in [
        json!({"url":"https://api.github.com"}),
        json!({"url":"https://prometheus.internal:9090/prometheus","path":"/api/v1/query","query":{"query":"up"}}),
        json!({"url":"http://localhost:9090","path":"/api/v1/query"}),
        json!({"url":STAND_IN,"headers":{"Accept":"application/json"}}),
        json!({"url":STAND_IN,"secretHeaders":{"Authorization":{"secret":"token","prefix":"Bearer "}}}),
        json!({"url":STAND_IN,"secretHeaders":{"DD-API-KEY":{"secret":"api-key"}}}),
    ] {
        assert!(
            check_arguments(&arguments(good.clone())).is_ok(),
            "{good}: {:?}",
            check_arguments(&arguments(good.clone()))
        );
    }
    for (bad, why) in [
        (
            json!({"url":"http://prometheus.internal"}),
            "only this computer",
        ),
        (json!({"url":"ftp://api.github.com"}), "https"),
        (
            json!({"url":"https://user:pass@api.github.com"}),
            "user name",
        ),
        (json!({"url":"https://api.github.com/#top"}), "fragment"),
        (json!({"url":"not a url"}), "not a URL"),
        (json!({"url":STAND_IN,"path":"api/v1"}), "starts with /"),
        (json!({"url":STAND_IN,"path":"/api/../admin"}), ".."),
        (json!({"url":STAND_IN,"path":"/api/%2E%2E/admin"}), ".."),
        (json!({"url":STAND_IN,"path":"/api?x=1"}), "query"),
        (json!({"url":STAND_IN,"path":"/a b"}), "space"),
        (
            json!({"url":STAND_IN,"headers":{"Authorization":"Bearer abc"}}),
            "credential",
        ),
        (
            json!({"url":STAND_IN,"headers":{"Cookie":"session=abc"}}),
            "credential",
        ),
        (
            json!({"url":STAND_IN,"headers":{"Host":"evil.example"}}),
            "set by the host",
        ),
        (
            json!({"url":STAND_IN,"secretHeaders":{"Proxy-Authorization":{"secret":"token"}}}),
            "set by the host",
        ),
        (
            json!({"url":STAND_IN,"headers":{"bad header":"x"}}),
            "not a header name",
        ),
        (
            json!({"url":STAND_IN,"headers":{"X-Note":"line\nbreak"}}),
            "visible text",
        ),
        (
            json!({"url":STAND_IN,"headers":{"accept":"a"},"secretHeaders":{"Accept":{"secret":"token"}}}),
            "more than once",
        ),
        (
            json!({"url":STAND_IN,"secretHeaders":{"Authorization":{"secret":"${settings.token}"}}}),
            "names a setting",
        ),
        (
            json!({"url":STAND_IN,"secretHeaders":{"Authorization":{"secret":"token","prefix":"Bearer\n"}}}),
            "printable ASCII",
        ),
        (json!({"url":STAND_IN,"method":"POST"}), "unknown field"),
    ] {
        let found = check_arguments(&arguments(bad.clone())).unwrap_err();
        assert!(found.contains(why), "{bad}: {found}");
    }
    let long = "a".repeat(MAX_URL);
    assert!(
        check_arguments(&arguments(json!({"url":STAND_IN,"query":{"q":long}})))
            .unwrap_err()
            .contains("longer than")
    );
}

#[test]
fn the_capability_is_a_declaration_whose_handler_sends_nothing() {
    let capability = capability();
    assert!(capability.takes_secret(SECRET_HEADERS));
    assert_eq!(capability.secret_slots, [SECRET_HEADERS]);
    let url = capability.settable_argument("url").unwrap();
    assert_eq!(url.accepts, [SettingType::Url]);
    assert!(capability.annotations.read_only);
    let refused = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on((capability.handler)(
            json!({"url":"https://api.github.com"}),
        ))
        .unwrap_err();
    assert!(
        refused.to_string().contains("only by the extension broker"),
        "{refused}"
    );
}

#[tokio::test]
async fn plain_http_reaches_an_allowed_loopback_host_only_when_the_person_allows_it() {
    let prometheus =
        server(|_| Reply::Json(json!({"status":"success","data":{"result":[]}}))).await;
    let url = prometheus.url("/api/v1/query?query=up");
    let refused = send(
        url.clone(),
        HeaderMap::new(),
        false,
        policy(vec![prometheus.rule()], false),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("Allow plain HTTP to this computer"),
        "{refused}"
    );
    assert!(prometheus.seen().is_empty(), "refused before connecting");

    let answer = send(
        url,
        HeaderMap::new(),
        false,
        policy(vec![prometheus.rule()], true),
    )
    .await
    .unwrap();
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body["status"], "success");
    assert_eq!(answer.content_type.as_deref(), Some("application/json"));
    assert_eq!(prometheus.seen()[0].target, "/api/v1/query?query=up");
}

#[tokio::test]
async fn a_host_off_the_allowlist_is_never_contacted() {
    let other = server(|_| Reply::Text("hello")).await;
    let allowed = HostRule::parse("127.0.0.1:1").unwrap();
    let refused = send(
        other.url("/"),
        HeaderMap::new(),
        false,
        policy(vec![allowed], true),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("not among this app's network.http hosts"),
        "{refused}"
    );
    assert!(other.seen().is_empty());
}

#[tokio::test]
async fn every_redirect_is_checked_against_the_allowlist_again() {
    let elsewhere = server(|_| Reply::Text("you should not be here")).await;
    let away = elsewhere.url("/stolen").to_string();
    let prometheus = server(move |target| match target {
        "/away" => Reply::Redirect(away.clone()),
        "/moved" => Reply::Redirect("/api/v1/query".into()),
        _ => Reply::Json(json!({"ok":true})),
    })
    .await;
    let refused = send(
        prometheus.url("/away"),
        HeaderMap::new(),
        false,
        policy(vec![prometheus.rule()], true),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("not among this app's network.http hosts"),
        "{refused}"
    );
    assert!(elsewhere.seen().is_empty(), "the redirect was not followed");

    let followed = send(
        prometheus.url("/moved"),
        HeaderMap::new(),
        false,
        policy(vec![prometheus.rule()], true),
    )
    .await
    .unwrap();
    assert_eq!(followed.body, json!({"ok":true}));
}

#[tokio::test]
async fn a_redirect_loop_is_followed_four_times_and_no_further() {
    let looping = server(|_| Reply::Redirect("/again".into())).await;
    let refused = send(
        looping.url("/"),
        HeaderMap::new(),
        false,
        policy(vec![looping.rule()], true),
    )
    .await
    .unwrap_err();
    assert_eq!(refused, "The server redirected too many times");
    // The first request and four redirects.
    assert_eq!(looping.seen().len(), 1 + http_policy::MAX_REDIRECTS);
    assert_eq!(http_policy::MAX_REDIRECTS, 4);
}

#[tokio::test]
async fn a_redirect_is_held_to_the_scheme_rule_too() {
    // An allowed host that redirects to plain HTTP on loopback, while the app may
    // not use plain HTTP: the hop is refused even though the port is allowlisted.
    let target = server(|_| Reply::Text("plain")).await;
    let policy = policy(vec![target.rule()], false);
    assert!(policy
        .check(&target.url("/"))
        .unwrap_err()
        .contains("Allow plain HTTP"));
    assert!(policy
        .check(&Url::parse("https://127.0.0.1/").unwrap())
        .unwrap_err()
        .contains("not among"));
}

#[tokio::test]
async fn a_request_carrying_a_secret_follows_no_redirect_to_another_origin() {
    let elsewhere = server(|_| Reply::Text("elsewhere")).await;
    let away = elsewhere.url("/collect").to_string();
    let grafana = server(move |_| Reply::Redirect(away.clone())).await;
    let rules = vec![grafana.rule(), elsewhere.rule()];
    let mut headers = HeaderMap::new();
    let mut key = HeaderValue::from_static("dd-api-7c1e4f");
    key.set_sensitive(true);
    headers.insert("dd-api-key", key);
    let refused = send(
        grafana.url("/api"),
        headers.clone(),
        true,
        policy(rules.clone(), true),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("does not follow a redirect to another origin"),
        "{refused}"
    );
    assert!(
        elsewhere.seen().is_empty(),
        "the secret did not leave for another origin"
    );
    // Without a secret the same hop is followed, since both hosts are allowed.
    send(
        grafana.url("/api"),
        HeaderMap::new(),
        false,
        policy(rules, true),
    )
    .await
    .unwrap();
    assert_eq!(elsewhere.seen().len(), 1);
}

#[tokio::test]
async fn a_response_past_the_size_limit_is_refused_whether_announced_or_streamed() {
    let big = server(|target| Reply::Large {
        len: 2048,
        length: target == "/announced",
    })
    .await;
    let mut small = policy(vec![big.rule()], true);
    small.limits.body = 1024;
    let refused = send(big.url("/announced"), HeaderMap::new(), false, small)
        .await
        .unwrap_err();
    assert!(refused.contains("larger than 1024 bytes"), "{refused}");
    let mut small = policy(vec![big.rule()], true);
    small.limits.body = 1024;
    let refused = send(big.url("/streamed"), HeaderMap::new(), false, small)
        .await
        .unwrap_err();
    assert!(refused.contains("larger than 1024 bytes"), "{refused}");
    // At the default limit the same body is read whole.
    let whole = send(
        big.url("/streamed"),
        HeaderMap::new(),
        false,
        policy(vec![big.rule()], true),
    )
    .await
    .unwrap();
    assert_eq!(whole.body.as_str().unwrap().len(), 2048);
}

#[tokio::test]
async fn a_server_that_does_not_answer_in_time_is_given_up_on() {
    let slow = server(|_| Reply::Slow(Duration::from_secs(5))).await;
    let mut hurried = policy(vec![slow.rule()], true);
    hurried.limits.total = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let refused = send(slow.url("/"), HeaderMap::new(), false, hurried)
        .await
        .unwrap_err();
    assert!(
        refused.contains("timed out") || refused.contains("longer than"),
        "{refused}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_failed_status_is_an_error_and_a_body_is_decoded_by_its_type() {
    let server = server(|target| match target {
        "/down" => Reply::Status(503, "Service Unavailable"),
        "/text" => Reply::Text("pong"),
        _ => Reply::Status(204, "No Content"),
    })
    .await;
    let rules = || policy(vec![server.rule()], true);
    let refused = send(server.url("/down"), HeaderMap::new(), false, rules())
        .await
        .unwrap_err();
    assert_eq!(refused, "The server answered HTTP 503 Service Unavailable");
    let text = send(server.url("/text"), HeaderMap::new(), false, rules())
        .await
        .unwrap();
    assert_eq!(text.body, json!("pong"));
    let empty = send(server.url("/empty"), HeaderMap::new(), false, rules())
        .await
        .unwrap();
    assert_eq!((empty.status, empty.body), (204, Value::Null));
    assert!(decode(Some("application/json"), b"{not json")
        .unwrap_err()
        .contains("not JSON"));
    assert_eq!(
        decode(Some("application/problem+json"), b"{\"a\":1}").unwrap(),
        json!({"a":1})
    );
    assert!(decode(Some("application/octet-stream"), &[0xff, 0xfe])
        .unwrap_err()
        .contains("UTF-8"));
}

#[tokio::test]
async fn a_failed_connection_says_so_without_naming_the_url() {
    // A port nothing listens on: bind one, then let it go.
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = Url::parse(&format!("http://127.0.0.1:{port}/secret-path")).unwrap();
    let rule = HostRule::parse(&format!("127.0.0.1:{port}")).unwrap();
    let refused = send(url, HeaderMap::new(), false, policy(vec![rule], true))
        .await
        .unwrap_err();
    assert!(
        refused.starts_with("Could not connect to the server"),
        "{refused}"
    );
    assert!(
        !refused.contains("127.0.0.1") && !refused.contains("secret-path"),
        "{refused}"
    );
}

/// The desktop vault as an unlocked one behaves.
#[derive(Default)]
struct Vault(Mutex<BTreeMap<String, String>>);

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

const APP: &str = "org.example.metrics";
const TOKEN: &str = "glc-9f2e-hunter2";

/// A metrics app: one Prometheus query, its URL a setting and its token a
/// secret sent as a bearer header.
fn metrics_manifest() -> Value {
    json!({
        "id":APP, "name":"Metrics", "version":"0.1.0", "srelensApiVersion":"^0.4",
        "kind":"declarative",
        "permissions":[
            {"capability":"network.http","hosts":["${settings.prometheusUrl}"]},
            SECRET_STORE_PERMISSION
        ],
        "settings":[
            {"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true},
            {"id":"token","type":"secret-reference","title":"API token"}
        ],
        "capabilities":[{"name":"up","title":"Targets up","target":"network.http",
            "arguments":{"url":"${settings.prometheusUrl}","path":"/api/v1/query","query":{"query":"up"},
                "headers":{"Accept":"application/json"},
                "secretHeaders":{"Authorization":{"secret":"token","prefix":"Bearer "}}},
            "inputs":[]}],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[]}
    })
}

fn broker(dir: &std::path::Path, vault: Arc<Vault>) -> (std::path::PathBuf, Registry) {
    let path = dir.join("settings.extensions.json");
    let mut reg = Registry::new();
    super::super::register_with_secrets(
        &mut reg,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
        vault,
    );
    (path, reg)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_installed_app_reaches_its_host_through_extensions_read_with_its_secret_injected() {
    let prometheus = server(|_| Reply::Json(json!({"status":"success"}))).await;
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::default());
    let (_, reg) = broker(dir.path(), vault.clone());
    let configure = |input: Value| reg.invoke("extensions.configure", input);
    let installed = configure(
        json!({"action":"install","manifest":metrics_manifest().to_string(),
        "grants":["network.http", SECRET_STORE_PERMISSION]}),
    )
    .await
    .unwrap();
    let revision = installed["plugins"][0]["revision"].clone();
    configure(json!({"action":"settings","id":APP,
        "settings":{"prometheusUrl": format!("http://{}", prometheus.addr)}}))
    .await
    .unwrap();
    let read = || {
        reg.invoke(
            "extensions.read",
            json!({"id":APP,"revision":revision,"capability":"up","context":"kind-dev"}),
        )
    };
    // Plain HTTP to this computer is a person's switch, off until turned on.
    let refused = read().await.unwrap_err().to_string();
    assert!(refused.contains("Allow plain HTTP"), "{refused}");
    configure(json!({"action":"loopbackHttp","id":APP,"allowLoopbackHttp":true}))
        .await
        .unwrap();
    // The secret is not set yet, and nothing is sent without it.
    let refused = read().await.unwrap_err().to_string();
    assert!(refused.contains("is not set"), "{refused}");
    assert!(prometheus.seen().is_empty());
    reg.invoke(
        "extension.secretStore",
        json!({"action":"set","id":APP,"setting":"token","secret":TOKEN}),
    )
    .await
    .unwrap();
    assert!(vault.contains(&secret_key(APP, "token")).unwrap());

    let answer = read().await.unwrap();
    assert_eq!(
        answer,
        json!({"status":200,"contentType":"application/json","body":{"status":"success"}})
    );
    let seen = prometheus.seen();
    assert_eq!(seen[0].target, "/api/v1/query?query=up");
    assert_eq!(
        seen[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(seen[0].header("accept"), Some("application/json"));
    // The value reached the server and nothing else: not the answer, not the inventory.
    assert!(!answer.to_string().contains(TOKEN));
    let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
    assert!(!listed.to_string().contains(TOKEN));
    assert_eq!(listed["plugins"][0]["allowLoopbackHttp"], true);

    // Moving the URL elsewhere moves the allowlist with it, and a stream is never
    // handed a network.http request.
    configure(json!({"action":"loopbackHttp","id":APP,"allowLoopbackHttp":false}))
        .await
        .unwrap();
    let refused = read().await.unwrap_err().to_string();
    assert!(refused.contains("Allow plain HTTP"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_loopback_switch_belongs_to_an_app_that_requests_network_http_and_survives_its_updates()
{
    let dir = tempfile::tempdir().unwrap();
    let (path, reg) = broker(dir.path(), Arc::new(Vault::default()));
    let configure = |input: Value| reg.invoke("extensions.configure", input);
    configure(
        json!({"action":"install","manifest":crate::extensions::tests::manifest(),
        "grants":["k8s.listCustomResource"]}),
    )
    .await
    .unwrap();
    let refused = configure(
        json!({"action":"loopbackHttp","id":"org.example.argocd","allowLoopbackHttp":true}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("does not request network.http"),
        "{refused}"
    );
    // `@srelens/core` sends the camelCase name; the Rust spelling is refused, so a
    // wrapper that drifted to it would fail here instead of saving nothing.
    let snake = configure(
        json!({"action":"loopbackHttp","id":"org.example.argocd","allow_loopback_http":false}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(snake.contains("allow_loopback_http"), "{snake}");
    // Turning it off is always allowed, and is the stored default.
    configure(json!({"action":"loopbackHttp","id":"org.example.argocd","allowLoopbackHttp":false}))
        .await
        .unwrap();
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("allowLoopbackHttp"));

    let grants = json!(["network.http", SECRET_STORE_PERMISSION]);
    let first = configure(
        json!({"action":"install","manifest":metrics_manifest().to_string(),"grants":grants}),
    )
    .await
    .unwrap();
    configure(json!({"action":"loopbackHttp","id":APP,"allowLoopbackHttp":true}))
        .await
        .unwrap();
    let revision = first["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|app| app["manifest"]["id"] == APP)
        .unwrap()["revision"]
        .clone();
    let mut next = metrics_manifest();
    next["version"] = json!("0.2.0");
    let updated = configure(json!({"action":"install","manifest":next.to_string(),"grants":grants,"reviewedRevision":revision}))
        .await
        .unwrap();
    let app = updated["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|app| app["manifest"]["id"] == APP)
        .unwrap()
        .clone();
    assert_eq!(app["manifest"]["version"], "0.2.0");
    assert_eq!(app["allowLoopbackHttp"], true);
}

#[test]
fn a_binding_is_checked_against_the_rest_of_its_manifest() {
    let core = fake_core();
    let check = |value: &Value, grants: &[&str]| {
        let manifest = Manifest::parse(&value.to_string()).unwrap();
        let grants: Vec<String> = grants.iter().map(|g| (*g).to_owned()).collect();
        super::super::validate_app(&manifest, &grants, core.clone())
            .err()
            .map(|errors| {
                errors
                    .0
                    .into_iter()
                    .map(|error| (error.path, error.message))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let grants = ["network.http", SECRET_STORE_PERMISSION];
    assert_eq!(check(&metrics_manifest(), &grants), []);
    // Every permission is granted, or the app is refused.
    let found = check(&metrics_manifest(), &[SECRET_STORE_PERMISSION]);
    assert!(
        found
            .iter()
            .any(|(_, why)| why == "network.http was not granted"),
        "{found:?}"
    );

    let mut inputs = metrics_manifest();
    inputs["capabilities"][0]["inputs"] = json!(["context"]);
    let found = check(&inputs, &grants);
    assert!(
        found
            .iter()
            .any(|(path, _)| path == "capabilities[0].inputs"),
        "{found:?}"
    );

    let mut undeclared = metrics_manifest();
    undeclared["capabilities"][0]["arguments"]["secretHeaders"]["Authorization"]["secret"] =
        json!("prometheusUrl");
    let found = check(&undeclared, &grants);
    assert!(
        found.iter().any(|(path, _)| path == "capabilities[0].arguments.secretHeaders.Authorization.secret"),
        "{found:?}"
    );

    let mut literal = metrics_manifest();
    literal["capabilities"][0]["arguments"]["url"] = json!("https://evil.example/");
    let found = check(&literal, &grants);
    assert!(
        found
            .iter()
            .any(|(path, _)| path == "capabilities[0].arguments.url"),
        "{found:?}"
    );
    literal["permissions"][0]["hosts"] = json!(["${settings.prometheusUrl}", "evil.example"]);
    assert_eq!(check(&literal, &grants), []);

    // A credential written into the public manifest is refused at install.
    let mut credential = metrics_manifest();
    credential["capabilities"][0]["arguments"]["headers"] = json!({"Authorization":"Bearer abc"});
    let found = check(&credential, &grants);
    assert!(
        found
            .iter()
            .any(|(path, why)| path == "capabilities[0].arguments" && why.contains("credential")),
        "{found:?}"
    );
}

#[test]
fn the_access_review_shows_each_host_and_what_a_request_sends() {
    let manifest = |hosts: Value| {
        let mut value = metrics_manifest();
        value["permissions"][0]["hosts"] = hosts;
        Manifest::parse(&value.to_string()).unwrap()
    };
    let grants: Vec<String> = vec!["network.http".into(), SECRET_STORE_PERMISSION.into()];
    let old = manifest(json!(["${settings.prometheusUrl}"]));
    let wider = manifest(json!(["${settings.prometheusUrl}", "*.grafana.net"]));
    let diff = super::super::permission_diff(Some((&old, &grants, 1)), &wider, &grants);
    // Adding a host is that host, and nothing else changed.
    assert_eq!(
        diff.added,
        ["Reach *.grafana.net with network.http"],
        "{diff:?}"
    );
    assert!(diff.removed.is_empty(), "{diff:?}");
    // A host read from a setting is shown with the setting's declaration.
    let setting = diff
        .unchanged
        .iter()
        .find(|item| item.starts_with("Reach ${settings.prometheusUrl}"))
        .unwrap();
    assert!(setting.contains("\"type\":\"url\""), "{setting}");
    let narrower = super::super::permission_diff(Some((&wider, &grants, 2)), &old, &grants);
    assert_eq!(narrower.removed, ["Reach *.grafana.net with network.http"]);
    // Listing the same hosts in another order is no change.
    let reordered = manifest(json!(["*.grafana.net", "${settings.prometheusUrl}"]));
    let diff = super::super::permission_diff(Some((&wider, &grants, 2)), &reordered, &grants);
    assert!(diff.added.is_empty() && diff.removed.is_empty(), "{diff:?}");
    // What the request sends is its own item, secret headers by reference only.
    assert!(diff
        .unchanged
        .iter()
        .any(|item| item.starts_with("Read network.http with")
            && item.contains("\"secret\":\"token\"")));
}
