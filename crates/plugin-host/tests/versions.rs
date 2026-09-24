//! A custom-resource reader that accepts several served API versions (#547).
//!
//! The binding lists `versions` in preference order; the host reads the first one the
//! cluster serves and, where a field moved, reads it through that version's
//! `jsonPathOverrides`. Everything that reads the binding's objects is rewritten for the
//! version resolved, and nothing is ever read at a version the binding does not list.
use serde_json::{json, Value};
use srelens_plugin_host::{
    DetailSection, Manifest, ValidationCode, ValidationError, MAX_BINDING_VERSIONS,
};

const MOVED: &str = ".status.lastAttemptedRevision";
const THERE: &str = ".status.lastReleaseRevision";

/// A HelmRelease reader serving `v2` or `v2beta2`, where `v2beta2` keeps the revision
/// elsewhere, and one of everything that reads it.
fn manifest() -> Value {
    json!({
        "id":"org.example.flux", "name":"Flux", "version":"0.1.0", "srelensApiVersion":"^0.3",
        "kind":"declarative", "permissions":["k8s.listCustomResource","k8s.setFields"],
        "capabilities":[
            {"name":"helmreleases","title":"List Helm releases","target":"k8s.listCustomResource",
             "versions":["v2","v2beta2"],
             "jsonPathOverrides":{"v2beta2":{MOVED: THERE}},
             "arguments":{"group":"helm.toolkit.fluxcd.io","plural":"helmreleases","kind":"HelmRelease",
                "namespaced":true,
                "printerColumns":[{"name":"Revision","jsonPath":MOVED,"type":"string"},
                                  {"name":"Suspended","jsonPath":".spec.suspend","type":"boolean"}]},
             "inputs":["context","namespace"]},
            {"name":"kustomizations","title":"List Kustomizations","target":"k8s.listCustomResource",
             "arguments":{"group":"kustomize.toolkit.fluxcd.io","version":"v1","plural":"kustomizations",
                "kind":"Kustomization","namespaced":true,
                "printerColumns":[{"name":"Revision","jsonPath":MOVED,"type":"string"}]},
             "inputs":["context","namespace"]}
        ],
        "actions":[{"name":"suspend","title":"Suspend","target":"k8s.setFields","resource":"helmreleases",
            "arguments":{"fields":{"/spec/suspend":true}},
            "preconditions":[{"jsonPath":MOVED,"present":true,"reason":"Not yet released"}],
            "availableWhen":[{"jsonPath":MOVED,"present":true,"reason":"Not yet released"}]}],
        "contributions":{
            "pages":[{"id":"helmreleases","title":"Helm releases","capability":"helmreleases"}],
            "detailTabs":[],"detailLinks":[],
            "joins":[{"id":"release","capability":"helmreleases","match":{"label":"helm.toolkit.fluxcd.io/name"}}],
            "tableColumns":[{"id":"revision","title":"Revision","forKinds":["apps/Deployment"],
                "source":{"join":"release","jsonPath":MOVED},"format":"text"}],
            "badges":[{"id":"helm","forKinds":["apps/Deployment"],"join":"release",
                "rules":[{"when":[{"jsonPath":MOVED,"present":true}],"status":"healthy","label":"Helm","reason":MOVED}]}],
            "detailPanels":[{"id":"release","title":"Release","forKinds":["helm.toolkit.fluxcd.io/HelmRelease"],
                "sections":[{"type":"fields","fields":[{"label":"Revision","jsonPath":MOVED}]}]}],
            "statusResolvers":[
                {"forKinds":["helm.toolkit.fluxcd.io/HelmRelease"],
                 "rules":[{"when":[{"jsonPath":MOVED,"present":true}],"status":"healthy","label":"Released","reason":MOVED},
                          {"when":[],"status":"unknown","label":"Unknown"}]},
                {"forKinds":["kustomize.toolkit.fluxcd.io/Kustomization"],
                 "rules":[{"when":[{"jsonPath":MOVED,"present":true}],"status":"healthy","label":"Applied"}]}
            ],
            "dashboardCards":[{"id":"released","title":"Released","size":"s","type":"list","source":"helmreleases",
                "predicate":{"jsonPath":MOVED,"absent":true},
                "list":{"jsonPath":MOVED}}]
        }
    })
}

