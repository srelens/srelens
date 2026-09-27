//! API 0.4 (#709): the fields added to API 0.3 in place while the extension platform was
//! being built, moved to a line of their own before a signed release used them.
//!
//! A host that implements 0.3 without them would meet them as unknown fields. So a
//! manifest whose range admits 0.3 may use none of them, and is told the version it
//! needs; one that requires `^0.4` may use every one; and a 0.3 manifest that uses none —
//! every signed release published so far — keeps installing.
use serde_json::{json, Value};
use srelens_plugin_host::{
    check_api_fields_in, matching_api_versions_in, negotiate_api_version, Manifest, ValidationCode,
    API_FIELDS, SUPPORTED_API_VERSIONS,
};

const READY: &str = ".status.conditions[?(@.type==\"Ready\")].status";

/// An Argo CD app written against API 0.3: a reader, a page and a guarded action.
///
/// Its printer column reads through the condition filter, as the published Flux 0.4.0
/// release does: printer columns evaluated that form before API 0.4, so it stays 0.3.
fn manifest() -> Value {
    json!({
        "id":"org.example.argocd", "name":"Argo CD", "version":"0.1.0", "srelensApiVersion":"^0.3",
        "kind":"declarative", "permissions":["k8s.listCustomResource","k8s.annotate"],
        "capabilities":[{"name":"applications","title":"List applications","target":"k8s.listCustomResource",
            "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"applications",
                "kind":"Application","namespaced":true,
                "printerColumns":[{"name":"Ready","jsonPath":READY,"type":"string"}]},
            "inputs":["context","namespace"]}],
        "actions":[{"name":"refresh","title":"Refresh","target":"k8s.annotate","resource":"applications",
            "arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"},
            "preconditions":[{"jsonPath":".operation","absent":true,"reason":"An operation is in progress"}],
            "availableWhen":[{"jsonPath":".operation","absent":true,"reason":"An operation is in progress"}]}],
        "contributions":{"pages":[{"id":"applications","title":"Applications","capability":"applications"}],
            "detailTabs":[],"detailLinks":[]}
    })
}

/// A predicate that asks about the Ready condition through the filter.
fn ready() -> Value {
    json!({"jsonPath":READY,"equals":"True","reason":"Wait until the application is ready"})
}

type Use = fn(&mut Value);

/// Each thing API 0.4 added, used validly, with the `API_FIELDS` entries it uses.
fn uses_of_0_4() -> Vec<(&'static [&'static str], Use)> {
    vec![
        (&["contributions.joins"], |v| {
            v["contributions"]["joins"] = json!([{"id":"app","capability":"applications",
                "match":{"label":"argocd.argoproj.io/instance"}}]);
        }),
        (&["contributions.tableColumns"], |v| {
            v["contributions"]["tableColumns"] = json!([{"id":"name","title":"Name",
                "forKinds":["apps/Deployment"],"source":{"jsonPath":".metadata.name"},"format":"text"}]);
        }),
        (&["contributions.detailPanels"], |v| {
            v["contributions"]["detailPanels"] = json!([{"id":"health","title":"Health",
                "forKinds":["argoproj.io/Application"],
                "sections":[{"type":"fields","fields":[{"label":"Health","jsonPath":".status.health.status"}]}]}]);
        }),
        (&["contributions.statusResolvers"], |v| {
            v["contributions"]["statusResolvers"] = json!([{"forKinds":["argoproj.io/Application"],
                "rules":[{"when":[{"jsonPath":READY,"equals":"True"}],"status":"healthy","label":"Ready"}]}]);
        }),
        (&["contributions.badges"], |v| {
            v["contributions"]["badges"] = json!([{"id":"managed","forKinds":["apps/Deployment"],
                "rules":[{"when":[{"jsonPath":".metadata.labels['argocd.argoproj.io/instance']","present":true}],
                    "status":"healthy","label":"Argo CD"}]}]);
        }),
        (&["contributions.dashboardCards"], |v| {
            v["contributions"]["dashboardCards"] = json!([{"id":"apps","title":"Applications",
                "size":"s","type":"count","source":"applications"}]);
        }),
        (&["contributions.commands"], |v| {
            v["contributions"]["commands"] = json!([{"id":"open","title":"Open applications",
                "target":{"page":"applications"}}]);
        }),
        (&["contributions.resourceLinks"], |v| {
            v["contributions"]["resourceLinks"] = json!([{"id":"owner","from":"apps/Deployment",
                "to":"argoproj.io/Application","relation":"managedBy",
                "match":{"label":"argocd.argoproj.io/instance"}}]);
        }),
        (&["settings"], |v| {
            v["settings"] = json!([{"id":"note","type":"string","title":"Note"}]);
        }),
        (&["capabilities[].versions"], |v| {
            let binding = &mut v["capabilities"][0];
            binding["arguments"]
                .as_object_mut()
                .unwrap()
                .remove("version");
            binding["versions"] = json!(["v1alpha1"]);
        }),
        (
            &[
                "capabilities[].versions",
                "capabilities[].jsonPathOverrides",
            ],
            |v| {
                let binding = &mut v["capabilities"][0];
                binding["arguments"]
                    .as_object_mut()
                    .unwrap()
                    .remove("version");
                binding["versions"] = json!(["v1alpha1", "v1beta1"]);
                binding["jsonPathOverrides"] =
                    json!({"v1beta1":{".operation":".status.operation"}});
            },
        ),
        (&["actions[].preconditions[].jsonPath"], |v| {
            v["actions"][0]["preconditions"][0] = ready();
        }),
        (&["actions[].availableWhen[].jsonPath"], |v| {
            v["actions"][0]["availableWhen"][0] = ready();
        }),
        // Brokered HTTP (#568): new in 0.4, not moved there from 0.3.
        (&["permissions[].hosts", "permissions[].capability"], |v| {
            v["permissions"]
                .as_array_mut()
                .unwrap()
                .push(json!({"capability":"network.http","hosts":["api.github.com"]}));
            v["capabilities"].as_array_mut().unwrap().push(json!({"name":"latest",
                "title":"Latest release","target":"network.http","inputs":[],
                "arguments":{"url":"https://api.github.com","path":"/repos/argoproj/argo-cd/releases/latest"}}));
        }),
    ]
}

