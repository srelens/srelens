//! Logs, exec and port-forwards for apps (#567): the pod bindings a manifest declares,
//! the scope each is held to, and the namespaces a permission may grant.
//!
//! A pod binding reaches only pods an object of a kind the app reads selects — its
//! own label selector — or pods in namespaces its permission grants. An exec binding
//! runs one command fixed in the manifest; a port-forward names one remote port.
use serde_json::{json, Value};
use srelens_plugin_host::{
    Manifest, PodScope, ValidationCode, ValidationError, POD_EXEC, POD_FORWARD, POD_LOGS,
};

/// A cert-manager app: it reads the controller Deployments and Argo Rollouts, streams
/// the controller's logs, runs `cmctl status` in it, and forwards its metrics port. One
/// more logs binding reaches any pod in the namespace its permission grants.
fn manifest() -> Value {
    json!({
        "id":"org.example.certmanager", "name":"cert-manager", "version":"0.1.0",
        "srelensApiVersion":"^0.5", "kind":"declarative",
        "permissions":["k8s.listDeployments", "k8s.listCustomResource",
            {"capability":"k8s.streamLogs","namespaces":["cert-manager"]},
            "k8s.exec", "k8s.portForward"],
        "capabilities":[
            {"name":"controllers","title":"Controllers","target":"k8s.listDeployments",
                "inputs":["context","namespace"],"arguments":{}},
            {"name":"rollouts","title":"Rollouts","target":"k8s.listCustomResource",
                "inputs":["context","namespace"],
                "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"rollouts",
                    "kind":"Rollout","namespaced":true}},
            {"name":"controllerLogs","title":"Controller logs","target":"k8s.streamLogs",
                "inputs":[],"arguments":{"resource":"controllers"}},
            {"name":"rolloutLogs","title":"Rollout logs","target":"k8s.streamLogs",
                "inputs":[],"arguments":{"resource":"rollouts","selector":".spec.selector"}},
            {"name":"namespaceLogs","title":"Logs in cert-manager","target":"k8s.streamLogs",
                "inputs":[],"arguments":{}},
            {"name":"status","title":"cmctl status","target":"k8s.exec","inputs":[],
                "arguments":{"resource":"controllers","container":"cert-manager-controller",
                    "command":["cmctl","status","certificate","--all-namespaces"]}},
            {"name":"metrics","title":"Controller metrics","target":"k8s.portForward","inputs":[],
                "arguments":{"resource":"controllers","port":9402}}
        ],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[]}
    })
}

const LOGS: usize = 2;
const ROLLOUT_LOGS: usize = 3;
const NAMESPACE_LOGS: usize = 4;
const EXEC: usize = 5;
const FORWARD: usize = 6;

fn parse(value: &Value) -> Manifest {
    Manifest::parse(&value.to_string()).unwrap_or_else(|e| panic!("valid manifest: {e}"))
}

fn errors(value: &Value) -> Vec<ValidationError> {
    Manifest::parse(&value.to_string())
        .expect_err("an invalid manifest")
        .0
}