fn parse(value: &Value) -> Manifest {
    Manifest::parse(&value.to_string()).unwrap_or_else(|errors| panic!("{errors}"))
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

/// Codes and paths, sorted so the assertion does not depend on check order.
fn problems(value: &Value) -> Vec<(String, String)> {
    let mut problems: Vec<_> = errors(value)
        .iter()
        .map(|error| (code(error.code), error.path.clone()))
        .collect();
    problems.sort();
    problems
}

fn one(code: &str, path: &str) -> Vec<(String, String)> {
    vec![(code.to_owned(), path.to_owned())]
}

/// Every JSONPath the manifest reads a HelmRelease through, as written.
fn helm_release_paths(manifest: &Manifest) -> Vec<String> {
    let binding = &manifest.capabilities[0];
    let contributions = &manifest.contributions;
    let action = &manifest.actions[0];
    let card = &contributions.dashboard_cards[0];
    let DetailSection::Fields { fields } = &contributions.detail_panels[0].sections[0] else {
        panic!("fields section")
    };
    let resolver = &contributions.status_resolvers[0].rules[0];
    let badge = &contributions.badges[0].rules[0];
    vec![
        binding.arguments["printerColumns"][0]["jsonPath"]
            .as_str()
            .unwrap()
            .to_owned(),
        action.preconditions[0].json_path.clone(),
        action.available_when[0].json_path.clone(),
        contributions.table_columns[0].source.json_path.clone(),
        badge.when[0].json_path.clone(),
        badge.reason.clone().unwrap(),
        fields[0].json_path.clone(),
        resolver.when[0].json_path.clone(),
        resolver.reason.clone().unwrap(),
        card.predicate.as_ref().unwrap().json_path.clone(),
        card.list.as_ref().unwrap().json_path.clone().unwrap(),
    ]
}

#[test]
fn a_binding_accepts_its_versions_in_preference_order() {
    let manifest = parse(&manifest());
    assert_eq!(
        manifest.accepted_versions("helmreleases"),
        ["v2".to_owned(), "v2beta2".to_owned()]
    );
    // A binding that fixes one version accepts exactly it.
    assert_eq!(
        manifest.accepted_versions("kustomizations"),
        ["v1".to_owned()]
    );
    assert!(manifest.accepted_versions("undeclared").is_empty());
}

#[test]
fn at_a_version_the_binding_reads_it_and_every_path_through_its_overrides() {
    let manifest = parse(&manifest());
    let older = manifest.at_version("helmreleases", "v2beta2").unwrap();
    let binding = &older.capabilities[0];
    assert_eq!(binding.arguments["version"], "v2beta2");
    assert!(binding.versions.is_empty() && binding.json_path_overrides.is_empty());
    assert_eq!(helm_release_paths(&older), vec![THERE.to_owned(); 11]);
    // A path the override does not name is left as written.
    assert_eq!(
        binding.arguments["printerColumns"][1]["jsonPath"],
        ".spec.suspend"
    );
    // The resolved manifest is an ordinary single-version one, and valid.
    older.validate().unwrap();
    // An action reaches its reader's kind at the resolved version.
    let action = older.action_binding(&older.actions[0]).unwrap();
    assert_eq!(action.arguments["version"], "v2beta2");
    assert_eq!(action.arguments["preconditions"][0]["jsonPath"], THERE);

    // Another reader of a path with the same text is not this binding's to rewrite.
    let other = &older.capabilities[1];
    assert_eq!(other.arguments["printerColumns"][0]["jsonPath"], MOVED);
    assert_eq!(
        older.contributions.status_resolvers[1].rules[0].when[0].json_path,
        MOVED
    );

    // The preferred version has no overrides: every path is read as written.
    let newer = manifest.at_version("helmreleases", "v2").unwrap();
    assert_eq!(newer.capabilities[0].arguments["version"], "v2");
    assert_eq!(helm_release_paths(&newer), vec![MOVED.to_owned(); 11]);
}

#[test]
fn a_version_the_binding_does_not_list_is_refused_never_read() {
    let manifest = parse(&manifest());
    let refused = manifest.at_version("helmreleases", "v2beta1").unwrap_err();
    assert!(refused.contains("v2, v2beta2"), "{refused}");
    // A single-version binding is at its own version already, and at no other.
    assert!(manifest.at_version("kustomizations", "v1").is_ok());
    assert!(manifest.at_version("kustomizations", "v1beta2").is_err());
    // Resolving twice at the same version is the same manifest; at another, refused.
    let resolved = manifest.at_version("helmreleases", "v2beta2").unwrap();
    assert!(resolved.at_version("helmreleases", "v2beta2").is_ok());
    assert!(resolved.at_version("helmreleases", "v2").is_err());
    assert!(manifest.at_version("undeclared", "v1").is_err());
}

#[test]
fn version_and_versions_are_one_choice() {
    let mut value = manifest();
    value["capabilities"][0]["arguments"]["version"] = json!("v2");
    assert_eq!(
        problems(&value),
        one("EXTENSION_INVALID_BINDING", "capabilities[0].versions")
    );
}

#[test]
fn only_a_custom_resource_reader_lists_versions() {
    let mut value = manifest();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.listEvents", "k8s.setFields"]);
    value["capabilities"].as_array_mut().unwrap().push(
        json!({"name":"events","title":"Events","target":"k8s.listEvents",
            "versions":["v1"],"arguments":{},"inputs":["context","namespace"]}),
    );
    assert_eq!(
        problems(&value),
        one("EXTENSION_INVALID_BINDING", "capabilities[2].versions")
    );
}

#[test]
fn listed_versions_are_distinct_bounded_names() {
    let mut value = manifest();
    value["capabilities"][0]["versions"] = json!(["v2", "v2beta2", "v2"]);
    assert_eq!(
        problems(&value),
        one(
            "EXTENSION_DUPLICATE_IDENTIFIER",
            "capabilities[0].versions[2]"
        )
    );
    let mut value = manifest();
    value["capabilities"][0]["versions"] = json!(["v2", "v2beta2", "../v1"]);
    assert_eq!(
        problems(&value),
        one("EXTENSION_INVALID_BINDING", "capabilities[0].versions[2]")
    );
    let mut value = manifest();
    let many: Vec<String> = (1..=9)
        .map(|n| format!("v{n}"))
        .chain(["v2beta2".into()])
        .collect();
    value["capabilities"][0]["versions"] = json!(many);
    assert_eq!(
        problems(&value),
        one("EXTENSION_INVALID_VALUE", "capabilities[0].versions")
    );
}

/// Every problem, as sorted (code, path) pairs: how many there are is how much of the
/// manifest validation went through.
fn problem_list(value: &Value) -> Vec<(String, String)> {
    problems(value)
}

#[test]
fn an_oversized_override_map_is_refused_whole_without_checking_each_entry() {
    // CWE-400 (PR #692 review): 2,000 override entries, none a listed version.
    let mut value = manifest();
    let overrides: serde_json::Map<String, Value> = (0..2_000)
        .map(|n| (format!("x{n}"), json!({MOVED: THERE})))
        .collect();
    value["capabilities"][0]["jsonPathOverrides"] = Value::Object(overrides);
    let found = errors(&value);
    // One bounded-input refusal, not one problem per entry.
    assert_eq!(found.len(), 1, "{:?}", &found[..found.len().min(3)]);
    assert_eq!(code(found[0].code), "EXTENSION_INVALID_VALUE");
    assert_eq!(found[0].path, "capabilities[0].jsonPathOverrides");
    assert!(
        found[0]
            .message
            .contains(&format!("at most {MAX_BINDING_VERSIONS} versions")),
        "{}",
        found[0].message
    );
}

#[test]
fn too_many_versions_stop_the_override_checks_as_well() {
    // The review's shape: thousands of listed versions, each with an override entry,
    // within the 256 KiB manifest limit. Each entry used to cost a manifest clone, a
    // walk and two full validations.
    let mut value = manifest();
    let versions: Vec<String> = (0..1_000).map(|n| format!("v{n}")).collect();
    let overrides: serde_json::Map<String, Value> = versions
        .iter()
        .map(|version| (version.clone(), json!({".status.unread": THERE})))
        .collect();
    value["capabilities"][0]["versions"] = json!(versions);
    value["capabilities"][0]["jsonPathOverrides"] = Value::Object(overrides);
    assert_eq!(
        problem_list(&value),
        [
            (
                "EXTENSION_INVALID_VALUE".to_owned(),
                "capabilities[0].jsonPathOverrides".to_owned()
            ),
            (
                "EXTENSION_INVALID_VALUE".to_owned(),
                "capabilities[0].versions".to_owned()
            ),
        ]
    );
    // Within the version limit, the same unread path is still reported per entry.
    let mut value = manifest();
    value["capabilities"][0]["jsonPathOverrides"]["v2beta2"][".status.unread"] = json!(THERE);
    assert_eq!(
        problem_list(&value),
        one(
            "EXTENSION_INVALID_BINDING",
            "capabilities[0].jsonPathOverrides.v2beta2"
        )
    );
}

#[test]
fn past_the_binding_limit_no_override_is_checked() {
    // Validation goes on past 32 capabilities to report everything else; the override
    // checks, whose cost grows with every binding, do not.
    let mut value = manifest();
    let reader = value["capabilities"][0].clone();
    let capabilities = value["capabilities"].as_array_mut().unwrap();
    for n in 0..40 {
        let mut copy = reader.clone();
        copy["name"] = json!(format!("copy{n}"));
        copy["jsonPathOverrides"] = json!({"v2beta2": {".status.unread": THERE}});
        capabilities.push(copy);
    }
    let found = problem_list(&value);
    assert!(
        found.contains(&(
            "EXTENSION_INVALID_VALUE".to_owned(),
            "capabilities".to_owned()
        )),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .all(|(_, path)| !path.contains("jsonPathOverrides")),
        "{found:?}"
    );
}

#[test]
fn an_override_names_a_listed_version() {
    let mut value = manifest();
    value["capabilities"][0]["jsonPathOverrides"] = json!({"v1": {MOVED: THERE}});
    assert_eq!(
        problems(&value),
        one(
            "EXTENSION_INVALID_BINDING",
            "capabilities[0].jsonPathOverrides.v1"
        )
    );
    // Without `versions` there is no version to override.
    let mut value = manifest();
    value["capabilities"][1]["jsonPathOverrides"] = json!({"v1": {MOVED: THERE}});
    assert_eq!(
        problems(&value),
        one(
            "EXTENSION_INVALID_BINDING",
            "capabilities[1].jsonPathOverrides.v1"
        )
    );
}

#[test]
fn an_override_replaces_only_a_path_the_binding_reads() {
    let mut value = manifest();
    value["capabilities"][0]["jsonPathOverrides"]["v2beta2"][".status.unread"] = json!(THERE);
    let errors = errors(&value);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].path, "capabilities[0].jsonPathOverrides.v2beta2");
    assert!(
        errors[0].message.contains(".status.unread"),
        "{}",
        errors[0].message
    );
}

