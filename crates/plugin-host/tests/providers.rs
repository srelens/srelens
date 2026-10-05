//! Metric, log and trace providers (#569): the query templates an app declares,
//! the variables the host binds into them, and how each value is escaped for its
//! query language so that a resource or cluster name can never become query syntax.
use proptest::prelude::*;
use serde_json::{json, Value};
use srelens_plugin_host::{
    host_parameters, Manifest, ProviderKind, QueryLanguage, QueryTemplate, QueryValues,
    ValidationCode, ValidationError, Variable,
};

/// Prometheus, Loki and Tempo behind one app, each reached through its own
/// `network.http` binding.
fn manifest() -> Value {
    json!({
        "id":"org.example.observability", "name":"Observability", "version":"0.1.0",
        "srelensApiVersion":"^0.7", "kind":"declarative",
        "permissions":[{"capability":"network.http","hosts":["prometheus.example.com","loki.example.com","tempo.example.com"]}],
        "capabilities":[
            {"name":"prom","title":"Prometheus range query","target":"network.http","inputs":[],
                "arguments":{"url":"https://prometheus.example.com","path":"/api/v1/query_range"}},
            {"name":"loki","title":"Loki range query","target":"network.http","inputs":[],
                "arguments":{"url":"https://loki.example.com","path":"/loki/api/v1/query_range"}},
            {"name":"tempo","title":"Tempo search","target":"network.http","inputs":[],
                "arguments":{"url":"https://tempo.example.com","path":"/api/search"}}
        ],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[],
            "metricProviders":[{"id":"cpu","title":"CPU","capability":"prom","language":"promql",
                "forKinds":["apps/Deployment","apps/StatefulSet"],"unit":"cores",
                "query":"sum by (pod) (rate(container_cpu_usage_seconds_total{namespace=\"${namespace}\",pod=~\"${workload:regex}-.*\"}[${step}]))"}],
            "logProviders":[{"id":"loki","title":"Loki","capability":"loki","language":"logql",
                "forKinds":["/Pod"],"query":"{namespace=\"${namespace}\", pod=\"${pod}\"}"}],
            "traceProviders":[{"id":"traces","title":"Recent traces","capability":"tempo","language":"traceql",
                "forKinds":["apps/Deployment","/Pod"],
                "query":"{ resource.k8s.namespace.name = \"${namespace}\" && resource.k8s.cluster.name = \"${cluster}\" }"}]
        }
    })
}

fn parse(value: &Value) -> Manifest {
    Manifest::parse(&value.to_string()).unwrap_or_else(|e| panic!("valid manifest: {e}"))
}

fn errors(value: &Value) -> Vec<ValidationError> {
    Manifest::parse(&value.to_string())
        .expect_err("an invalid manifest")
        .0
}

/// `(code, path)` of every problem, sorted.
fn problems(value: &Value) -> Vec<(ValidationCode, String)> {
    let mut found: Vec<_> = errors(value)
        .into_iter()
        .map(|error| (error.code, error.path))
        .collect();
    found.sort();
    found
}

fn with_metric_query(query: &str) -> Value {
    let mut value = manifest();
    value["contributions"]["metricProviders"][0]["query"] = json!(query);
    value
}

fn on_workload(name: &str) -> QueryValues {
    QueryValues {
        cluster: "kind-dev".into(),
        namespace: "team".into(),
        workload: Some(name.into()),
        pod: None,
        range_seconds: Some(3600),
        step_seconds: Some(15),
    }
}

fn template(language: QueryLanguage, query: &str) -> QueryTemplate {
    QueryTemplate::parse(language, query).unwrap_or_else(|e| panic!("{query}: {e}"))
}

// ---------------------------------------------------------------------------
// A tokenizer written here, independently of the host's, that reads a query the
// way PromQL, LogQL and TraceQL read double-quoted strings: `\` escapes the next
// character. It is what proves a bound value stayed inside its string.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Token {
    /// Everything outside a double-quoted string, run together.
    Outside(String),
    /// A double-quoted string's value, escapes undone.
    Quoted(String),
}