fn with_range(mut value: Value, range: &str) -> String {
    value["srelensApiVersion"] = json!(range);
    value.to_string()
}

#[test]
fn a_0_3_manifest_that_uses_none_of_them_still_installs() {
    let source = manifest().to_string();
    let parsed = Manifest::parse(&source).unwrap();
    let range = semver::VersionReq::parse(&parsed.api_version).unwrap();
    assert_eq!(
        negotiate_api_version(&range).map(|v| v.to_string()),
        Some("0.3.0".into())
    );
    // Reverification of the stored form, too, so an installed 0.3 app stays enabled.
    let stored: Manifest = serde_json::from_value(manifest()).unwrap();
    stored
        .check_api_fields(SUPPORTED_API_VERSIONS, API_FIELDS)
        .unwrap();
}

#[test]
fn every_0_4_addition_under_a_0_3_range_is_told_it_requires_api_0_4() {
    for (fields, apply) in uses_of_0_4() {
        let mut value = manifest();
        apply(&mut value);
        // Valid on the line it belongs to, so nothing else below is at fault.
        for range in ["^0.4", ">=0.4, <0.5"] {
            Manifest::parse(&with_range(value.clone(), range))
                .unwrap_or_else(|e| panic!("{fields:?} under {range}: {e}"));
        }
        // Any range that admits 0.3 claims hosts that implement 0.3 without it.
        for range in ["^0.3", ">=0.3, <0.5"] {
            let errors = Manifest::parse(&with_range(value.clone(), range))
                .expect_err(&format!("{fields:?} under {range}"))
                .0;
            assert_eq!(errors.len(), 1, "{fields:?} under {range}: {errors:?}");
            let error = &errors[0];
            assert_eq!(error.code, ValidationCode::ApiIncompatible, "{error:?}");
            assert_eq!(error.path, "srelensApiVersion", "{error:?}");
            assert!(
                error.message.contains("requires API 0.4.0")
                    && error.message.contains("admits API 0.3.0"),
                "{error:?}"
            );
            assert!(
                fields
                    .iter()
                    .any(|field| error.message.contains(&format!("`{field}`"))),
                "{fields:?}: {error:?}"
            );
        }
    }
}

#[test]
fn every_0_4_entry_in_the_table_has_a_case_above() {
    let covered: Vec<&str> = uses_of_0_4()
        .iter()
        .flat_map(|(fields, _)| fields.iter().copied())
        .collect();
    let gated: Vec<&str> = API_FIELDS
        .iter()
        .filter(|field| field.introduced == "0.4.0")
        .map(|field| field.path)
        .collect();
    assert!(!gated.is_empty());
    for path in &gated {
        assert!(covered.contains(path), "{path} has no case");
    }
    for path in covered {
        assert!(gated.contains(&path), "{path} is not in API_FIELDS");
    }
}

