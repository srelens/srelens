//! Command palette contributions (#544): what an app may put in the palette,
//! and every way a declaration can fail, each reported at the field that has
//! to change.
use serde_json::{json, Value};
use srelens_plugin_host::{CommandTarget, Manifest, ValidationCode, ValidationError, MAX_COMMANDS};

/// One custom-resource reader with a page, and one declared action on it.
fn manifest() -> Value {
    json!({
        "id":"org.example.gitops", "name":"GitOps", "version":"0.1.0", "srelensApiVersion":"^0.3",
        "kind":"declarative", "permissions":["k8s.annotate","k8s.listCustomResource"],
        "capabilities":[
            {"name":"applications","title":"List applications", "target":"k8s.listCustomResource",
             "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"applications","kind":"Application","namespaced":true},
             "inputs":["context","namespace"]},
            {"name":"projects","title":"List projects", "target":"k8s.listCustomResource",
             "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"appprojects","kind":"AppProject","namespaced":true},
             "inputs":["context","namespace"]}
        ],
        "actions":[
            {"name":"refresh","title":"Refresh","target":"k8s.annotate","resource":"applications",
             "arguments":{"annotations":{"argocd.argoproj.io/refresh":"normal"}}},
            {"name":"refresh-project","title":"Refresh project","target":"k8s.annotate","resource":"projects",
             "arguments":{"annotations":{"example.io/refresh":"now"}}}
        ],
        "contributions":{
            "pages":[{"id":"applications","title":"Applications","capability":"applications"}],
            "detailTabs":[],"detailLinks":[],
            "commands":[
                {"id":"open-applications","title":"Open applications","target":{"page":"applications"}},
                {"id":"refresh","title":"Refresh application","target":{"action":"refresh"},
                 "forKinds":["argoproj.io/Application"]}
            ]
        }
    })
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
fn problems(errors: &[ValidationError]) -> Vec<(String, String)> {
    let mut problems: Vec<_> = errors
        .iter()
        .map(|error| (code(error.code), error.path.clone()))
        .collect();
    problems.sort();
    problems
}

fn expected(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = pairs
        .iter()
        .map(|(code, path)| (code.to_string(), path.to_string()))
        .collect();
    pairs.sort();
    pairs
}

#[test]
fn the_issues_example_parses_and_round_trips_its_own_spelling() {
    let parsed = Manifest::parse(&manifest().to_string()).expect("the #544 example is valid");
    let commands = &parsed.contributions.commands;
    assert_eq!(commands.len(), 2);
    assert!(matches!(&commands[0].target, CommandTarget::Page(page) if page == "applications"));
    assert!(commands[0].for_kinds.is_empty());
    assert!(matches!(&commands[1].target, CommandTarget::Action(action) if action == "refresh"));
    // The wire spelling is the manifest author's, not the struct's.
    let serialized = serde_json::to_value(&parsed).unwrap();
    assert_eq!(
        serialized["contributions"]["commands"][0]["target"],
        json!({"page":"applications"})
    );
    assert_eq!(
        serialized["contributions"]["commands"][1],
        json!({"id":"refresh","title":"Refresh application","target":{"action":"refresh"},
               "forKinds":["argoproj.io/Application"]})
    );
    assert!(serialized["contributions"]["commands"][0]
        .get("forKinds")
        .is_none());
}

#[test]
fn a_manifest_without_commands_stores_without_the_field() {
    let mut value = manifest();
    value["contributions"]
        .as_object_mut()
        .unwrap()
        .remove("commands");
    let parsed = Manifest::parse(&value.to_string()).unwrap();
    let serialized = serde_json::to_value(&parsed).unwrap();
    assert!(serialized["contributions"].get("commands").is_none());
}

#[test]
fn targets_must_resolve_to_a_declared_page_or_action() {
    let mut value = manifest();
    value["contributions"]["commands"][0]["target"] = json!({"page":"missing"});
    value["contributions"]["commands"][1]["target"] = json!({"action":"missing"});
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            (
                "EXTENSION_UNRESOLVED_PAGE",
                "contributions.commands[0].target.page"
            ),
            (
                "EXTENSION_UNRESOLVED_CAPABILITY",
                "contributions.commands[1].target.action"
            ),
        ])
    );
}

#[test]
fn a_reader_is_not_an_action_a_command_can_run() {
    // `applications` is a capability name, but not a declared action: a
    // command runs a mutation the manifest declared, nothing else.
    let mut value = manifest();
    value["contributions"]["commands"][1]["target"] = json!({"action":"applications"});
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_UNRESOLVED_CAPABILITY",
            "contributions.commands[1].target.action"
        )])
    );
}

#[test]
fn a_target_names_exactly_one_page_or_action() {
    for target in [
        json!({"page":"applications","action":"refresh"}),
        json!({}),
        json!({"route":"/"}),
        json!("applications"),
    ] {
        let mut value = manifest();
        value["contributions"]["commands"][0]["target"] = target.clone();
        let found = errors(&value);
        assert_eq!(found.len(), 1, "{target}: {found:?}");
        assert!(
            found[0]
                .path
                .starts_with("contributions.commands[0].target"),
            "{target}: {found:?}"
        );
    }
}

