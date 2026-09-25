//! `network.http` (#568): the one way an app reaches a system outside the
//! cluster — Prometheus, GitHub, Grafana — and only the hosts it was granted.
//!
//! **Broker-only.** The capability is registered in the broker's registry,
//! beside `crd::check_capability`, and never in the one the catalog and MCP
//! are built from: called directly it would be a fetch of any URL. Its
//! declaration here is what a binding is checked against — the arguments it
//! takes, the URL a `url` setting may fill, the argument a secret may go into,
//! and [`check_arguments`] — and [`read`] is the only path that sends one.
//!
//! [`read`] builds the request itself rather than handing JSON to a handler,
//! so a secret header travels as a [`srelens_plugin_host::SecretValue`] and
//! then a sensitive header value, never through a `serde_json::Value` that a
//! log line or an error could print.
//!
//! What a request is held to, on every call:
//!
//! - the URL and every redirect: HTTPS, or plain HTTP to this computer when a
//!   person has allowed that for the app; no credentials or fragment; a host
//!   and port on the allowlist resolved from the manifest and the app's saved
//!   settings ([`srelens_plugin_host::Manifest::network_allowlist`]);
//! - a request carrying a secret follows no redirect to another origin;
//! - GET only, fixed in the manifest: no caller input (#569 adds templates);
//! - [`Limits`]: connect and total timeouts and a response size limit.
use super::http_policy::{self, BodyError, UrlRules};
use super::Installed;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::Url;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use srelens_capability::settings::SettingType;
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_plugin_host::{
    Binding, HostRule, Manifest, PluginHost, SecretStore, ValidationCode as Code, ValidationErrors,
    NETWORK_HTTP,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// The argument a secret goes into, by reference (#543).
pub(super) const SECRET_HEADERS: &str = "secretHeaders";
/// What a `url` setting stands for while a binding is checked without values.
const STAND_IN: &str = "https://settings.invalid/";
/// The longest request URL, path and query included.
const MAX_URL: usize = 8 * 1024;
/// The longest `path`.
const MAX_PATH: usize = 2048;
/// Most query parameters.
const MAX_QUERY: usize = 32;
/// Most headers, literal and secret together.
const MAX_HEADERS: usize = 32;
/// The longest literal header value.
const MAX_HEADER_VALUE: usize = 4096;
/// The longest text put before a secret, such as `Bearer `.
const MAX_PREFIX: usize = 32;
/// The largest response body.
pub(super) const MAX_RESPONSE: usize = 4 * 1024 * 1024;

/// Never set by an app: they describe the connection, not the request.
const CONNECTION_HEADERS: &[&str] = &[
    "connection",
    "content-length",
    "expect",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];
/// Credentials: never literal in a public manifest, only by reference in
/// `secretHeaders`.
const CREDENTIAL_HEADERS: &[&str] = &["authorization", "cookie"];

/// A `network.http` binding's arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HttpIn {
    /// The URL to GET: `https://…`, or `http://` to this computer when a person
    /// allows it for the app. May be `${settings.<id>}` for a `url` setting.
    url: String,
    /// Appended to the URL's path: starts with `/`, no `.` or `..` segments.
    #[serde(default)]
    path: Option<String>,
    /// Query parameters added to the URL's own.
    #[serde(default)]
    query: BTreeMap<String, String>,
    /// Literal headers. Credentials go in `secretHeaders`.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Headers whose value is one of the app's `secret-reference` settings, put
    /// in by the host as the request is sent. The app never sees the value.
    #[serde(default, rename = "secretHeaders")]
    secret_headers: BTreeMap<String, SecretHeader>,
}

/// One header filled from a secret: `prefix` then the secret, e.g. `Bearer …`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SecretHeader {
    /// The id of a `secret-reference` setting the app declares.
    secret: String,
    /// Literal text before the secret, such as `Bearer `.
    #[serde(default)]
    prefix: String,
}

/// What a request answers: its status, its content type and its body, parsed
/// when the server says it is JSON and as text otherwise.
#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct HttpOut {
    status: u16,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    body: Value,
}

/// The declaration a `network.http` binding is checked against. Its handler
/// refuses: [`read`] sends the request, with the app's allowlist and secrets.
pub(crate) fn capability() -> Capability {
    Capability::typed::<HttpIn, HttpOut, _, _>(
        NETWORK_HTTP,
        "GET a URL on one of an app's granted hosts; sent only by the extension broker",
        Annotations::READ_ONLY,
        |_| async {
            Err(CapabilityError::Handler(
                "network.http is sent only by the extension broker, for an installed app".into(),
            ))
        },
    )
    .checking_bound_arguments(check_arguments)
    .with_settable("url", &[SettingType::Url], json!(STAND_IN))
    .with_secret_slot(SECRET_HEADERS)
}