#[test]
fn an_override_is_a_valid_path_wherever_it_is_read() {
    // Valid as a column path, but a status rule's condition has no wildcard.
    let mut value = manifest();
    value["capabilities"][0]["jsonPathOverrides"]["v2beta2"][MOVED] =
        json!(".status.history[*].revision");
    let errors = errors(&value);
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .all(|e| e.path == "capabilities[0].jsonPathOverrides.v2beta2"),
        "{errors:?}"
    );
    let mut value = manifest();
    value["capabilities"][0]["jsonPathOverrides"]["v2beta2"][MOVED] = json!("status");
    assert!(errors_at(
        &value,
        "capabilities[0].jsonPathOverrides.v2beta2"
    ));
}

#[test]
fn each_bad_override_is_reported_at_its_own_binding_and_version() {
    // Two readers that list versions; a bad override on each, and a good one.
    let mut value = manifest();
    let kustomizations = &mut value["capabilities"][1];
    kustomizations["arguments"]
        .as_object_mut()
        .unwrap()
        .remove("version");
    kustomizations["versions"] = json!(["v1", "v1beta2"]);
    // Read by the Kustomization status resolver's condition, which has no wildcard.
    kustomizations["jsonPathOverrides"] = json!({"v1beta2": {MOVED: ".status.history[*].x"}});
    // v2 (bad) is checked beside Kustomization's v1beta2; v2beta2 (good) after.
    value["capabilities"][0]["jsonPathOverrides"]["v2"] = json!({MOVED: ".status.list[*].x"});
    let found = errors(&value);
    let mut paths: Vec<_> = found.iter().map(|e| e.path.as_str()).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(
        paths,
        [
            "capabilities[0].jsonPathOverrides.v2",
            "capabilities[1].jsonPathOverrides.v1beta2"
        ],
        "{found:?}"
    );
    // Each names the declaration it broke there.
    let helm: Vec<_> = found
        .iter()
        .filter(|e| e.path.starts_with("capabilities[0]"))
        .collect();
    assert!(
        helm.iter()
            .any(|e| e.message.contains("contributions.statusResolvers[0]")),
        "{helm:?}"
    );
    assert!(
        helm.iter()
            .all(|e| !e.message.contains("contributions.statusResolvers[1]")),
        "{helm:?}"
    );
    let kustomization: Vec<_> = found
        .iter()
        .filter(|e| e.path.starts_with("capabilities[1]"))
        .collect();
    assert!(
        kustomization
            .iter()
            .all(|e| e.message.contains("contributions.statusResolvers[1]")),
        "{kustomization:?}"
    );
}

