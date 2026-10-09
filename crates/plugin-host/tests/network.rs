//! Brokered HTTP (#568): the scoped `network.http` permission a manifest
//! declares, the hosts it may name, and how the host resolves them into the
//! allowlist every request is held to.
use serde_json::{json, Map, Value};
use srelens_plugin_host::{
    HostRule, Manifest, Permission, ValidationCode, ValidationError, NETWORK_HTTP,
};

/// A metrics app: one Prometheus query, its URL a setting, reaching that URL's
/// host and one fixed API host.
fn manifest() -> Value {
    json!({
        "id":"org.example.metrics", "name":"Metrics", "version":"0.1.0", "srelensApiVersion":"^0.4",
        "kind":"declarative",
        "permissions":[{"capability":"network.http","hosts":["${settings.prometheusUrl}","api.github.com"]}],
        "settings":[{"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true}],
        "capabilities":[{"name":"up","title":"Targets up","target":"network.http",
            "arguments":{"url":"${settings.prometheusUrl}","path":"/api/v1/query","query":{"query":"up"}},
            "inputs":[]}],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[]}
    })
}

fn parse(value: &Value) -> Manifest {
    Manifest::parse(&value.to_string()).expect("valid manifest")
}

fn errors(value: &Value) -> Vec<ValidationError> {
    Manifest::parse(&value.to_string()).unwrap_err().0
}

fn code(code: ValidationCode) -> String {
    serde_json::to_value(code)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn problems(errors: &[ValidationError]) -> Vec<(String, String)> {
    let mut problems: Vec<_> = errors
        .iter()
        .map(|error| (code(error.code), error.path.clone()))
        .collect();
    problems.sort();
    problems
}

fn with_hosts(hosts: Value) -> Value {
    let mut value = manifest();
    value["permissions"][0]["hosts"] = hosts;
    value
}

#[test]
fn a_scoped_permission_parses_and_is_stored_as_written() {
    let parsed = parse(&manifest());
    assert_eq!(parsed.permissions.len(), 1);
    assert_eq!(parsed.permissions[0].capability(), NETWORK_HTTP);
    assert_eq!(
        parsed.permissions[0].hosts(),
        ["${settings.prometheusUrl}", "api.github.com"]
    );
    // The grant is the capability's id; the scope is the manifest's, shown for review.
    assert_eq!(parsed.permission_names(), [NETWORK_HTTP]);
    // A signed manifest is compared by its stored form, so both spellings round-trip.
    let stored = serde_json::to_value(&parsed).unwrap();
    assert_eq!(stored["permissions"], manifest()["permissions"]);
}

#[test]
fn a_plain_permission_still_reads_and_writes_as_a_string() {
    let permission: Permission = serde_json::from_value(json!("k8s.listCustomResource")).unwrap();
    assert_eq!(permission, Permission::from("k8s.listCustomResource"));
    assert!(permission == *"k8s.listCustomResource");
    assert!(permission.hosts().is_empty());
    assert_eq!(
        serde_json::to_value(&permission).unwrap(),
        json!("k8s.listCustomResource")
    );
}

#[test]
fn network_http_requires_api_0_4() {
    // Without settings, which API 0.4 also added (#709), so the scoped permission is
    // the one 0.4 field here.
    let mut value = manifest();
    value.as_object_mut().unwrap().remove("settings");
    value["permissions"][0]["hosts"] = json!(["api.github.com"]);
    value["capabilities"][0]["arguments"]["url"] = json!("https://api.github.com");
    parse(&value);
    value["srelensApiVersion"] = json!("^0.3");
    let found = errors(&value);
    assert_eq!(
        problems(&found),
        [(
            code(ValidationCode::ApiIncompatible),
            "srelensApiVersion".into()
        )]
    );
    assert!(
        found[0]
            .message
            .contains("`permissions[].hosts` requires API 0.4.0"),
        "{}",
        found[0].message
    );
    // A range that also admits 0.3 claims 0.3 hosts, so it is refused too.
    value["srelensApiVersion"] = json!(">=0.3, <0.5");
    assert_eq!(
        problems(&errors(&value)),
        [(
            code(ValidationCode::ApiIncompatible),
            "srelensApiVersion".into()
        )]
    );
}

#[test]
fn network_http_is_always_scoped_and_nothing_else_is() {
    let mut bare = manifest();
    bare["permissions"] = json!(["network.http"]);
    assert_eq!(
        problems(&errors(&bare)),
        [(code(ValidationCode::InvalidValue), "permissions[0]".into())]
    );

    let mut scoped = manifest();
    scoped["permissions"] = json!([
        {"capability":"network.http","hosts":["api.github.com"]},
        {"capability":"k8s.listEvents","hosts":["api.github.com"]}
    ]);
    let found = problems(&errors(&scoped));
    assert!(
        found.contains(&(
            code(ValidationCode::InvalidField),
            "permissions[1].hosts".into()
        )),
        "{found:?}"
    );
}

#[test]
fn a_scoped_permission_is_strict_about_its_fields() {
    let mut value = manifest();
    value["permissions"][0]["methods"] = json!(["POST"]);
    let found = errors(&value);
    assert_eq!(
        problems(&found),
        [(
            code(ValidationCode::UnknownField),
            "permissions[0].methods".into()
        )],
        "{found:?}"
    );
    let mut value = manifest();
    value["permissions"] = json!([42]);
    assert_eq!(
        problems(&errors(&value)),
        [(code(ValidationCode::InvalidField), "permissions[0]".into())]
    );
}

#[test]
fn hosts_are_names_ports_subdomain_wildcards_ip_literals_or_url_settings() {
    for host in [
        "api.github.com",
        "api.github.com:8443",
        "*.grafana.net",
        "*.eu.grafana.net:443",
        "localhost",
        "localhost:9090",
        "127.0.0.1",
        "10.0.0.5:9090",
        "[::1]",
        "[::1]:9090",
        "prometheus",
        "xn--bcher-kva.example",
    ] {
        let value = with_hosts(json!(["${settings.prometheusUrl}", host]));
        assert!(
            Manifest::parse(&value.to_string()).is_ok(),
            "{host}: {:?}",
            Manifest::parse(&value.to_string()).err()
        );
    }
    for host in [
        "",
        "*",
        "*.com",
        "*.com:443",
        "api.*.com",
        "foo*.bar.com",
        "*.*.example.com",
        "**.example.com",
        "https://api.github.com",
        "api.github.com/v3",
        "api.github.com?x=1",
        "user@api.github.com",
        "API.GitHub.com",
        "api.github.com.",
        "api..github.com",
        "-api.github.com",
        "api-.github.com",
        "api_github.com",
        "api.github.com:",
        "api.github.com:0",
        "api.github.com:65536",
        "api.github.com:x",
        "::1",
        "[::1",
        "[localhost]",
        "*.127.0.0.1",
        "bücher.example",
        " api.github.com",
    ] {
        let value = with_hosts(json!(["${settings.prometheusUrl}", host]));
        let found = problems(&errors(&value));
        assert_eq!(
            found,
            [(
                code(ValidationCode::InvalidValue),
                "permissions[0].hosts[1]".into()
            )],
            "{host:?}"
        );
    }
}

#[test]
fn hosts_are_counted_and_listed_once() {
    let found = problems(&errors(&with_hosts(json!([]))));
    assert!(found.contains(&(
        code(ValidationCode::InvalidValue),
        "permissions[0].hosts".into()
    )));
    let many: Vec<String> = (0..17).map(|i| format!("h{i}.example.com")).collect();
    let mut value = with_hosts(json!(many));
    value["permissions"][0]["hosts"][0] = json!("${settings.prometheusUrl}");
    let found = problems(&errors(&value));
    assert!(
        found.contains(&(
            code(ValidationCode::InvalidValue),
            "permissions[0].hosts".into()
        )),
        "{found:?}"
    );
    let found = problems(&errors(&with_hosts(json!([
        "${settings.prometheusUrl}",
        "api.github.com",
        "api.github.com"
    ]))));
    assert_eq!(
        found,
        [(
            code(ValidationCode::DuplicateIdentifier),
            "permissions[0].hosts[2]".into()
        )]
    );
}

#[test]
fn a_host_setting_is_a_whole_reference_to_a_declared_url_setting() {
    let mut value = manifest();
    value["settings"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"note","type":"string","title":"Note"}));
    for (host, why) in [
        ("${settings.note}", "a string setting"),
        ("${settings.missing}", "an undeclared setting"),
        ("https://${settings.prometheusUrl}", "part of a value"),
        ("${settings.prometheusUrl}:9090", "part of a value"),
        ("${ settings.prometheusUrl}", "a near miss"),
    ] {
        let mut value = value.clone();
        value["permissions"][0]["hosts"][1] = json!(host);
        let found = problems(&errors(&value));
        assert_eq!(
            found,
            [(
                code(ValidationCode::InvalidValue),
                "permissions[0].hosts[1]".into()
            )],
            "{why}: {host}"
        );
    }
    // An optional URL setting is fine: while it is unset it reaches nothing.
    let mut optional = manifest();
    optional["settings"][0]["required"] = json!(false);
    optional["capabilities"][0]["arguments"]["url"] = json!("https://api.github.com");
    parse(&optional);
}