#[test]
fn the_filter_is_refused_by_its_parsed_form_not_its_spelling() {
    let admits_0_3 = matching_api_versions_in(
        &semver::VersionReq::parse("^0.3").unwrap(),
        SUPPORTED_API_VERSIONS,
    );
    let mut value = manifest();
    // A quoted key holding the characters is a key, and API 0.3 reads it.
    value["actions"][0]["preconditions"][0]["jsonPath"] =
        json!(".metadata.annotations['acme.io/[?(pinned']");
    check_api_fields_in(&value, &admits_0_3, API_FIELDS).unwrap();
    value["actions"][0]["preconditions"][0]["jsonPath"] = json!(READY);
    let error = check_api_fields_in(&value, &admits_0_3, API_FIELDS).unwrap_err();
    assert!(
        error.starts_with(
            "the `[?(@.key==\"text\")]` filter in `actions[].preconditions[].jsonPath` requires API 0.4.0"
        ),
        "{error}"
    );
}

#[test]
fn a_0_3_app_installed_with_a_0_4_field_is_quarantined_on_reverification() {
    // A prerelease host accepted these fields under ^0.3. What it stored reverifies
    // against the same table, so it is disabled with the version it needs rather than
    // left enabled on a range that claims hosts without the field.
    let mut value = manifest();
    value["contributions"]["commands"] =
        json!([{"id":"open","title":"Open applications","target":{"page":"applications"}}]);
    let stored: Manifest = serde_json::from_value(value).unwrap();
    let reason = stored
        .check_api_fields(SUPPORTED_API_VERSIONS, API_FIELDS)
        .unwrap_err();
    assert!(
        reason.contains("`contributions.commands` requires API 0.4.0"),
        "{reason}"
    );
}

#[test]
fn a_0_4_manifest_is_incompatible_with_a_host_that_implements_only_0_3() {
    // What a host on the 0.3 line checks first: a range it has no version for is refused
    // as "requires API ^0.4; host supports 0.3.0" before any field is read, and its
    // catalog lists the release as incompatible rather than offering it.
    let only_0_3 = ["0.3.0"];
    for range in ["^0.4", ">=0.4, <0.5"] {
        let range = semver::VersionReq::parse(range).unwrap();
        assert!(matching_api_versions_in(&range, &only_0_3).is_empty());
        assert_eq!(
            negotiate_api_version(&range).map(|v| v.to_string()),
            Some("0.4.0".into())
        );
    }
}

/// Each thing API 0.5 added (#728), used validly, with the `API_FIELDS` entry it uses:
/// a link's spec `path`, and a link's `to` naming a built-in kind. API 0.4 was published
/// in srelens builds without them, so a `^0.4` manifest may use neither.
fn uses_of_0_5() -> Vec<(&'static str, Use)> {
    vec![
        ("contributions.resourceLinks[].match.path", |v| {
            // Toward the reader's own kind, so the path is the only new thing.
            v["contributions"]["resourceLinks"] = json!([{"id":"parent",
                "from":"argoproj.io/Application","to":"argoproj.io/Application",
                "relation":"references","match":{"path":".spec.parentRefs[*]"}}]);
        }),
        ("contributions.resourceLinks[].to", |v| {
            v["contributions"]["resourceLinks"] = json!([{"id":"account",
                "from":"apps/Deployment","to":"/ServiceAccount","relation":"references",
                "match":{"label":"example.io/service-account"}}]);
        }),
        // Logs, exec and port-forwards (#567): a pod binding scoped by the object a
        // reader lists is a new target, and no new field.
        ("capabilities[].target", |v| {
            v["permissions"]
                .as_array_mut()
                .unwrap()
                .push(json!("k8s.streamLogs"));
            v["capabilities"].as_array_mut().unwrap().push(json!({"name":"logs",
                "title":"Logs","target":"k8s.streamLogs","inputs":[],
                "arguments":{"resource":"applications","selector":".spec.selector"}}));
        }),
        // One scoped by the namespaces its permission grants. A grant needs a pod
        // binding, so it is listed before the target and is what the refusal names.
        ("permissions[].namespaces", |v| {
            v["permissions"]
                .as_array_mut()
                .unwrap()
                .push(json!({"capability":"k8s.exec","namespaces":["argocd"]}));
            v["capabilities"].as_array_mut().unwrap().push(json!({"name":"version",
                "title":"Argo CD version","target":"k8s.exec","inputs":[],
                "arguments":{"command":["argocd","version","--client"]}}));
        }),
        // Metric, log and trace providers (#569), each through a `network.http`
        // binding, which API 0.4 already had: the provider list is the one new field.
        ("contributions.metricProviders", |v| {
            with_request(v, "/api/v1/query_range");
            v["contributions"]["metricProviders"] = json!([{"id":"cpu","title":"CPU",
                "capability":"request","language":"promql","forKinds":["apps/Deployment"],
                "unit":"cores","query":"sum(rate(container_cpu_usage_seconds_total{namespace=\"${namespace}\"}[${step}]))"}]);
        }),
        ("contributions.logProviders", |v| {
            with_request(v, "/loki/api/v1/query_range");
            v["contributions"]["logProviders"] = json!([{"id":"loki","title":"Loki",
                "capability":"request","language":"logql","forKinds":["/Pod"],
                "query":"{namespace=\"${namespace}\", pod=\"${pod}\"}"}]);
        }),
        ("contributions.traceProviders", |v| {
            with_request(v, "/api/search");
            v["contributions"]["traceProviders"] = json!([{"id":"traces","title":"Traces",
                "capability":"request","language":"traceql","forKinds":["/Pod"],
                "query":"{ resource.k8s.pod.name = \"${pod}\" }"}]);
        }),
    ]
}