/// The rules a binding's arguments meet beyond the schema: a URL this host
/// could fetch, a plain path, and headers an app may set. Run at install (with
/// the stand-in for a setting), on save and on every request (with its value).
/// No reason repeats a value.
fn check_arguments(arguments: &Map<String, Value>) -> Result<(), String> {
    let input: HttpIn = serde_json::from_value(Value::Object(arguments.clone()))
        .map_err(|e| format!("network.http arguments: {e}"))?;
    let url = request_url(&input)?;
    // Whether this app may use plain HTTP to this computer is a person's
    // switch, checked on each request; a binding may only aim at loopback.
    http_policy::check_url(
        &url,
        UrlRules {
            ports: true,
            loopback_http: true,
        },
    )?;
    header_problems(&input)
}

/// The URL a binding GETs: `url`, then `path` appended to its path, then
/// `query` added to its query.
fn request_url(input: &HttpIn) -> Result<Url, String> {
    let mut url = Url::parse(&input.url).map_err(|_| "`url` is not a URL")?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err("`url` is an https URL".into());
    }
    if let Some(path) = &input.path {
        let plain = path.starts_with('/')
            && path.len() <= MAX_PATH
            && !path
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '\\'))
            && path.split('/').all(|segment| {
                !matches!(
                    segment.to_ascii_lowercase().as_str(),
                    "." | ".." | "%2e" | "%2e%2e" | ".%2e" | "%2e."
                )
            });
        if !plain {
            return Err(format!(
                "`path` starts with / and has no . or .. segment, query, fragment, backslash or space, in at most {MAX_PATH} characters"
            ));
        }
        let base = url.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{base}{path}"));
    }
    if input.query.len() > MAX_QUERY {
        return Err(format!("Declare at most {MAX_QUERY} query parameters"));
    }
    if input.query.iter().any(|(key, value)| {
        key.is_empty() || key.chars().chain(value.chars()).any(char::is_control)
    }) {
        return Err("A query parameter has a name, and no control characters".into());
    }
    if !input.query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in &input.query {
            pairs.append_pair(key, value);
        }
    }
    if url.as_str().len() > MAX_URL {
        return Err(format!("The request URL is longer than {MAX_URL} bytes"));
    }
    Ok(url)
}

/// Every header a binding sets is a real header name, one an app may set, with
/// a value a header can carry, and listed once.
fn header_problems(input: &HttpIn) -> Result<(), String> {
    if input.headers.len() + input.secret_headers.len() > MAX_HEADERS {
        return Err(format!("Declare at most {MAX_HEADERS} headers"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let names = input
        .headers
        .keys()
        .map(|name| (name, false))
        .chain(input.secret_headers.keys().map(|name| (name, true)));
    for (name, secret) in names {
        let header = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("\"{name}\" is not a header name"))?;
        if CONNECTION_HEADERS.contains(&header.as_str()) {
            return Err(format!("{header} is set by the host, not by an app"));
        }
        if !secret && CREDENTIAL_HEADERS.contains(&header.as_str()) {
            return Err(format!(
                "{header} carries a credential: send it from a secret-reference setting in secretHeaders"
            ));
        }
        if !seen.insert(header.as_str().to_owned()) {
            return Err(format!("{header} is set more than once"));
        }
    }
    for (name, value) in &input.headers {
        if value.len() > MAX_HEADER_VALUE || HeaderValue::from_str(value).is_err() {
            return Err(format!(
                "The {name} header's value is at most {MAX_HEADER_VALUE} characters of visible text"
            ));
        }
    }
    for (name, header) in &input.secret_headers {
        let id = &header.secret;
        if id.is_empty()
            || id.len() > 64
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(format!(
                "The {name} header's secret names a setting: 1–64 letters, digits and -"
            ));
        }
        if header.prefix.len() > MAX_PREFIX
            || !header.prefix.bytes().all(|b| (b' '..=b'~').contains(&b))
        {
            return Err(format!(
                "The {name} header's prefix is at most {MAX_PREFIX} characters of printable ASCII"
            ));
        }
    }
    Ok(())
}

/// The rules a `network.http` binding meets that need the rest of the
/// manifest: every secret header names a declared secret, and a literal URL is
/// on the app's own hosts. The rest are [`check_arguments`], run by the broker.
pub(super) fn binding_problems(
    manifest: &Manifest,
    index: usize,
    binding: &Binding,
    problems: &mut ValidationErrors,
) {
    let at = format!("capabilities[{index}]");
    if !binding.inputs.is_empty() {
        problems.push(
            Code::InvalidBinding,
            format!("{at}.inputs"),
            "A network.http request is fixed in the manifest, so it takes no inputs",
        );
    }
    if let Some(Value::Object(headers)) = binding.arguments.get(SECRET_HEADERS) {
        for (name, header) in headers {
            let Some(id) = header.get("secret").and_then(Value::as_str) else {
                continue; // The capability's own check says what is wrong with it.
            };
            let declared = manifest
                .setting(id)
                .is_some_and(|setting| setting.setting_type == SettingType::SecretReference);
            if !declared {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.arguments.{SECRET_HEADERS}.{name}.secret"),
                    format!("No secret-reference setting \"{id}\" is declared"),
                );
            }
        }
    }
    let literal = binding
        .arguments
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !srelens_capability::settings::mentions_setting(url))
        .and_then(|url| Url::parse(url).ok());
    if let Some(url) = literal {
        let allowlist = manifest.network_allowlist(&Map::new());
        if !allowlist.iter().any(|rule| rule.allows(&url)) {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments.url"),
                "The URL's host and port are not among this app's network.http hosts",
            );
        }
    }
}