#[test]
fn a_setting_is_mentioned_nowhere_else_in_a_permission() {
    let mut value = manifest();
    value["permissions"][0]["capability"] = json!("${settings.prometheusUrl}");
    let found = problems(&errors(&value));
    assert!(
        found.contains(&(
            code(ValidationCode::InvalidBinding),
            "permissions[0].capability".into()
        )),
        "{found:?}"
    );
}

fn url(raw: &str) -> url::Url {
    url::Url::parse(raw).unwrap()
}

#[test]
fn a_rule_matches_its_host_and_port_and_nothing_that_merely_resembles_it() {
    let rule = |entry: &str| HostRule::parse(entry).unwrap();
    let github = rule("api.github.com");
    assert!(github.allows(&url("https://api.github.com/repos")));
    // The URL parser lowercases a host, so case cannot hide one.
    assert!(github.allows(&url("https://API.GitHub.com/repos")));
    // Without a port, a rule is the scheme's default port.
    assert!(github.allows(&url("https://api.github.com:443/")));
    for other in [
        "https://api.github.com:8443/",
        "https://github.com/",
        "https://api.github.com.evil.example/",
        "https://evilapi.github.com/",
        "https://api.github.co/",
        // A trailing dot is another spelling of the name; it is not guessed at.
        "https://api.github.com./",
    ] {
        assert!(!github.allows(&url(other)), "{other}");
    }

    let ported = rule("prometheus.internal:9090");
    assert!(ported.allows(&url("https://prometheus.internal:9090/api")));
    assert!(!ported.allows(&url("https://prometheus.internal/api")));

    let wildcard = rule("*.grafana.net");
    assert!(wildcard.allows(&url("https://myorg.grafana.net/api")));
    for other in [
        // A wildcard is one label, as a certificate's is.
        "https://a.b.grafana.net/",
        "https://grafana.net/",
        "https://myorg.grafana.net.evil.example/",
        "https://myorggrafana.net/",
        "https://myorg.grafana.net:8443/",
    ] {
        assert!(!wildcard.allows(&url(other)), "{other}");
    }

    let loopback = rule("[::1]:9090");
    assert!(loopback.allows(&url("http://[::1]:9090/")));
    assert!(!loopback.allows(&url("http://127.0.0.1:9090/")));
    assert!(rule("127.0.0.1:9090").allows(&url("http://127.0.0.1:9090/metrics")));
    assert!(!rule("127.0.0.1:9090").allows(&url("http://localhost:9090/")));
}