fn errors_at(value: &Value, path: &str) -> bool {
    let errors = errors(value);
    !errors.is_empty() && errors.iter().all(|e| e.path == path)
}

#[test]
fn an_override_cannot_rewrite_a_path_another_kind_reads_too() {
    // One resolver for both kinds: rewriting it for HelmRelease would rewrite it for
    // Kustomization as well.
    let mut value = manifest();
    let resolvers = value["contributions"]["statusResolvers"]
        .as_array_mut()
        .unwrap();
    resolvers.remove(1);
    resolvers[0]["forKinds"] = json!([
        "helm.toolkit.fluxcd.io/HelmRelease",
        "kustomize.toolkit.fluxcd.io/Kustomization"
    ]);
    let errors = errors(&value);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].path, "capabilities[0].jsonPathOverrides.v2beta2");
    assert!(
        errors[0].message.contains("statusResolvers[0]"),
        "{}",
        errors[0].message
    );
    // A shared resolver that the override does not touch is fine.
    value["capabilities"][0]["jsonPathOverrides"] =
        json!({"v2beta2": {".spec.suspend": ".spec.paused"}});
    parse(&value);
}

#[test]
fn the_manifest_round_trips_without_the_new_fields_when_unused() {
    let manifest = parse(&manifest());
    let single = serde_json::to_value(&manifest.capabilities[1]).unwrap();
    assert!(single.get("versions").is_none() && single.get("jsonPathOverrides").is_none());
    let multi = serde_json::to_value(&manifest.capabilities[0]).unwrap();
    assert_eq!(multi["versions"], json!(["v2", "v2beta2"]));
    assert_eq!(
        multi["jsonPathOverrides"],
        json!({"v2beta2": {MOVED: THERE}})
    );
}

