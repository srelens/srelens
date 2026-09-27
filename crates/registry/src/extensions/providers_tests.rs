//! Providers (#569) against a real socket: a local server on loopback stands in
//! for Prometheus, Loki and Tempo, so each query goes through `network.http`'s
//! own client, allowlist and limits, and what comes back is read as the host
//! reads it.
use super::*;
use crate::extensions::http_test_support::{server, Reply, Server, Vault};
use crate::extensions::tests::fake_core;
use serde_json::{json, Value};
use srelens_capability::Registry;
use srelens_plugin_host::Manifest;
use srelens_streams::test_util::TestSink;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const APP: &str = "org.example.observability";

/// What `@srelens/core`'s `queryExtensionProvider` sends, byte for byte:
/// `extensions.test.ts` holds the wrapper to this same file.
const WRAPPER_PAYLOAD: &str =
    include_str!("../../../../packages/core/src/lib/extension-query-provider.json");

/// Prometheus, Loki and Tempo behind one app, each URL a setting on this
/// computer's loopback.
fn observability() -> Value {
    json!({
        "id":APP, "name":"Observability", "version":"0.1.0", "srelensApiVersion":"^0.6",
        "kind":"declarative",
        "permissions":[{"capability":"network.http",
            "hosts":["${settings.prometheusUrl}","${settings.lokiUrl}","${settings.tempoUrl}"]}],
        "settings":[
            {"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true},
            {"id":"lokiUrl","type":"url","title":"Loki URL","required":true},
            {"id":"tempoUrl","type":"url","title":"Tempo URL","required":true}
        ],
        "capabilities":[
            {"name":"prom","title":"Prometheus range query","target":"network.http","inputs":[],
                "arguments":{"url":"${settings.prometheusUrl}","path":"/api/v1/query_range"}},
            {"name":"loki","title":"Loki range query","target":"network.http","inputs":[],
                "arguments":{"url":"${settings.lokiUrl}","path":"/loki/api/v1/query_range"}},
            {"name":"tempo","title":"Tempo search","target":"network.http","inputs":[],
                "arguments":{"url":"${settings.tempoUrl}","path":"/api/search"}}
        ],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[],
            "metricProviders":[{"id":"cpu","title":"CPU","capability":"prom","language":"promql",
                "forKinds":["apps/Deployment"],"unit":"cores",
                "query":"sum by (pod) (rate(container_cpu_usage_seconds_total{namespace=\"${namespace}\",pod=~\"${workload:regex}-.*\"}[${step}]))"},
                {"id":"clusterUp","title":"Targets up","capability":"prom","language":"promql",
                "forKinds":["apps/Deployment"],"unit":"number",
                "query":"up{cluster=\"${cluster}\"}"}],
            "logProviders":[{"id":"loki","title":"Loki","capability":"loki","language":"logql",
                "forKinds":["/Pod"],"query":"{namespace=\"${namespace}\", pod=\"${pod}\"}"}],
            "traceProviders":[{"id":"traces","title":"Recent traces","capability":"tempo","language":"traceql",
                "forKinds":["apps/Deployment"],
                "query":"{ resource.k8s.namespace.name = \"${namespace}\" && resource.k8s.deployment.name = \"${workload}\" }"}]
        }
    })
}

/// A broker whose kubeconfig declares `context`, with the app installed and
/// pointed at `server` for all three backends, loopback HTTP allowed.
struct Harness {
    reg: Registry,
    revision: Value,
    streams: Arc<super::super::streams::ExtensionStreams>,
    _dir: tempfile::TempDir,
}

async fn harness(server: &Server, context: &str) -> Harness {
    harness_with(server, context, observability()).await
}

async fn harness_with(server: &Server, context: &str, manifest: Value) -> Harness {
    let url = format!("http://{}", server.addr);
    harness_for(
        context,
        manifest,
        json!({"prometheusUrl":url,"lokiUrl":url,"tempoUrl":url}),
    )
    .await
}