/// A `network.http` binding named `request` to `path` on one granted host.
fn with_request(v: &mut Value, path: &str) {
    v["permissions"].as_array_mut().unwrap().push(
        json!({"capability":"network.http","hosts":["observability.example.com"]}),
    );
    v["capabilities"].as_array_mut().unwrap().push(json!({"name":"request",
        "title":"Query","target":"network.http","inputs":[],
        "arguments":{"url":"https://observability.example.com","path":path}}));
}

#[test]
fn every_0_5_addition_under_a_0_4_range_is_told_it_requires_api_0_5() {
    for (field, apply) in uses_of_0_5() {
        let mut value = manifest();
        apply(&mut value);
        for range in ["^0.5", ">=0.5, <0.6"] {
            Manifest::parse(&with_range(value.clone(), range))
                .unwrap_or_else(|e| panic!("{field} under {range}: {e}"));
        }
        // A range that admits 0.4 claims the hosts published on that line.
        for range in ["^0.4", ">=0.4, <0.6"] {
            let errors = Manifest::parse(&with_range(value.clone(), range))
                .expect_err(&format!("{field} under {range}"))
                .0;
            assert_eq!(errors.len(), 1, "{field} under {range}: {errors:?}");
            let error = &errors[0];
            assert_eq!(error.code, ValidationCode::ApiIncompatible, "{error:?}");
            assert_eq!(error.path, "srelensApiVersion", "{error:?}");
            assert!(
                error.message.contains("requires API 0.5.0")
                    && error.message.contains("admits API 0.4.0")
                    && error.message.contains(&format!("`{field}`")),
                "{error:?}"
            );
        }
    }
}

#[test]
fn every_0_5_entry_in_the_table_has_a_case_above() {
    let covered: Vec<&str> = uses_of_0_5().iter().map(|(field, _)| *field).collect();
    let gated: Vec<&str> = API_FIELDS
        .iter()
        .filter(|field| field.introduced == "0.5.0")
        .map(|field| field.path)
        .collect();
    assert_eq!(gated.len(), covered.len(), "{gated:?}");
    for path in &gated {
        assert!(covered.contains(path), "{path} has no case");
    }
}

#[test]
fn a_link_to_a_kind_a_reader_lists_stays_a_0_4_link() {
    // The built-in form is decided by the value: `to` naming a reader's kind is what API
    // 0.4 already had, and a `^0.4` manifest keeps writing it.
    let mut value = manifest();
    value["contributions"]["resourceLinks"] = json!([{"id":"owner","from":"apps/Deployment",
        "to":"argoproj.io/Application","relation":"managedBy",
        "match":{"label":"argocd.argoproj.io/instance"}}]);
    Manifest::parse(&with_range(value, "^0.4")).expect("a reader target is 0.4");
}

#[test]
fn a_0_5_manifest_is_incompatible_with_a_host_on_the_0_4_line() {
    // What the published 0.4 hosts (srelens 0.15.1-186 and -187) check first.
    let published = ["0.3.0", "0.4.0"];
    for range in ["^0.5", ">=0.5, <0.6"] {
        let range = semver::VersionReq::parse(range).unwrap();
        assert!(matching_api_versions_in(&range, &published).is_empty());
        assert_eq!(
            negotiate_api_version(&range).map(|v| v.to_string()),
            Some("0.5.0".into())
        );
    }
}

#[test]
fn a_0_4_manifest_keeps_its_line_on_a_host_that_also_implements_0_5() {
    // `^0.4` pins its minor, so a 0.4 app is served under 0.4 here and keeps installing
    // on a host that implements only 0.4 (#567).
    for range in ["^0.4", ">=0.4, <0.5"] {
        let range = semver::VersionReq::parse(range).unwrap();
        assert_eq!(
            negotiate_api_version(&range).map(|v| v.to_string()),
            Some("0.4.0".into())
        );
    }
    let range = semver::VersionReq::parse("^0.5").unwrap();
    assert_eq!(
        negotiate_api_version(&range).map(|v| v.to_string()),
        Some("0.5.0".into())
    );
    assert!(matching_api_versions_in(&range, &["0.3.0", "0.4.0"]).is_empty());
}