#[test]
fn ids_titles_and_counts_follow_the_sibling_contribution_rules() {
    let mut value = manifest();
    let commands = &mut value["contributions"]["commands"];
    commands[1]["id"] = json!("open-applications");
    commands[0]["title"] = json!("Open\u{202e}applications");
    commands
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"bad id","title":" ","target":{"page":"applications"}}));
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            ("EXTENSION_INVALID_VALUE", "contributions.commands[0].title"),
            (
                "EXTENSION_DUPLICATE_IDENTIFIER",
                "contributions.commands[1].id"
            ),
            ("EXTENSION_INVALID_VALUE", "contributions.commands[2].id"),
            ("EXTENSION_INVALID_VALUE", "contributions.commands[2].title"),
        ])
    );

    let mut value = manifest();
    let many: Vec<_> = (0..=MAX_COMMANDS)
        .map(|i| json!({"id":format!("open-{i}"),"title":"Open","target":{"page":"applications"}}))
        .collect();
    value["contributions"]["commands"] = json!(many);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[("EXTENSION_INVALID_VALUE", "contributions.commands")])
    );
}

#[test]
fn an_action_command_names_the_kinds_its_action_acts_on() {
    // Missing, malformed and repeated entries are the `forKinds` rules every
    // sibling contribution follows.
    let mut value = manifest();
    value["contributions"]["commands"][1]["forKinds"] = json!([]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_VALUE",
            "contributions.commands[1].forKinds"
        )])
    );
    let mut value = manifest();
    value["contributions"]["commands"][1]
        .as_object_mut()
        .unwrap()
        .remove("forKinds");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_VALUE",
            "contributions.commands[1].forKinds"
        )])
    );
    let mut value = manifest();
    value["contributions"]["commands"][1]["forKinds"] = json!([
        "Application",
        "argoproj.io/Application",
        "argoproj.io/Application"
    ]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            (
                "EXTENSION_INVALID_KIND",
                "contributions.commands[1].forKinds[0]"
            ),
            (
                "EXTENSION_DUPLICATE_IDENTIFIER",
                "contributions.commands[1].forKinds[2]"
            ),
        ])
    );
    // A well-formed kind the action does not act on would put the command
    // on resources it can never run against.
    let mut value = manifest();
    value["contributions"]["commands"][1]["forKinds"] =
        json!(["argoproj.io/Application", "argoproj.io/AppProject"]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_BINDING",
            "contributions.commands[1].forKinds[1]"
        )])
    );
}

#[test]
fn a_page_command_has_no_kinds_to_be_scoped_to() {
    let mut value = manifest();
    value["contributions"]["commands"][0]["forKinds"] = json!(["argoproj.io/Application"]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_BINDING",
            "contributions.commands[0].forKinds"
        )])
    );
}

#[test]
fn an_action_command_needs_a_page_that_shows_its_resource() {
    // The palette offers an action on the resource the reader has open, and
    // an app's resource is only ever open through one of its pages. With no
    // page listing `projects`, the command could never be offered.
    let mut value = manifest();
    value["contributions"]["commands"][1] = json!({
        "id":"refresh-project","title":"Refresh project","target":{"action":"refresh-project"},
        "forKinds":["argoproj.io/AppProject"]
    });
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_BINDING",
            "contributions.commands[1].target.action"
        )])
    );
}

#[test]
fn the_gitops_examples_reach_their_pages_and_reconciles_from_the_palette() {
    let flux = Manifest::parse(include_str!("../../../examples/extensions/flux.json")).unwrap();
    let argo = Manifest::parse(include_str!("../../../examples/extensions/argocd.json")).unwrap();
    let has = |manifest: &Manifest, target: &str, kind: Option<&str>| {
        manifest.contributions.commands.iter().any(|command| {
            let named = match &command.target {
                CommandTarget::Page(page) | CommandTarget::Action(page) => page,
            };
            named == target
                && kind.map_or(command.for_kinds.is_empty(), |kind| {
                    command.for_kinds == [kind.to_owned()]
                })
        })
    };
    assert!(has(&flux, "helmreleases", None));
    assert!(has(&flux, "kustomizations", None));
    assert!(has(
        &flux,
        "helmreleases-reconcile",
        Some("helm.toolkit.fluxcd.io/HelmRelease")
    ));
    assert!(has(
        &flux,
        "kustomizations-reconcile",
        Some("kustomize.toolkit.fluxcd.io/Kustomization")
    ));
    assert!(has(&argo, "applications", None));
    assert!(has(&argo, "sync", Some("argoproj.io/Application")));
}

#[test]
fn an_action_on_a_built_in_reader_is_not_a_palette_command() {
    // The palette's confirmation is the app resource inspector's, which reads
    // custom resources only; a workload restart already has its own surfaces.
    let mut value = manifest();
    value["permissions"] = json!([
        "k8s.annotate",
        "k8s.listCustomResource",
        "k8s.listDeployments",
        "k8s.requestRolloutRestart"
    ]);
    value["capabilities"].as_array_mut().unwrap().push(json!({
        "name":"deployments","title":"List deployments","target":"k8s.listDeployments",
        "arguments":{},"inputs":["context","namespace"]
    }));
    value["actions"].as_array_mut().unwrap().push(json!({
        "name":"restart","title":"Restart","target":"k8s.requestRolloutRestart","resource":"deployments","arguments":{}
    }));
    value["contributions"]["pages"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"deployments","title":"Deployments","capability":"deployments"
        }));
    value["contributions"]["commands"][1] = json!({
        "id":"restart","title":"Restart","target":{"action":"restart"},"forKinds":["apps/Deployment"]
    });
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_BINDING",
            "contributions.commands[1].target.action"
        )])
    );
}