#[test]
fn an_action_is_bound_only_once_its_reader_is_at_a_version() {
    let manifest = parse(&manifest());
    let refused = manifest.action_binding(&manifest.actions[0]).unwrap_err();
    assert!(refused.contains("v2, v2beta2"), "{refused}");
    for version in ["v2", "v2beta2"] {
        let resolved = manifest.at_version("helmreleases", version).unwrap();
        let bound = resolved.action_binding(&resolved.actions[0]).unwrap();
        assert_eq!(bound.arguments["version"], version);
    }
}

#[test]
fn the_flux_example_reads_releases_and_oci_sources_of_older_flux_too() {
    let flux = Manifest::parse(include_str!("../../../examples/extensions/flux.json")).unwrap();
    assert_eq!(flux.accepted_versions("helmreleases"), ["v2", "v2beta2"]);
    assert_eq!(flux.accepted_versions("ocirepositories"), ["v1", "v1beta2"]);
    for binding in ["helmreleases", "ocirepositories"] {
        for version in flux.accepted_versions(binding) {
            flux.at_version(binding, &version)
                .unwrap()
                .validate()
                .unwrap();
        }
    }
}

/// The real host registry, with the reader and the action primitive answering with
/// the arguments they were called with instead of calling a cluster.
fn echoing_core() -> std::sync::Arc<srelens_capability::Registry> {
    let mut core = srelens_registry::build_registry();
    for id in ["k8s.listCustomResource", "k8s.setFields", "k8s.annotate"] {
        let mut capability = core.get(id).unwrap().clone();
        capability.handler = std::sync::Arc::new(|args| Box::pin(async move { Ok(args) }));
        core.register(capability);
    }
    std::sync::Arc::new(core)
}

#[test]
fn the_host_checks_a_multi_version_reader_and_its_action_against_their_targets() {
    let manifest = parse(&manifest());
    let host = srelens_plugin_host::PluginHost::new(echoing_core());
    // `version` is chosen per cluster, so a reader that lists versions leaves it unbound.
    assert_eq!(
        host.binding_problems(0, &manifest, &manifest.capabilities[0]),
        vec![]
    );
    assert_eq!(
        host.action_problems(0, &manifest, &manifest.actions[0]),
        vec![]
    );
    // Neither a version nor a list of them is still a reader without a version.
    let mut unversioned = manifest.capabilities[0].clone();
    unversioned.versions.clear();
    unversioned.json_path_overrides.clear();
    let problems = host.binding_problems(0, &manifest, &unversioned);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].path, "capabilities[0].arguments.version");
}