/// `manifest`, installed as [`APP`] with `settings` saved and loopback HTTP allowed.
async fn harness_for(context: &str, manifest: Value, settings: Value) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let kubeconfig = kubeconfig(dir.path(), context);
    let path = dir.path().join("settings.extensions.json");
    let mut reg = Registry::new();
    let streams = super::super::register_with_secrets(
        &mut reg,
        path,
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![kubeconfig]),
        Arc::new(Vault::default()),
    );
    streams.set_provider_timing(ProviderTiming {
        poll: Duration::from_millis(50),
        full_page: Duration::from_millis(10),
    });
    let installed = reg
        .invoke(
            "extensions.configure",
            json!({"action":"install","manifest":manifest.to_string(),"grants":["network.http"]}),
        )
        .await
        .unwrap();
    let revision = installed["plugins"][0]["revision"].clone();
    reg.invoke(
        "extensions.configure",
        json!({"action":"settings","id":APP,"settings":settings}),
    )
    .await
    .unwrap();
    reg.invoke(
        "extensions.configure",
        json!({"action":"loopbackHttp","id":APP,"allowLoopbackHttp":true}),
    )
    .await
    .unwrap();
    Harness {
        reg,
        revision,
        streams,
        _dir: dir,
    }
}

/// A kubeconfig with one context named `context`, written as JSON so any name
/// survives the YAML reader.
fn kubeconfig(dir: &Path, context: &str) -> PathBuf {
    let path = dir.join("config");
    let config = json!({
        "apiVersion":"v1","kind":"Config",
        "clusters":[{"name":"c","cluster":{"server":"https://127.0.0.1:1"}}],
        "users":[{"name":"u","user":{}}],
        "contexts":[{"name":context,"context":{"cluster":"c","user":"u"}}]
    });
    std::fs::write(&path, config.to_string()).unwrap();
    path
}

impl Harness {
    async fn query(&self, input: Value) -> Result<Value, String> {
        let mut request = json!({"id":APP,"revision":self.revision,"context":"kind-dev",
            "namespace":"team","resourceKind":"apps/Deployment","name":"web","rangeSeconds":3600});
        for (key, value) in input.as_object().unwrap() {
            request[key] = value.clone();
        }
        self.reg
            .invoke("extensions.queryProvider", request)
            .await
            .map_err(|e| e.to_string())
    }
}