/// How long a request may take and how much may come back.
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub connect: Duration,
    pub total: Duration,
    pub body: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connect: http_policy::CONNECT_TIMEOUT,
            total: http_policy::TIMEOUT,
            body: MAX_RESPONSE,
        }
    }
}

/// Where one app's requests may go.
pub(super) struct Policy {
    pub allowlist: Vec<HostRule>,
    /// A person's switch for the app: plain HTTP to this computer.
    pub loopback_http: bool,
    pub limits: Limits,
}

const NOT_ALLOWED: &str = "The URL's host and port are not among this app's network.http hosts";
const SECRET_REDIRECT: &str =
    "A request carrying a secret header does not follow a redirect to another origin";

impl Policy {
    /// Why `url` may not be fetched for this app. Never names the URL.
    pub fn check(&self, url: &Url) -> Result<(), &'static str> {
        http_policy::check_url(
            url,
            UrlRules {
                ports: true,
                loopback_http: self.loopback_http,
            },
        )?;
        if !self.allowlist.iter().any(|rule| rule.allows(url)) {
            return Err(NOT_ALLOWED);
        }
        Ok(())
    }
}

/// `extensions.read` of a `network.http` binding. The caller has already made
/// every check an app request makes (`resolver_app`'s): the app is enabled, at
/// the reviewed revision, on a cluster it is enabled for, with its grants.
pub(super) async fn read(
    core: &Registry,
    secrets: &dyn SecretStore,
    plugin: &Installed,
    name: &str,
) -> Result<Value, CapabilityError> {
    read_with(core, secrets, plugin, name, Limits::default()).await
}