#[tokio::test]
async fn a_reader_is_callable_only_at_a_resolved_version_and_its_action_goes_with_it() {
    let manifest = parse(&manifest());
    let grants = manifest.permissions.clone();
    let host = srelens_plugin_host::PluginHost::new(echoing_core());

    // Unresolved, nothing reads HelmReleases or acts on them: no version was chosen.
    let mut reg = srelens_capability::Registry::new();
    host.register(&mut reg, manifest.clone(), &grants).unwrap();
    assert!(reg.get("plugin/org.example.flux/helmreleases").is_none());
    assert!(reg.get("plugin/org.example.flux/suspend").is_none());
    assert!(reg.get("plugin/org.example.flux/kustomizations").is_some());

    let resolved = manifest.at_version("helmreleases", "v2beta2").unwrap();
    let mut reg = srelens_capability::Registry::new();
    host.register(&mut reg, resolved, &grants).unwrap();
    let read = reg
        .invoke(
            "plugin/org.example.flux/helmreleases",
            json!({"context":"c","namespace":"n"}),
        )
        .await
        .unwrap();
    assert_eq!(read["version"], "v2beta2");
    assert_eq!(read["printerColumns"][0]["jsonPath"], THERE);
    let acted = reg
        .invoke(
            "plugin/org.example.flux/suspend",
            json!({"context":"c","namespace":"n","name":"web","uid":"u","resourceVersion":"7"}),
        )
        .await
        .unwrap();
    assert_eq!(acted["version"], "v2beta2");
    assert_eq!(acted["plural"], "helmreleases");
    assert_eq!(acted["preconditions"][0]["jsonPath"], THERE);
    assert_eq!(
        (acted["uid"].clone(), acted["resourceVersion"].clone()),
        (json!("u"), json!("7"))
    );
}

/// The example with a setting (#542) that an action on the multi-version reader
/// interpolates into the annotation it writes.
fn with_setting() -> Value {
    let mut value = manifest();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.setFields", "k8s.annotate"]);
    value["settings"] = json!([{"id":"mode","type":"string","title":"Mode","default":"normal"}]);
    value["actions"].as_array_mut().unwrap().push(json!({
        "name":"refresh","title":"Refresh","target":"k8s.annotate","resource":"helmreleases",
        "arguments":{"key":"example.io/refresh","value":"${settings.mode}"}
    }));
    value
}

fn saved(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn a_saved_setting_is_checked_by_an_action_on_a_multi_version_reader() {
    let manifest = parse(&with_setting());
    let host = srelens_plugin_host::PluginHost::new(echoing_core());
    // The reader is checked as bound at a version, as at install: not refused for
    // lacking the `version` a cluster chooses.
    let found = host.settings_problems(&manifest, &saved(json!({"mode":"hard"})));
    assert_eq!(found, vec![]);
    // A value the action's primitive refuses is refused on save, at the action,
    // even though the action binds only once a cluster resolves its reader.
    let found = host.settings_problems(&manifest, &saved(json!({"mode":"$abc123"})));
    let paths: Vec<_> = found.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["actions[1].arguments"], "{found:?}");
}

#[tokio::test]
async fn a_request_interpolates_the_setting_into_the_resolved_version_binding() {
    let manifest = parse(&with_setting());
    let host = srelens_plugin_host::PluginHost::new(echoing_core());
    let resolved = manifest.at_version("helmreleases", "v2beta2").unwrap();
    let mut reg = srelens_capability::Registry::new();
    host.register_with_settings(
        &mut reg,
        resolved,
        &manifest.permissions,
        &saved(json!({"mode":"hard"})),
    )
    .unwrap();
    let sent = reg
        .invoke(
            "plugin/org.example.flux/refresh",
            json!({"context":"c","namespace":"n","name":"web","uid":"u","resourceVersion":"7"}),
        )
        .await
        .unwrap();
    assert_eq!(sent["version"], "v2beta2");
    assert_eq!(sent["value"], "hard");
    assert_eq!(sent["key"], "example.io/refresh");
}