fn tokens(query: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut outside = String::new();
    let mut chars = query.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            outside.push(c);
            continue;
        }
        if !outside.is_empty() {
            out.push(Token::Outside(std::mem::take(&mut outside)));
        }
        let mut quoted = String::new();
        loop {
            match chars.next().expect("a closed string") {
                '\\' => quoted.push(chars.next().expect("an escaped character")),
                '"' => break,
                other => quoted.push(other),
            }
        }
        out.push(Token::Quoted(quoted));
    }
    if !outside.is_empty() {
        out.push(Token::Outside(outside));
    }
    out
}

// ---------------------------------------------------------------------------
// The manifest contract
// ---------------------------------------------------------------------------

#[test]
fn providers_parse_and_are_stored_as_written() {
    let parsed = parse(&manifest());
    let stored = serde_json::to_value(&parsed).unwrap();
    for list in ["metricProviders", "logProviders", "traceProviders"] {
        assert_eq!(
            stored["contributions"][list],
            manifest()["contributions"][list],
            "{list}"
        );
    }
    let providers = parsed.providers();
    let found: Vec<_> = providers
        .iter()
        .map(|provider| (provider.kind, provider.id, provider.language))
        .collect();
    assert_eq!(
        found,
        [
            (ProviderKind::Metrics, "cpu", QueryLanguage::Promql),
            (ProviderKind::Logs, "loki", QueryLanguage::Logql),
            (ProviderKind::Traces, "traces", QueryLanguage::Traceql),
        ]
    );
    let cpu = parsed.provider("cpu").expect("the metric provider");
    assert_eq!(cpu.capability, "prom");
    assert_eq!(cpu.for_kinds, ["apps/Deployment", "apps/StatefulSet"]);
    assert_eq!(
        serde_json::to_value(cpu.unit.expect("a unit")).unwrap(),
        "cores"
    );
    assert!(parsed.provider("missing").is_none());
}

#[test]
fn a_provider_names_a_declared_network_http_binding() {
    let mut value = manifest();
    value["contributions"]["metricProviders"][0]["capability"] = json!("nothing");
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::UnresolvedCapability,
            "contributions.metricProviders[0].capability".into()
        )]
    );
    // A reader is not a request a provider can send its query through.
    let mut value = manifest();
    value["permissions"]
        .as_array_mut()
        .unwrap()
        .push(json!("k8s.listDeployments"));
    value["capabilities"].as_array_mut().unwrap().push(
        json!({"name":"deployments","title":"Deployments","target":"k8s.listDeployments",
            "inputs":["context","namespace"],"arguments":{}}),
    );
    value["contributions"]["logProviders"][0]["capability"] = json!("deployments");
    let found = errors(&value);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].code, ValidationCode::InvalidBinding);
    assert_eq!(found[0].path, "contributions.logProviders[0].capability");
    assert!(
        found[0].message.contains("network.http"),
        "{}",
        found[0].message
    );
}

#[test]
fn each_list_queries_in_its_own_language() {
    let mut value = manifest();
    value["contributions"]["metricProviders"][0]["language"] = json!("logql");
    let found = errors(&value);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].path, "contributions.metricProviders[0].language");
    assert!(found[0].message.contains("promql"), "{}", found[0].message);
}

#[test]
fn provider_ids_are_identifiers_unique_across_every_list() {
    let mut value = manifest();
    value["contributions"]["traceProviders"][0]["id"] = json!("cpu");
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::DuplicateIdentifier,
            "contributions.traceProviders[0].id".into()
        )]
    );
    let mut value = manifest();
    value["contributions"]["logProviders"][0]["id"] = json!("not an id");
    value["contributions"]["logProviders"][0]["title"] = json!("");
    assert_eq!(
        problems(&value),
        [
            (
                ValidationCode::InvalidValue,
                "contributions.logProviders[0].id".into()
            ),
            (
                ValidationCode::InvalidValue,
                "contributions.logProviders[0].title".into()
            ),
        ]
    );
}