/// The query parameters of a request target, decoded.
fn params(target: &str) -> BTreeMap<String, String> {
    url::Url::parse(&format!("http://x{target}"))
        .unwrap()
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn number(params: &BTreeMap<String, String>, key: &str) -> i64 {
    params[key]
        .parse()
        .unwrap_or_else(|_| panic!("{key}: {}", params[key]))
}

/// A Prometheus range query's answer: each series with a sample at the first,
/// second and third step of the asked range, the second one NaN for `web-2`.
fn matrix(target: &str, series: usize) -> Reply {
    let asked = params(target);
    let (start, step) = (number(&asked, "start"), number(&asked, "step"));
    let result: Vec<Value> = (1..=series)
        .map(|n| {
            let second = if n == 2 { "NaN".to_owned() } else { format!("{n}.5") };
            json!({"metric":{"pod":format!("web-{n}")},
                "values":[[start, format!("{n}")],[start + step, second],[start + 2 * step, "0.25"]]})
        })
        .collect();
    Reply::Json(json!({"status":"success","data":{"resultType":"matrix","result":result}}))
}

#[test]
fn the_wrappers_payload_deserializes_and_snake_case_is_rejected() {
    let payload: Value = serde_json::from_str(WRAPPER_PAYLOAD).unwrap();
    let input: QueryIn = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(input.resource_kind, "apps/Deployment");
    assert_eq!(input.range_seconds, Some(3600));
    for (camel, snake) in [
        ("resourceKind", "resource_kind"),
        ("rangeSeconds", "range_seconds"),
    ] {
        let mut wrong = payload.clone();
        let value = wrong.as_object_mut().unwrap().remove(camel).unwrap();
        wrong[snake] = value;
        let refused = serde_json::from_value::<QueryIn>(wrong).unwrap_err();
        assert!(refused.to_string().contains(snake), "{refused}");
    }
}

#[test]
fn the_wrappers_log_follow_payload_deserializes_and_snake_case_is_rejected() {
    use super::super::streams::{OpenStreamIn, StreamSourceIn};
    let payload: Value = serde_json::from_str(include_str!(
        "../../../../packages/core/src/lib/extension-stream-log-provider.json"
    ))
    .unwrap();
    let input: OpenStreamIn = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(
        input.source,
        StreamSourceIn::LogProvider {
            provider: "loki".into(),
            resource_kind: "/Pod".into(),
            name: "web-1".into(),
            tail_lines: Some(200),
            since_seconds: Some(3600),
            timestamps: true,
        }
    );
    for (camel, snake) in [
        ("resourceKind", "resource_kind"),
        ("tailLines", "tail_lines"),
        ("sinceSeconds", "since_seconds"),
    ] {
        let mut wrong = payload.clone();
        let source = wrong["source"].as_object_mut().unwrap();
        let value = source.remove(camel).unwrap();
        source.insert(snake.into(), value);
        let refused = serde_json::from_value::<OpenStreamIn>(wrong).unwrap_err();
        assert!(refused.to_string().contains(snake), "{refused}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_metric_query_is_bound_sent_through_network_http_and_drawn_as_a_chart() {
    let prometheus = server(|target| matrix(target, 2)).await;
    let h = harness(&prometheus, "kind-dev").await;
    let answer = h.query(json!({"provider":"cpu"})).await.unwrap();

    let seen = prometheus.seen();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].target.starts_with("/api/v1/query_range?"),
        "{}",
        seen[0].target
    );
    let asked = params(&seen[0].target);
    assert_eq!(
        asked["query"],
        "sum by (pod) (rate(container_cpu_usage_seconds_total{namespace=\"team\",pod=~\"web-.*\"}[15s]))"
    );
    let (start, end, step) = (
        number(&asked, "start"),
        number(&asked, "end"),
        number(&asked, "step"),
    );
    assert_eq!((end - start, step), (3600, 15));
    assert_eq!(end % step, 0, "the range is aligned to its step");

    assert_eq!(answer["kind"], "metrics");
    let chart = &answer["chart"];
    assert_eq!(chart["label"], "CPU");
    assert_eq!(chart["unit"], "cores");
    assert_eq!(
        chart["range"],
        json!({"start": start * 1000, "end": end * 1000})
    );
    let times = chart["times"].as_array().unwrap();
    assert_eq!(times.len(), 241);
    assert_eq!(times[0], start * 1000);
    assert_eq!(times[1], (start + step) * 1000);
    let series = chart["series"].as_array().unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0]["name"], "pod=\"web-1\"");
    let values = series[0]["values"].as_array().unwrap();
    assert_eq!(values.len(), 241);
    assert_eq!(
        &values[..4],
        &[json!(1.0), json!(1.5), json!(0.25), Value::Null]
    );
    // A NaN sample is a gap, never a number.
    assert_eq!(series[1]["values"][1], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_series_name_shows_an_invisible_character_as_an_escape() {
    let prometheus = server(|_| {
        Reply::Json(
            json!({"status":"success","data":{"resultType":"matrix","result":[
            {"metric":{"pod":"web-1\u{202e}2"},"values":[]}]}}),
        )
    })
    .await;
    let h = harness(&prometheus, "kind-dev").await;
    let answer = h.query(json!({"provider":"cpu"})).await.unwrap();
    let name = answer["chart"]["series"][0]["name"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!name.contains('\u{202e}'), "{name:?}");
    assert!(name.starts_with("pod=\"web-1"), "{name}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cluster_name_reaches_the_query_as_one_escaped_string() {
    // A kubeconfig context name is free text, and here it tries to end the string.
    let context = "prod\"} or vector(1) #";
    let prometheus = server(|target| matrix(target, 1)).await;
    let h = harness(&prometheus, context).await;
    h.query(json!({"provider":"clusterUp","context":context}))
        .await
        .unwrap();
    let asked = params(&prometheus.seen()[0].target);
    assert_eq!(asked["query"], "up{cluster=\"prod\\\"} or vector(1) #\"}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_the_view_sends_is_held_to_what_a_kubernetes_name_can_be() {
    let prometheus = server(|target| matrix(target, 1)).await;
    let h = harness(&prometheus, "kind-dev").await;
    for (field, value) in [
        ("name", "web\"} or vector(1)"),
        ("name", "Web"),
        ("namespace", "team\")"),
    ] {
        let refused = h
            .query(json!({"provider":"cpu", field: value}))
            .await
            .unwrap_err();
        assert!(!refused.contains(value), "{refused}");
    }
    assert!(prometheus.seen().is_empty(), "nothing was sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_outside_the_allowlist_is_refused_and_never_contacted() {
    let allowed = server(|target| matrix(target, 1)).await;
    let elsewhere = server(|target| matrix(target, 1)).await;
    // The request goes to a URL from another setting, which the hosts do not list.
    let mut manifest = observability();
    manifest["settings"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"otherUrl","type":"url",
        "title":"Other URL","default":format!("http://{}", elsewhere.addr)}));
    manifest["capabilities"][0]["arguments"]["url"] = json!("${settings.otherUrl}");
    let h = harness_with(&allowed, "kind-dev", manifest).await;
    let refused = h.query(json!({"provider":"cpu"})).await.unwrap_err();
    assert!(
        refused.contains("not among this app's network.http hosts"),
        "{refused}"
    );
    assert!(!refused.contains(&elsewhere.addr.to_string()), "{refused}");
    assert!(elsewhere.seen().is_empty() && allowed.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_response_past_the_size_limit_is_refused() {
    let big = server(|_| Reply::Large {
        len: network::MAX_RESPONSE + 1,
        length: false,
    })
    .await;
    let h = harness(&big, "kind-dev").await;
    let refused = h.query(json!({"provider":"cpu"})).await.unwrap_err();
    assert!(refused.contains("4 MiB"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn more_series_than_a_chart_draws_is_refused_never_cut() {
    let prometheus = server(|target| matrix(target, MAX_SERIES + 1)).await;
    let h = harness(&prometheus, "kind-dev").await;
    let refused = h.query(json!({"provider":"cpu"})).await.unwrap_err();
    assert!(
        refused.contains("9 series") && refused.contains("at most 8"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_that_matches_nothing_is_a_chart_with_no_data_not_an_error() {
    let prometheus = server(|_| {
        Reply::Json(json!({"status":"success","data":{"resultType":"matrix","result":[]}}))
    })
    .await;
    let h = harness(&prometheus, "kind-dev").await;
    let answer = h.query(json!({"provider":"cpu"})).await.unwrap();
    let series = answer["chart"]["series"].as_array().unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["name"], "CPU");
    assert!(series[0]["values"]
        .as_array()
        .unwrap()
        .iter()
        .all(Value::is_null));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_error_answer_or_an_unreachable_server_says_what_failed() {
    let prometheus = server(|_| Reply::Status(503, "Service Unavailable")).await;
    let h = harness(&prometheus, "kind-dev").await;
    let refused = h.query(json!({"provider":"cpu"})).await.unwrap_err();
    assert!(refused.contains("HTTP 503"), "{refused}");
    let wrong = server(|_| Reply::Text("<html>login</html>")).await;
    let h = harness(&wrong, "kind-dev").await;
    let refused = h.query(json!({"provider":"cpu"})).await.unwrap_err();
    assert!(refused.contains("not a Prometheus"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_answers_only_for_the_kinds_it_is_for() {
    let prometheus = server(|target| matrix(target, 1)).await;
    let h = harness(&prometheus, "kind-dev").await;
    let refused = h
        .query(json!({"provider":"cpu","resourceKind":"/Pod"}))
        .await
        .unwrap_err();
    assert!(refused.contains("not for /Pod"), "{refused}");
    let refused = h.query(json!({"provider":"missing"})).await.unwrap_err();
    assert!(refused.contains("no provider \"missing\""), "{refused}");
    for range in [0, 60, 8 * 24 * 3600] {
        let refused = h
            .query(json!({"provider":"cpu","rangeSeconds":range}))
            .await
            .unwrap_err();
        assert!(refused.contains("range"), "{refused}");
    }
    assert!(prometheus.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trace_search_is_listed_newest_first_and_cut_at_the_limit() {
    let tempo = server(|_| {
        let traces: Vec<Value> = (0..(MAX_TRACES + 5))
            .map(|n| {
                json!({"traceID":format!("{n:032x}"),"rootServiceName":"checkout",
                    "rootTraceName":"GET /cart","startTimeUnixNano":format!("{}", 1_700_000_000_000_000_000u64 + n as u64 * 1_000_000),
                    "durationMs":n})
            })
            .collect();
        Reply::Json(json!({"traces":traces,"metrics":{"totalBlocks":1}}))
    })
    .await;
    let h = harness(&tempo, "kind-dev").await;
    let answer = h.query(json!({"provider":"traces"})).await.unwrap();
    let asked = params(&tempo.seen()[0].target);
    assert_eq!(
        asked["q"],
        "{ resource.k8s.namespace.name = \"team\" && resource.k8s.deployment.name = \"web\" }"
    );
    assert_eq!(asked["limit"], MAX_TRACES.to_string());
    assert_eq!(number(&asked, "end") - number(&asked, "start"), 3600);
    assert_eq!(answer["kind"], "traces");
    assert_eq!(answer["truncated"], true);
    let traces = answer["traces"].as_array().unwrap();
    assert_eq!(traces.len(), MAX_TRACES);
    assert_eq!(
        traces[0],
        json!({"traceId":format!("{:032x}", MAX_TRACES + 4),"rootService":"checkout",
            "rootName":"GET /cart","start":1_700_000_000_000i64 + (MAX_TRACES as i64 + 4),
            "durationMs":MAX_TRACES + 4})
    );
}

/// Loki's range query answer for `entries` of `(nanoseconds, line)` on one pod.
fn streams(entries: &[(u64, &str)]) -> Reply {
    let values: Vec<Value> = entries
        .iter()
        .map(|(ns, line)| json!([ns.to_string(), line]))
        .collect();
    Reply::Json(
        json!({"status":"success","data":{"resultType":"streams","result":[
        {"stream":{"namespace":"team","pod":"web-1","container":"app"},"values":values}]}}),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_query_answers_lines_oldest_first() {
    let loki = server(|_| {
        streams(&[
            (1_700_000_000_000_000_002, "second"),
            (1_700_000_000_000_000_001, "first"),
        ])
    })
    .await;
    let h = harness(&loki, "kind-dev").await;
    let answer = h
        .query(json!({"provider":"loki","resourceKind":"/Pod","name":"web-1"}))
        .await
        .unwrap();
    let asked = params(&loki.seen()[0].target);
    assert_eq!(asked["query"], "{namespace=\"team\", pod=\"web-1\"}");
    assert_eq!(asked["direction"], "backward");
    assert_eq!(answer["kind"], "logs");
    assert_eq!(
        answer["lines"],
        json!([
            {"time":"2023-11-14T22:13:20.000000001Z","source":"web-1/app","line":"first"},
            {"time":"2023-11-14T22:13:20.000000002Z","source":"web-1/app","line":"second"}
        ])
    );
}

fn follow(h: &Harness, channel: &str) -> Value {
    json!({"id":APP,"revision":h.revision,"view":"view-1","channel":channel,
        "context":"kind-dev","namespace":"team",
        "source":{"kind":"logProvider","provider":"loki","resourceKind":"/Pod","name":"web-1",
            "tailLines":100,"sinceSeconds":600,"timestamps":true}})
}

async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never happened: {what}");
}

fn data(sink: &TestSink, channel: &str) -> Vec<Value> {
    sink.payloads_for(channel)
        .into_iter()
        .filter(|frame| frame["type"] == "data")
        .map(|frame| frame["data"].clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_provider_streams_its_history_then_each_new_line_once() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counted = polls.clone();
    let loki = server(move |target| {
        let asked = params(target);
        if asked["direction"] == "backward" {
            return streams(&[(20, "two"), (10, "one")]);
        }
        // The first poll starts at the last line it has, so Loki answers it again.
        match counted.fetch_add(1, Ordering::SeqCst) {
            0 => streams(&[(20, "two"), (30, "three")]),
            _ => streams(&[]),
        }
    })
    .await;
    let h = harness(&loki, "kind-dev").await;
    let sink = Arc::new(TestSink::default());
    h.streams
        .open(sink.clone(), follow(&h, "extstream:loki"))
        .await
        .unwrap();
    eventually("the new line", || {
        data(&sink, "extstream:loki")
            .iter()
            .any(|frame| frame.to_string().contains("three"))
    })
    .await;
    let frames = data(&sink, "extstream:loki");
    let lines: Vec<String> = frames
        .iter()
        .filter(|frame| frame["event"] == "lines")
        .flat_map(|frame| frame["lines"].as_array().unwrap().clone())
        .map(|line| line["line"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        lines,
        [
            "1970-01-01T00:00:00.000000010Z one",
            "1970-01-01T00:00:00.000000020Z two",
            "1970-01-01T00:00:00.000000030Z three"
        ]
    );
    assert_eq!(frames[0]["lines"][0]["source"], "web-1/app");
    assert_eq!(
        frames[1],
        json!({"event":"status","source":"loki","status":"live"})
    );
    let seen = loki.seen();
    let history = params(&seen[0].target);
    assert_eq!(history["limit"], "100");
    assert_eq!(
        number(&history, "end") - number(&history, "start"),
        600_000_000_000
    );
    let poll = params(&seen[1].target);
    assert_eq!(
        (poll["direction"].as_str(), number(&poll, "start")),
        ("forward", 20)
    );
    // Closing the view ends the stream, and nothing is asked after it.
    assert_eq!(h.streams.close_view("view-1"), 1);
    let asked = loki.seen().len();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(loki.seen().len(), asked);
    assert_eq!(
        sink.payloads_for("extstream:loki").last().unwrap()["type"],
        "close"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_poll_reconnects_and_a_refusal_ends_the_follow() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counted = polls.clone();
    let loki = server(move |target| {
        if params(target)["direction"] == "backward" {
            return streams(&[(10, "one")]);
        }
        match counted.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::Status(503, "Service Unavailable"),
            1 => streams(&[(20, "two")]),
            _ => Reply::Status(403, "Forbidden"),
        }
    })
    .await;
    let h = harness(&loki, "kind-dev").await;
    let sink = Arc::new(TestSink::default());
    h.streams
        .open(sink.clone(), follow(&h, "extstream:loki"))
        .await
        .unwrap();
    eventually("the terminal frame", || {
        sink.payloads_for("extstream:loki")
            .iter()
            .any(|frame| frame["type"] == "error")
    })
    .await;
    let statuses: Vec<String> = data(&sink, "extstream:loki")
        .iter()
        .filter(|frame| frame["event"] == "status")
        .map(|frame| frame["status"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(statuses, ["live", "reconnecting", "live"]);
    let error = sink.payloads_for("extstream:loki").last().unwrap().clone();
    assert_eq!(error["code"], "source");
    assert!(
        error["message"].as_str().unwrap().contains("HTTP 403"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_follow_is_opened_only_through_a_log_provider_for_the_views_kind() {
    let loki = server(|_| streams(&[])).await;
    let h = harness(&loki, "kind-dev").await;
    let sink = Arc::new(TestSink::default());
    let mut input = follow(&h, "extstream:a");
    input["source"]["provider"] = json!("cpu");
    let refused = h.streams.open(sink.clone(), input).await.unwrap_err();
    assert!(refused.contains("not a log provider"), "{refused}");
    let mut input = follow(&h, "extstream:b");
    input["source"]["resourceKind"] = json!("apps/Deployment");
    let refused = h.streams.open(sink.clone(), input).await.unwrap_err();
    assert!(refused.contains("not for apps/Deployment"), "{refused}");
    let mut input = follow(&h, "extstream:c");
    input["source"]["tailLines"] = json!(MAX_HISTORY + 1);
    assert!(h.streams.open(sink.clone(), input).await.is_err());
    assert!(loki.seen().is_empty());
}

#[test]
fn the_access_review_lists_each_provider_with_its_query() {
    let grants: Vec<String> = vec!["network.http".into()];
    let old = Manifest::parse(&observability().to_string()).unwrap();
    let mut changed = observability();
    changed["contributions"]["metricProviders"][0]["query"] =
        json!("sum(rate(container_cpu_usage_seconds_total{namespace=\"${namespace}\"}[${step}]))");
    let new = Manifest::parse(&changed.to_string()).unwrap();
    let diff = super::super::permission_diff(Some((&old, &grants, 1)), &new, &grants);
    assert_eq!(diff.added.len(), 1, "{diff:?}");
    assert_eq!(diff.removed.len(), 1, "{diff:?}");
    assert!(
        diff.added[0].starts_with("Query metrics cpu with PromQL")
            && diff.added[0].contains("sum(rate(container_cpu_usage_seconds_total")
            && diff.added[0].contains("through prom")
            && diff.added[0].contains("apps/Deployment"),
        "{}",
        diff.added[0]
    );
    // A log provider says it asks again while a log view is open.
    assert!(
        diff.unchanged
            .iter()
            .any(|item| item.starts_with("Follow logs loki with LogQL")
                && item.contains("every 5 s while a log view is open")),
        "{diff:?}"
    );
}

/// A reference example as written, under [`APP`]: the examples' `org.srelens` IDs are
/// reserved for signed releases, which a test does not install.
fn reference(source: &str) -> Value {
    let mut manifest: Value = serde_json::from_str(source).unwrap();
    manifest["id"] = json!(APP);
    manifest
}

#[tokio::test(flavor = "multi_thread")]
async fn the_prometheus_reference_charts_each_workload_and_pod_query() {
    let prometheus = server(|target| {
        if target.starts_with("/api/v1/query_range?") {
            matrix(target, 2)
        } else {
            Reply::Status(404, "Not Found")
        }
    })
    .await;
    let url = format!("http://{}", prometheus.addr);
    let h = harness_for(
        "kind-dev",
        reference(include_str!(
            "../../../../examples/extensions/prometheus.json"
        )),
        json!({"prometheusUrl": url}),
    )
    .await;
    for (provider, kind, name) in [
        ("cpu", "apps/Deployment", "web"),
        ("memory", "apps/StatefulSet", "db"),
        ("podCpu", "/Pod", "web-1"),
        ("podMemory", "/Pod", "web-1"),
    ] {
        let answer = h
            .query(json!({"provider":provider,"resourceKind":kind,"name":name}))
            .await
            .unwrap_or_else(|e| panic!("{provider}: {e}"));
        assert_eq!(answer["kind"], "metrics", "{provider}");
        assert_eq!(
            answer["chart"]["series"].as_array().unwrap().len(),
            2,
            "{provider}"
        );
    }
    let asked: Vec<String> = prometheus
        .seen()
        .iter()
        .map(|seen| params(&seen.target)["query"].clone())
        .collect();
    assert_eq!(
        asked[0],
        "sum by (pod) (rate(container_cpu_usage_seconds_total{namespace=\"team\", pod=~\"web-.+\", container!=\"\"}[5m]))"
    );
    assert!(asked[2].contains("pod=\"web-1\""), "{}", asked[2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_loki_reference_follows_a_pod_and_reads_a_workloads_lines() {
    let loki = server(|target| {
        if target.starts_with("/loki/api/v1/query_range?") {
            streams(&[(1_700_000_000_000_000_000, "ready")])
        } else {
            Reply::Status(404, "Not Found")
        }
    })
    .await;
    let url = format!("http://{}", loki.addr);
    let h = harness_for(
        "kind-dev",
        reference(include_str!("../../../../examples/extensions/loki.json")),
        json!({"lokiUrl": url}),
    )
    .await;
    let answer = h
        .query(json!({"provider":"workload","resourceKind":"apps/Deployment","name":"web"}))
        .await
        .unwrap();
    assert_eq!(answer["lines"][0]["line"], "ready");
    assert_eq!(
        params(&loki.seen()[0].target)["query"],
        "{namespace=\"team\", pod=~\"web-.+\"}"
    );
    let sink = Arc::new(TestSink::default());
    let mut input = follow(&h, "extstream:reference");
    input["source"]["provider"] = json!("pod");
    h.streams.open(sink.clone(), input).await.unwrap();
    eventually("the reference's first line", || {
        data(&sink, "extstream:reference")
            .iter()
            .any(|frame| frame.to_string().contains("ready"))
    })
    .await;
    assert_eq!(h.streams.close_view("view-1"), 1);
}