async fn read_with(
    core: &Registry,
    secrets: &dyn SecretStore,
    plugin: &Installed,
    name: &str,
    limits: Limits,
) -> Result<Value, CapabilityError> {
    let failed = |why: String| CapabilityError::Handler(why);
    let manifest = &plugin.manifest;
    let binding = manifest
        .capabilities
        .iter()
        .find(|binding| binding.name == name && binding.target == NETWORK_HTTP)
        .ok_or_else(|| failed(format!("\"{name}\" is not a network.http binding")))?;
    let target = core
        .get(NETWORK_HTTP)
        .ok_or_else(|| failed("This host does not provide network.http".into()))?;
    // The one path a setting takes into a request, as for every other binding.
    let arguments = PluginHost::interpolate(target, manifest, binding, Some(&plugin.settings))
        .map_err(|problems| {
            failed(
                problems
                    .into_iter()
                    .map(|(key, why)| format!("{key}: {why}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
    check_arguments(&arguments).map_err(failed)?;
    let input: HttpIn = serde_json::from_value(Value::Object(arguments))
        .map_err(|e| failed(format!("network.http arguments: {e}")))?;
    let url = request_url(&input).map_err(failed)?;
    let policy = Policy {
        allowlist: manifest.network_allowlist(&plugin.settings),
        loopback_http: plugin.allow_loopback_http,
        limits,
    };
    // Refused before any secret is read: a request that cannot go out has no
    // reason to unseal the store.
    policy.check(&url).map_err(|why| failed(why.into()))?;
    let mut headers = HeaderMap::new();
    for (key, value) in &input.headers {
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|e| failed(e.to_string()))?;
        let value = HeaderValue::from_str(value).map_err(|e| failed(e.to_string()))?;
        headers.insert(name, value);
    }
    for (key, reference) in &input.secret_headers {
        let secret = PluginHost::inject_secret(
            target,
            SECRET_HEADERS,
            manifest,
            &plugin.grants,
            &plugin.settings,
            &reference.secret,
            secrets,
        )
        .map_err(failed)?;
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|e| failed(e.to_string()))?;
        let mut value = HeaderValue::from_str(&format!("{}{}", reference.prefix, secret.expose()))
            .map_err(|_| {
                failed(format!(
                    "The secret for the {name} header holds a character a header cannot carry"
                ))
            })?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    let carries_secret = !input.secret_headers.is_empty();
    let answer = send(url, headers, carries_secret, policy)
        .await
        .map_err(failed)?;
    serde_json::to_value(answer).map_err(|e| failed(e.to_string()))
}

/// GETs `url` under `policy`, and reads what comes back. Every reason is the
/// host's own words, with the URL and its host scrubbed out of whatever the
/// connection reported.
pub(super) async fn send(
    url: Url,
    headers: HeaderMap,
    carries_secret: bool,
    policy: Policy,
) -> Result<HttpOut, String> {
    policy.check(&url)?;
    http_policy::install_crypto_provider();
    let limits = policy.limits;
    let origin = url.origin();
    let policy = Arc::new(policy);
    let hop = policy.clone();
    let mut builder = reqwest::Client::builder()
        .connect_timeout(limits.connect)
        .timeout(limits.total)
        .user_agent("srelens-app-http")
        .redirect(http_policy::redirects(move |next| {
            hop.check(next)?;
            if carries_secret && next.origin() != origin {
                return Err(SECRET_REDIRECT);
            }
            Ok(())
        }));
    // A proxy is for leaving this computer; loopback never goes through one.
    if http_policy::is_loopback(&url) {
        builder = builder.no_proxy();
    }
    let client = builder.build().map_err(|e| describe(e, &url))?;
    let reported = url.clone();
    let exchange = async move {
        let mut response = client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| describe(e, &reported))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("The server answered HTTP {status}"));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let raw = http_policy::read_limited(&mut response, limits.body)
            .await
            .map_err(|error| match error {
                BodyError::TooLarge(why) => why,
                BodyError::Read(error) => describe(error, &reported),
            })?;
        Ok(HttpOut {
            status: status.as_u16(),
            body: decode(content_type.as_deref(), &raw)?,
            content_type,
        })
    };
    tokio::time::timeout(limits.total, exchange)
        .await
        .map_err(|_| timed_out(limits.total))?
}

fn timed_out(total: Duration) -> String {
    format!("The request took longer than {} s", total.as_secs_f32())
}

/// A body as JSON when the server says it is, else as text.
fn decode(content_type: Option<&str>, raw: &[u8]) -> Result<Value, String> {
    if raw.is_empty() {
        return Ok(Value::Null);
    }
    let json = content_type
        .and_then(|value| value.split(';').next())
        .map(|essence| essence.trim().to_ascii_lowercase())
        .is_some_and(|essence| essence == "application/json" || essence.ends_with("+json"));
    if json {
        return serde_json::from_slice(raw)
            .map_err(|_| "The server said it sent JSON, and what it sent is not JSON".into());
    }
    String::from_utf8(raw.to_vec())
        .map(Value::String)
        .map_err(|_| "The response is neither JSON nor UTF-8 text".into())
}

/// What went wrong with a request, in the host's words, with the URL and its
/// host scrubbed out of what the connection reported: a URL may be a setting's
/// value, and this text reaches errors a log or an MCP client may keep.
fn describe(error: reqwest::Error, url: &Url) -> String {
    if let Some(why) = http_policy::redirect_refusal(&error) {
        return why.into();
    }
    if error.is_timeout() {
        return "The request timed out".into();
    }
    let lead = if error.is_connect() {
        "Could not connect to the server"
    } else {
        "The request failed"
    };
    let mut causes = Vec::new();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let mut text = if causes.is_empty() {
        lead.to_owned()
    } else {
        format!("{lead}: {}", causes.join(": "))
    };
    for spelling in [Some(url.as_str()), url.host_str()].into_iter().flatten() {
        if !spelling.is_empty() {
            text = text.replace(spelling, "the server");
        }
    }
    text
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