#[test]
fn the_allowlist_is_the_manifests_hosts_with_each_url_setting_as_saved() {
    let mut value = manifest();
    value["permissions"][0]["hosts"] = json!([
        "${settings.prometheusUrl}",
        "${settings.lokiUrl}",
        "${settings.tempoUrl}",
        "api.github.com"
    ]);
    value["settings"] = json!([
        {"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true},
        {"id":"lokiUrl","type":"url","title":"Loki URL","default":"https://loki.example.com:3100"},
        {"id":"tempoUrl","type":"url","title":"Tempo URL"}
    ]);
    let parsed = parse(&value);
    let saved: Map<String, Value> =
        serde_json::from_value(json!({"prometheusUrl":"http://localhost:9090/prometheus"}))
            .unwrap();
    let allowlist = parsed.network_allowlist(&saved);
    for allowed in [
        "http://localhost:9090/api/v1/query",
        "https://loki.example.com:3100/loki/api/v1/query_range",
        "https://api.github.com/repos",
    ] {
        assert!(
            allowlist.iter().any(|rule| rule.allows(&url(allowed))),
            "{allowed}"
        );
    }
    for refused in [
        // A setting's host is allowed at the port it names, not at every port.
        "http://localhost:9091/",
        "https://loki.example.com/",
        "https://tempo.example.com/",
    ] {
        assert!(
            !allowlist.iter().any(|rule| rule.allows(&url(refused))),
            "{refused}"
        );
    }
    // An unset setting without a default adds nothing; a value the setting's
    // own rules refuse (a hand-edited inventory) adds nothing either.
    let tampered: Map<String, Value> = serde_json::from_value(
        json!({"prometheusUrl":"https://user:pass@evil.example/","tempoUrl":"ftp://tempo.example.com/"}),
    )
    .unwrap();
    let allowlist = parsed.network_allowlist(&tampered);
    assert!(!allowlist
        .iter()
        .any(|rule| rule.allows(&url("https://evil.example/"))));
    assert!(!allowlist
        .iter()
        .any(|rule| rule.allows(&url("https://tempo.example.com/"))));
    // A manifest without the permission reaches nothing.
    let mut none = manifest();
    none["permissions"] = json!([]);
    none["capabilities"] = json!([]);
    let parsed: Manifest = serde_json::from_value(none).unwrap();
    assert!(parsed.network_allowlist(&Map::new()).is_empty());
}