#[test]
fn a_provider_is_for_workloads_and_pods_only() {
    let mut value = manifest();
    value["contributions"]["traceProviders"][0]["forKinds"] = json!(["batch/Job"]);
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::InvalidKind,
            "contributions.traceProviders[0].forKinds[0]".into()
        )]
    );
    let mut value = manifest();
    value["contributions"]["traceProviders"][0]["forKinds"] = json!([]);
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::InvalidValue,
            "contributions.traceProviders[0].forKinds".into()
        )]
    );
    let mut value = manifest();
    value["contributions"]["traceProviders"][0]["forKinds"] = json!(["/Pod", "/Pod"]);
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::DuplicateIdentifier,
            "contributions.traceProviders[0].forKinds[1]".into()
        )]
    );
}

#[test]
fn each_list_is_bounded() {
    let mut value = manifest();
    let providers: Vec<Value> = (0..17)
        .map(|index| {
            let mut provider = manifest()["contributions"]["metricProviders"][0].clone();
            provider["id"] = json!(format!("cpu-{index}"));
            provider
        })
        .collect();
    value["contributions"]["metricProviders"] = json!(providers);
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::InvalidValue,
            "contributions.metricProviders".into()
        )]
    );
}

#[test]
fn the_host_sets_the_query_and_time_parameters_itself() {
    // A binding a PromQL provider sends through may not fix what the host binds.
    let mut value = manifest();
    value["capabilities"][0]["arguments"]["query"] = json!({"step": "1m", "dedup": "true"});
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::InvalidBinding,
            "capabilities[0].arguments.query.step".into()
        )]
    );
    // Tempo's search takes its query as `q`.
    let mut value = manifest();
    value["capabilities"][2]["arguments"]["query"] = json!({"q": "{}"});
    assert_eq!(
        problems(&value),
        [(
            ValidationCode::InvalidBinding,
            "capabilities[2].arguments.query.q".into()
        )]
    );
    assert_eq!(
        host_parameters(QueryLanguage::Promql),
        ["query", "start", "end", "step"]
    );
    assert_eq!(
        host_parameters(QueryLanguage::Logql),
        ["query", "start", "end", "limit", "direction"]
    );
    assert_eq!(
        host_parameters(QueryLanguage::Traceql),
        ["q", "start", "end", "limit"]
    );
}

// ---------------------------------------------------------------------------
// Templates: where a variable may stand
// ---------------------------------------------------------------------------

#[test]
fn an_identity_variable_stands_only_inside_a_double_quoted_string() {
    for query in [
        // Bare, where a value would be syntax.
        "up{namespace=${namespace}}",
        "sum(up) by (${namespace})",
        // Inside a raw string or a single-quoted one, which escape differently or not at all.
        "up{namespace=`${namespace}`}",
        "up{namespace='${namespace}'}",
    ] {
        let found = errors(&with_metric_query(query));
        assert_eq!(found.len(), 1, "{query}: {found:?}");
        assert_eq!(found[0].code, ValidationCode::InvalidBinding, "{query}");
        assert_eq!(found[0].path, "contributions.metricProviders[0].query");
        assert!(
            found[0].message.contains("${namespace}"),
            "{query}: {}",
            found[0].message
        );
    }
}

#[test]
fn a_duration_variable_stands_only_outside_a_string() {
    parse(&with_metric_query(
        "increase(http_requests_total{namespace=\"${namespace}\"}[${range}])",
    ));
    let found = errors(&with_metric_query("up{interval=\"${step}\"}"));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].path, "contributions.metricProviders[0].query");
    assert!(found[0].message.contains("${step}"), "{}", found[0].message);
}