fn code(code: ValidationCode) -> String {
    serde_json::to_value(code)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

/// `(code, path)` of every problem, sorted.
fn problems(value: &Value) -> Vec<(String, String)> {
    let mut problems: Vec<_> = errors(value)
        .iter()
        .map(|error| (code(error.code), error.path.clone()))
        .collect();
    problems.sort();
    problems
}

fn one(value: &Value) -> ValidationError {
    let errors = errors(value);
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors.into_iter().next().unwrap()
}

fn with_arguments(index: usize, arguments: Value) -> Value {
    let mut value = manifest();
    value["capabilities"][index]["arguments"] = arguments;
    value
}

#[test]
fn the_pod_bindings_parse_and_name_their_capabilities() {
    let parsed = parse(&manifest());
    assert_eq!(
        parsed.permission_names(),
        [
            "k8s.listDeployments",
            "k8s.listCustomResource",
            POD_LOGS,
            POD_EXEC,
            POD_FORWARD
        ]
    );
    assert_eq!(parsed.pod_namespaces(POD_LOGS), ["cert-manager"]);
    assert!(parsed.pod_namespaces(POD_EXEC).is_empty());
    // Stored as written, so a signed manifest round-trips to its bytes: a namespace
    // grant carries no empty `hosts`, and a plain entry stays a string.
    let stored = serde_json::to_value(&parsed).unwrap();
    assert_eq!(stored["permissions"], manifest()["permissions"]);
}

#[test]
fn a_binding_names_the_scope_its_pods_are_held_to() {
    let parsed = parse(&manifest());
    let scope = |index: usize| parsed.pod_scope(&parsed.capabilities[index]).unwrap();
    match scope(LOGS) {
        PodScope::Selected { reader, selector } => {
            assert_eq!(reader.name, "controllers");
            // A built-in workload's selector is where Kubernetes keeps it, never where
            // an app says.
            assert_eq!(selector, ".spec.selector");
        }
        other => panic!("{other:?}"),
    }
    match scope(ROLLOUT_LOGS) {
        PodScope::Selected { reader, selector } => {
            assert_eq!(reader.name, "rollouts");
            assert_eq!(selector, ".spec.selector");
        }
        other => panic!("{other:?}"),
    }
    match scope(NAMESPACE_LOGS) {
        PodScope::Namespaces(namespaces) => assert_eq!(namespaces, ["cert-manager"]),
        other => panic!("{other:?}"),
    }
    // A reader is no pod binding, and has no pod scope.
    assert!(parsed.pod_scope(&parsed.capabilities[0]).is_err());
    assert_eq!(
        parsed.exec_command(&parsed.capabilities[EXEC]).unwrap(),
        ["cmctl", "status", "certificate", "--all-namespaces"]
    );
}

#[test]
fn a_resource_scope_names_a_namespaced_reader_that_selects_pods() {
    // Not declared at all.
    let error = one(&with_arguments(LOGS, json!({"resource":"nope"})));
    assert_eq!(
        (code(error.code), error.path.as_str()),
        (
            "EXTENSION_UNRESOLVED_CAPABILITY".into(),
            "capabilities[2].arguments.resource"
        )
    );
    // Declared, but nothing it lists selects pods: a node, an event, another pod binding.
    let mut value = manifest();
    value["permissions"]
        .as_array_mut()
        .unwrap()
        .extend([json!("k8s.listNodes"), json!("k8s.listEvents")]);
    value["capabilities"].as_array_mut().unwrap().extend([
        json!({"name":"nodes","title":"Nodes","target":"k8s.listNodes","inputs":["context"],"arguments":{}}),
        json!({"name":"events","title":"Events","target":"k8s.listEvents","inputs":["context","namespace"],"arguments":{}}),
    ]);
    for resource in ["nodes", "events", "controllerLogs"] {
        let mut value = value.clone();
        value["capabilities"][LOGS]["arguments"] = json!({"resource": resource});
        let error = one(&value);
        assert_eq!(
            (code(error.code), error.path.as_str()),
            (
                "EXTENSION_INVALID_BINDING".into(),
                "capabilities[2].arguments.resource"
            ),
            "{resource}: {error:?}"
        );
        assert!(
            error
                .message
                .contains("Deployment, StatefulSet or DaemonSet"),
            "{error:?}"
        );
    }
    // A cluster-scoped custom resource has no namespace for its pods to be in.
    let mut value = manifest();
    value["capabilities"][1]["arguments"]["namespaced"] = json!(false);
    value["capabilities"][1]["inputs"] = json!(["context"]);
    let error = one(&value);
    assert_eq!(error.path, "capabilities[3].arguments.resource");
    assert!(error.message.contains("namespaced"), "{error:?}");
}

#[test]
fn a_custom_resource_names_its_selector_and_a_built_in_one_cannot() {
    let error = one(&with_arguments(
        ROLLOUT_LOGS,
        json!({"resource":"rollouts"}),
    ));
    assert_eq!(
        (code(error.code), error.path.as_str()),
        (
            "EXTENSION_INVALID_BINDING".into(),
            "capabilities[3].arguments.selector"
        )
    );
    assert!(error.message.contains(".spec.selector"), "{error:?}");
    let error = one(&with_arguments(
        LOGS,
        json!({"resource":"controllers","selector":".spec.template.metadata.labels"}),
    ));
    assert_eq!(error.path, "capabilities[2].arguments.selector");
    assert!(error.message.contains("Deployment"), "{error:?}");
    for bad in [
        "spec.selector",
        ".spec..selector",
        "",
        ".spec[?(@.x==\"y\")]",
    ] {
        let error = one(&with_arguments(
            ROLLOUT_LOGS,
            json!({"resource":"rollouts","selector": bad}),
        ));
        assert_eq!(
            error.path, "capabilities[3].arguments.selector",
            "{bad}: {error:?}"
        );
    }
}

#[test]
fn without_a_resource_the_permission_must_grant_namespaces() {
    let mut value = manifest();
    value["permissions"][2] = json!("k8s.streamLogs");
    let error = one(&value);
    assert_eq!(
        (code(error.code), error.path.as_str()),
        (
            "EXTENSION_INVALID_BINDING".into(),
            "capabilities[4].arguments"
        )
    );
    assert!(
        error.message.contains("resource") && error.message.contains("namespaces"),
        "{error:?}"
    );
    // A resource scope is the object's; granted namespaces do not widen it, and a
    // binding cannot name both.
    let error = one(&with_arguments(
        LOGS,
        json!({"resource":"controllers","namespaces":["kube-system"]}),
    ));
    assert_eq!(error.path, "capabilities[2].arguments.namespaces");
}

#[test]
fn a_namespace_grant_lists_real_namespaces_on_a_pod_capability_only() {
    let with_permission = |permission: Value| {
        let mut value = manifest();
        value["permissions"][2] = permission;
        value
    };
    for (namespaces, why) in [
        (json!([]), "1–16"),
        (json!(["Cert-Manager"]), "namespace"),
        (json!(["cert-manager", "cert-manager"]), "more than once"),
        (
            json!((0..17).map(|n| format!("ns-{n}")).collect::<Vec<_>>()),
            "1–16",
        ),
    ] {
        let errors = errors(&with_permission(
            json!({"capability":"k8s.streamLogs","namespaces": namespaces}),
        ));
        assert!(
            errors
                .iter()
                .any(|e| e.path.starts_with("permissions[2].namespaces") && e.message.contains(why)),
            "{namespaces}: {errors:?}"
        );
    }
    // Only the pod capabilities are granted namespaces, and only network.http hosts.
    let mut value = manifest();
    value["permissions"][0] =
        json!({"capability":"k8s.listDeployments","namespaces":["cert-manager"]});
    let error = one(&value);
    assert_eq!(
        (code(error.code), error.path.as_str()),
        (
            "EXTENSION_INVALID_FIELD".into(),
            "permissions[0].namespaces"
        )
    );
    let error = one(&with_permission(
        json!({"capability":"k8s.streamLogs","hosts":["api.github.com"],"namespaces":["cert-manager"]}),
    ));
    assert_eq!(error.path, "permissions[2].hosts");
    // A scoped entry scopes something, as a `network.http` one lists hosts.
    let found = errors(&with_permission(json!({"capability":"k8s.streamLogs"})));
    assert!(
        found
            .iter()
            .any(|e| e.path == "permissions[2].namespaces" && e.message.contains("1–16")),
        "{found:?}"
    );
    // A namespace is a name, never a setting a person could point elsewhere later.
    let errors = errors(&with_permission(
        json!({"capability":"k8s.streamLogs","namespaces":["${settings.namespace}"]}),
    ));
    assert!(
        errors
            .iter()
            .any(|e| e.path == "permissions[2].namespaces[0]"),
        "{errors:?}"
    );
}

#[test]
fn a_pod_binding_takes_no_inputs_and_no_arguments_it_does_not_know() {
    let mut value = manifest();
    value["capabilities"][LOGS]["inputs"] = json!(["context", "pod"]);
    assert_eq!(
        problems(&value),
        [
            (
                "EXTENSION_INVALID_BINDING".into(),
                "capabilities[2].inputs[0]".into()
            ),
            (
                "EXTENSION_INVALID_BINDING".into(),
                "capabilities[2].inputs[1]".into()
            ),
        ]
    );
    for (index, key) in [
        (LOGS, "command"),
        (LOGS, "port"),
        (EXEC, "stdin"),
        (EXEC, "tty"),
        (EXEC, "port"),
        (FORWARD, "command"),
        (FORWARD, "localPort"),
    ] {
        let mut value = manifest();
        value["capabilities"][index]["arguments"][key] = json!(true);
        let error = one(&value);
        assert_eq!(
            error.path,
            format!("capabilities[{index}].arguments.{key}"),
            "{key}: {error:?}"
        );
    }
}

#[test]
fn an_exec_command_is_one_fixed_program_and_never_a_shell() {
    let command = |command: Value| {
        with_arguments(
            EXEC,
            json!({"resource":"controllers","container":"cert-manager-controller","command": command}),
        )
    };
    let mut missing = manifest();
    missing["capabilities"][EXEC]["arguments"]
        .as_object_mut()
        .unwrap()
        .remove("command");
    assert_eq!(
        one(&missing).path,
        "capabilities[5].arguments.command",
        "a command is required"
    );
    for (bad, path) in [
        (json!([]), "capabilities[5].arguments.command"),
        (json!("cmctl status"), "capabilities[5].arguments.command"),
        (
            json!((0..33).map(|n| n.to_string()).collect::<Vec<_>>()),
            "capabilities[5].arguments.command",
        ),
        (json!(["cmctl", 3]), "capabilities[5].arguments.command[1]"),
        (json!(["cmctl", ""]), "capabilities[5].arguments.command[1]"),
        (
            json!(["cmctl", "a\u{0}b"]),
            "capabilities[5].arguments.command[1]",
        ),
        (
            json!(["cmctl", "x".repeat(1025)]),
            "capabilities[5].arguments.command[1]",
        ),
    ] {
        let errors = errors(&command(bad.clone()));
        assert!(errors.iter().any(|e| e.path == path), "{bad}: {errors:?}");
    }
    for shell in [
        json!(["sh", "-c", "cmctl status"]),
        json!(["/bin/bash", "-lc", "id"]),
        json!(["/usr/bin/env", "sh", "-c", "id"]),
        json!(["busybox", "ash"]),
        // A wrapper's own options and assignments come before the program it runs.
        json!(["env", "-i", "sh", "-c", "id"]),
        json!(["/usr/bin/env", "FOO=1", "bash", "-c", "id"]),
        json!(["env", "-u", "HOME", "sh"]),
        json!(["env", "--unset=HOME", "-C", "/tmp", "--", "sh"]),
        // -S re-splits one argument into a command line, so it is refused outright.
        json!(["env", "-S", "sh -c id"]),
        json!(["env", "--split-string=cmctl status"]),
        json!(["env", "-iS", "sh -c id"]),
        // A wrapper running a wrapper, and the other programs that run the
        // program named after them.
        json!(["env", "busybox", "sh", "-c", "id"]),
        json!(["busybox", "env", "sh", "-c", "id"]),
        json!(["env", "--", "toybox", "sh"]),
        json!(["env", "env", "-S", "sh -c id"]),
        json!(["nice", "sh", "-c", "id"]),
        json!(["nice", "-n", "5", "bash"]),
        json!(["nohup", "sh", "-c", "id"]),
        json!(["timeout", "30", "sh", "-c", "id"]),
        json!(["timeout", "-k", "5", "30", "env", "sh"]),
        json!(["setsid", "-f", "sh"]),
        json!(["stdbuf", "-oL", "sh", "-c", "id"]),
        json!(["xargs", "-0", "sh", "-c"]),
        // Its name anywhere after a wrapper is refused, even as an option's value:
        // the host does not parse every wrapper's options to guess the program.
        json!(["env", "-u", "sh", "cmctl", "version"]),
    ] {
        let error = one(&command(shell.clone()));
        assert_eq!(error.path, "capabilities[5].arguments.command", "{shell}");
        assert!(error.message.contains("shell"), "{shell}: {error:?}");
    }
    // A command reads no setting: what was reviewed is what runs.
    let errors = errors(&command(json!(["cmctl", "${settings.flag}"])));
    assert!(
        errors
            .iter()
            .any(|e| e.path == "capabilities[5].arguments.command[1]"),
        "{errors:?}"
    );
    // A container is a container name.
    let error = one(&with_arguments(
        EXEC,
        json!({"resource":"controllers","container":"Not A Name","command":["cmctl"]}),
    ));
    assert_eq!(error.path, "capabilities[5].arguments.container");
    // `env` or `busybox` running a program that is not a shell is a program.
    parse(&command(json!(["busybox", "nslookup", "example.com"])));
    parse(&command(json!(["/usr/bin/env", "cmctl", "version"])));
    parse(&command(json!(["env", "-i", "LANG=C", "cmctl", "version"])));
    parse(&command(json!(["timeout", "30", "cmctl", "status"])));
    parse(&command(json!(["nice", "-n", "5", "cmctl", "status"])));
    // Without a wrapper, only the program is read: an argument named like a
    // shell is the program's own business.
    parse(&command(json!(["cmctl", "check", "--shell", "sh"])));
}

#[test]
fn a_port_forward_names_one_remote_port() {
    let mut missing = manifest();
    missing["capabilities"][FORWARD]["arguments"]
        .as_object_mut()
        .unwrap()
        .remove("port");
    assert_eq!(one(&missing).path, "capabilities[6].arguments.port");
    for bad in [json!(0), json!(65536), json!("9402"), json!(94.2)] {
        let error = one(&with_arguments(
            FORWARD,
            json!({"resource":"controllers","port": bad}),
        ));
        assert_eq!(error.path, "capabilities[6].arguments.port", "{bad}");
    }
    // Through a Service: the host resolves it to a pod the scope still has to hold.
    parse(&with_arguments(
        FORWARD,
        json!({"resource":"controllers","port":9402,"service":true}),
    ));
    let error = one(&with_arguments(
        FORWARD,
        json!({"resource":"controllers","port":9402,"service":"yes"}),
    ));
    assert_eq!(error.path, "capabilities[6].arguments.service");
}

#[test]
fn a_pod_binding_is_granted_like_any_other() {
    // Bound without being requested, or requested without being bound.
    let mut value = manifest();
    value["permissions"].as_array_mut().unwrap().remove(3);
    assert_eq!(one(&value).code, ValidationCode::PermissionMismatch);
    let mut value = manifest();
    value["capabilities"].as_array_mut().unwrap().remove(EXEC);
    assert_eq!(one(&value).code, ValidationCode::PermissionMismatch);
}