#[test]
fn unknown_variables_settings_and_malformed_templates_are_refused() {
    for (query, says) in [
        ("up{job=\"${job}\"}", "${job}"),
        // The settings rule's own refusal, at the same path: a setting reaches only a
        // binding argument its capability marks settable.
        (
            "up{job=\"${settings.job}\"}",
            "Settings are interpolated only",
        ),
        ("up{job=\"${namespace:json}\"}", ":json"),
        ("up{job=\"${namespace\"}", "${"),
        ("up{job=\"unterminated}", "string"),
        ("up # a comment", "#"),
        ("up\n+ up", "control"),
        ("", "1–2048"),
    ] {
        let found = errors(&with_metric_query(query));
        assert_eq!(found.len(), 1, "{query:?}: {found:?}");
        assert_eq!(found[0].code, ValidationCode::InvalidBinding, "{query:?}");
        assert_eq!(found[0].path, "contributions.metricProviders[0].query");
        assert!(
            found[0].message.contains(says),
            "{query:?}: {}",
            found[0].message
        );
    }
    let long = format!("up{{job=\"{}\"}}", "a".repeat(2048));
    assert_eq!(errors(&with_metric_query(&long)).len(), 1);
}

#[test]
fn a_variable_must_be_known_on_every_kind_the_provider_is_for() {
    // `pod` is known where the view is a pod; the metric provider is for workloads.
    let found = errors(&with_metric_query("up{pod=\"${pod}\"}"));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].path, "contributions.metricProviders[0].query");
    assert!(
        found[0].message.contains("${pod}") && found[0].message.contains("apps/Deployment"),
        "{}",
        found[0].message
    );
    // `workload` is not known on a Pod, which the trace provider is also for.
    let mut value = manifest();
    value["contributions"]["traceProviders"][0]["query"] =
        json!("{ resource.k8s.deployment.name = \"${workload}\" }");
    let found = errors(&value);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].message.contains("/Pod"), "{}", found[0].message);
    // A time range belongs to a metric panel; a log follow and a trace search set their own.
    let mut value = manifest();
    value["contributions"]["logProviders"][0]["query"] =
        json!("{pod=\"${pod}\"} |= \"x\" [${range}]");
    let found = errors(&value);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].path, "contributions.logProviders[0].query");
}

#[test]
fn log_and_trace_templates_are_held_to_their_languages_quoting() {
    // LogQL writes raw strings in backticks, and has no single-quoted string.
    let mut value = manifest();
    value["contributions"]["logProviders"][0]["query"] =
        json!("{pod=\"${pod}\"} |~ `level=(warn|error)`");
    parse(&value);
    value["contributions"]["logProviders"][0]["query"] = json!("{pod=\"${pod}\"} |= 'x'");
    assert_eq!(errors(&value).len(), 1);
    // TraceQL strings are double-quoted only.
    let mut value = manifest();
    value["contributions"]["traceProviders"][0]["query"] =
        json!("{ resource.service.name = `checkout` }");
    assert_eq!(errors(&value).len(), 1);
}

// ---------------------------------------------------------------------------
// Binding: the value a view names, escaped for the language
// ---------------------------------------------------------------------------

#[test]
fn bound_values_are_substituted_and_durations_written_in_seconds() {
    let parsed = parse(&manifest());
    let cpu = parsed.provider("cpu").unwrap();
    let template = template(cpu.language, cpu.query);
    assert_eq!(
        template.variables().into_iter().collect::<Vec<_>>(),
        [Variable::Namespace, Variable::Workload, Variable::Step]
    );
    assert_eq!(
        template.render(&on_workload("web")).unwrap(),
        "sum by (pod) (rate(container_cpu_usage_seconds_total{namespace=\"team\",pod=~\"web-.*\"}[15s]))"
    );
}

#[test]
fn a_cluster_name_cannot_close_its_string_or_add_to_the_query() {
    // A kubeconfig context name is free text; these are the attempts that matter.
    let template = template(
        QueryLanguage::Traceql,
        "{ resource.k8s.cluster.name = \"${cluster}\" }",
    );
    let bound = |cluster: &str| {
        template.render(&QueryValues {
            cluster: cluster.into(),
            ..on_workload("web")
        })
    };
    for hostile in [
        "prod\" } || { true",
        "prod\\\" } || { true",
        "prod\\",
        "a\"b\"c",
        "} or vector(1) #",
        "*/ || { true } /*",
        "{{ __line__ }}",
        "prod staging",
    ] {
        let refused = bound(hostile).unwrap_err();
        assert!(!refused.contains(hostile), "{refused}");
    }
    // The names kubeconfigs really carry reach the query whole, inside its string.
    for real in [
        "kind-dev",
        "arn:aws:eks:eu-west-1:123456789012:cluster/prod",
        "gke_my-project_europe-west1_prod",
        "default/api-crc-testing:6443/kubeadmin",
        "admin@prod.example.com",
    ] {
        let rendered = bound(real).unwrap();
        assert_eq!(
            tokens(&rendered),
            [
                Token::Outside("{ resource.k8s.cluster.name = ".into()),
                Token::Quoted(real.into()),
                Token::Outside(" }".into()),
            ],
            "{real:?} rendered as {rendered}"
        );
    }
}

#[test]
fn a_comment_the_language_would_skip_is_refused() {
    // LogQL's and TraceQL's lexers skip `/* */` and `//`: a quote inside one is no
    // string to them, so a value placed there would be read as query syntax.
    for (language, query) in [
        (
            QueryLanguage::Logql,
            "{pod=\"${pod}\"} /* \"${cluster}\" */",
        ),
        (
            QueryLanguage::Traceql,
            "{ span.x = \"a\" } /* \"${cluster}\" */",
        ),
        (
            QueryLanguage::Traceql,
            "{ span.x = \"a\" } // \"${cluster}\"",
        ),
        (QueryLanguage::Promql, "up /* \"${cluster}\" */"),
    ] {
        let refused = QueryTemplate::parse(language, query).unwrap_err();
        assert!(refused.contains("comment"), "{query}: {refused}");
    }
    // Inside a string they are only text.
    template(
        QueryLanguage::Logql,
        "{pod=\"${pod}\"} |= \"/* not a comment */\"",
    );
}

#[test]
fn a_name_in_a_regex_matcher_is_written_as_a_regex() {
    // Bare, `.*` or `prod|staging` as a cluster's name would match every cluster.
    for (language, query) in [
        (QueryLanguage::Promql, "up{cluster=~\"${cluster}\"}"),
        (QueryLanguage::Promql, "up{pod!~\"${workload}-.+\"}"),
        (
            QueryLanguage::Logql,
            "{namespace=\"${namespace}\"} |~ \"${pod}\"",
        ),
        (
            QueryLanguage::Traceql,
            "{ resource.service.name =~ \"${workload}\" }",
        ),
        // A line filter's `or` alternatives are patterns of the same filter.
        (
            QueryLanguage::Logql,
            "{namespace=\"${namespace}\"} |~ \"error\" or \"${pod}\"",
        ),
        (
            QueryLanguage::Logql,
            "{namespace=\"${namespace}\"} !~ `error` or\"${pod}\"",
        ),
    ] {
        let refused = QueryTemplate::parse(language, query).unwrap_err();
        assert!(refused.contains(":regex"), "{query}: {refused}");
    }
    template(
        QueryLanguage::Logql,
        "{namespace=\"${namespace}\"} |~ \"error\" or \"${pod:regex}\" or `panic`",
    );
    // An exact filter's alternatives are exact, and `or` between label filters
    // starts a new one.
    template(
        QueryLanguage::Logql,
        "{namespace=\"${namespace}\"} |= \"error\" or \"${pod}\"",
    );
    template(
        QueryLanguage::Logql,
        "{namespace=\"${namespace}\"} | level=~\"err|warn\" or pod=\"${pod}\"",
    );
    template(
        QueryLanguage::Logql,
        "{namespace=\"${namespace}\"} |~ \"${pod:regex}\"",
    );
    template(
        QueryLanguage::Traceql,
        "{ resource.service.name =~ \"${workload:regex}\" }",
    );
    // And quoted for a regex where the string is matched literally, it would carry
    // backslashes into an exact match.
    let refused =
        QueryTemplate::parse(QueryLanguage::Promql, "up{pod=\"${pod:regex}\"}").unwrap_err();
    assert!(refused.contains("=~"), "{refused}");
}

#[test]
fn a_line_or_paragraph_separator_is_refused_in_a_template() {
    for separator in ['\u{2028}', '\u{2029}'] {
        let query = format!("up{{job=\"a{separator}b\"}}");
        assert!(QueryTemplate::parse(QueryLanguage::Promql, &query).is_err());
    }
}

#[test]
fn a_regex_variable_matches_the_name_literally() {
    let template = template(QueryLanguage::Promql, "up{pod=~\"${workload:regex}-.*\"}");
    // `.` in a Kubernetes name is a regex metacharacter; quoted, it matches only a dot.
    let rendered = template.render(&on_workload("api.v2")).unwrap();
    assert_eq!(rendered, "up{pod=~\"api\\\\.v2-.*\"}");
    assert_eq!(
        tokens(&rendered),
        [
            Token::Outside("up{pod=~".into()),
            Token::Quoted("api\\.v2-.*".into()),
            Token::Outside("}".into()),
        ]
    );
}

#[test]
fn a_value_a_query_cannot_carry_is_refused_not_passed_on() {
    let template = template(
        QueryLanguage::Logql,
        "{namespace=\"${namespace}\", pod=\"${pod}\"}",
    );
    let pod = |pod: &str| QueryValues {
        workload: None,
        pod: Some(pod.into()),
        ..on_workload("web")
    };
    assert_eq!(
        template.render(&pod("web-1")).unwrap(),
        "{namespace=\"team\", pod=\"web-1\"}"
    );
    // The view names a pod and a namespace; both must be Kubernetes names.
    for bad in ["Web-1", "web\"1", "", &"a".repeat(254)] {
        assert!(template.render(&pod(bad)).is_err(), "{bad:?}");
    }
    let mut values = pod("web-1");
    values.namespace = "team\"}".into();
    assert!(template.render(&values).is_err());
    // A template that asks for the pod on a view that names none is refused, not left empty.
    assert!(template.render(&on_workload("web")).is_err());
    // A cluster name with a control character has no spelling every language accepts.
    let cluster = template_with_cluster();
    for bad in ["kind\ndev", "kind\u{202e}dev", ""] {
        let values = QueryValues {
            cluster: bad.into(),
            ..on_workload("web")
        };
        let error = cluster.render(&values).unwrap_err();
        assert!(!error.contains(bad) || bad.is_empty(), "{error}");
    }
}

fn template_with_cluster() -> QueryTemplate {
    template(QueryLanguage::Promql, "up{cluster=\"${cluster}\"}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Whatever the context is called, it is refused or it is the text of one string:
    /// the query outside that string is the template's.
    #[test]
    fn no_cluster_name_changes_the_query_outside_its_string(cluster in "\\PC{1,64}") {
        let template = template_with_cluster();
        let values = QueryValues { cluster: cluster.clone(), ..on_workload("web") };
        if let Ok(rendered) = template.render(&values) {
            prop_assert_eq!(
                tokens(&rendered),
                vec![
                    Token::Outside("up{cluster=".into()),
                    Token::Quoted(cluster),
                    Token::Outside("}".into()),
                ]
            );
        }
    }

    /// And a name made of what a context name may hold is always carried.
    #[test]
    fn a_name_of_the_allowed_characters_is_always_carried(cluster in "[A-Za-z0-9._:/@+-]{1,64}") {
        let template = template_with_cluster();
        let values = QueryValues { cluster: cluster.clone(), ..on_workload("web") };
        prop_assert!(template.render(&values).is_ok());
    }
}
