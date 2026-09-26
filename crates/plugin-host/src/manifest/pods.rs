//! Logs, exec and port-forwards for apps (#567): the pod bindings a manifest
//! declares, and the scope the host holds each one to.
//!
//! A pod binding never names a pod. It names where its pods come from:
//!
//! - `resource`, a reader binding in the same manifest whose kind selects
//!   pods. The view names one object of that kind; the host reads the
//!   object's own label selector — `.spec.selector` for a Deployment,
//!   StatefulSet or DaemonSet, the path the binding declares in `selector` for
//!   a custom resource — and only the pods it selects, in the object's
//!   namespace, are in scope.
//! - or, without one, the namespaces its permission grants:
//!   `{"capability": "k8s.streamLogs", "namespaces": ["cert-manager"]}`.
//!
//! What each binding does is fixed here too: an exec binding's `command` (a
//! list of arguments the host runs without a shell), a port-forward's remote
//! `port`. The view chooses which pod in scope, and nothing else.
use super::{condition_json_path, namespace_name, unique, Binding, Manifest};
use crate::{ValidationCode as Code, ValidationErrors};
use serde_json::Value;

/// Follow a pod's logs, as a stream (#565).
pub const POD_LOGS: &str = "k8s.streamLogs";
/// Run a command fixed in the manifest in a pod's container, once.
pub const POD_EXEC: &str = "k8s.exec";
/// Forward a local port the host picks to a pod's port, for as long as the view is open.
pub const POD_FORWARD: &str = "k8s.portForward";
/// The capabilities a pod binding targets, and the only ones granted namespaces.
pub const POD_TARGETS: &[&str] = &[POD_LOGS, POD_EXEC, POD_FORWARD];
/// Most namespaces one pod permission may grant.
pub const MAX_POD_NAMESPACES: usize = 16;
/// Most arguments one exec command may have, program included.
pub const MAX_COMMAND_ARGS: usize = 32;
/// Most characters one argument of an exec command may have.
pub const MAX_COMMAND_ARG_CHARS: usize = 1024;
/// Where Kubernetes keeps a built-in workload's pod selector.
pub const WORKLOAD_SELECTOR: &str = ".spec.selector";

/// Programs that read a script rather than run one program: an exec command may
/// not be one, because `sh -c` would turn the reviewed command into whatever
/// its script says.
const SHELLS: &[&str] = &[
    "sh", "bash", "ash", "dash", "zsh", "ksh", "mksh", "csh", "tcsh", "fish", "pwsh",
];
/// Programs that run the program named after them. One of these running a shell
/// is a shell.
const WRAPPERS: &[&str] = &["env", "busybox", "toybox"];

/// Whether `target` is a pod binding's.
pub fn is_pod_target(target: &str) -> bool {
    POD_TARGETS.contains(&target)
}

/// The arguments a pod binding of `target` may bind.
fn arguments_of(target: &str) -> &'static [&'static str] {
    match target {
        POD_LOGS => &["resource", "selector", "container"],
        POD_EXEC => &["resource", "selector", "container", "command"],
        _ => &["resource", "selector", "port", "service"],
    }
}

/// Which pods a pod binding may reach.
#[derive(Debug, Clone)]
pub enum PodScope<'a> {
    /// The pods one object of `reader`'s kind selects, read at `selector` on
    /// that object, in its namespace.
    Selected {
        reader: &'a Binding,
        selector: String,
    },
    /// Any pod in these namespaces, which the binding's permission grants.
    Namespaces(&'a [String]),
}

/// The last path segment of a program, which is what names it.
fn program(argument: &str) -> &str {
    argument.rsplit('/').next().unwrap_or(argument)
}

/// A wrapper's short options that take the next argument as their value:
/// `env`'s `-u NAME`, `-C DIR`, `-a ARG0` and BSD's `-P PATH`.
const VALUE_SHORT: &[char] = &['u', 'C', 'a', 'P'];
/// The long spellings of those, which also accept `--name=value`.
const VALUE_LONG: &[&str] = &["--unset", "--chdir", "--argv0"];

/// Whether `command` starts a shell, directly or through a wrapper.
///
/// After a wrapper, its own options (a cluster such as `-iu NAME` included)
/// and `NAME=value` assignments are skipped, and the program they lead to is
/// the one checked. `-S`/`--split-string` is refused outright: it splits one
/// argument into a whole command line, which no list of arguments reviews.
fn runs_a_shell(command: &[&str]) -> bool {
    let Some((first, rest)) = command.split_first() else {
        return false;
    };
    if SHELLS.contains(&program(first)) {
        return true;
    }
    if !WRAPPERS.contains(&program(first)) {
        return false;
    }
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        if *arg == "--" {
            return args
                .next()
                .is_some_and(|next| SHELLS.contains(&program(next)));
        }
        if let Some(long) = arg.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or(long);
            if name == "split-string" {
                return true;
            }
            if VALUE_LONG.contains(&format!("--{name}").as_str()) && !long.contains('=') {
                args.next();
            }
            continue;
        }
        if let Some(cluster) = arg.strip_prefix('-').filter(|cluster| !cluster.is_empty()) {
            for (at, flag) in cluster.char_indices() {
                if flag == 'S' {
                    return true;
                }
                if VALUE_SHORT.contains(&flag) {
                    // The value is the rest of the cluster, or the next argument.
                    if at + flag.len_utf8() == cluster.len() {
                        args.next();
                    }
                    break;
                }
            }
            continue;
        }
        if arg
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty())
        {
            continue;
        }
        return SHELLS.contains(&program(arg));
    }
    false
}

impl Manifest {
    /// The namespaces the `capability` permission grants; empty when it grants none.
    pub fn pod_namespaces(&self, capability: &str) -> &[String] {
        self.permissions
            .iter()
            .find(|permission| permission.capability() == capability)
            .map_or(&[], |permission| permission.namespaces())
    }

    /// The scope a pod binding's pods are held to, or why `binding` has none.
    pub fn pod_scope<'a>(&'a self, binding: &Binding) -> Result<PodScope<'a>, String> {
        if !is_pod_target(&binding.target) {
            return Err(format!("\"{}\" is not a pod binding", binding.name));
        }
        let Some(resource) = binding.arguments.get("resource") else {
            let namespaces = self.pod_namespaces(&binding.target);
            if namespaces.is_empty() {
                return Err(format!(
                    "\"{}\" names no resource and {} grants no namespaces",
                    binding.name, binding.target
                ));
            }
            return Ok(PodScope::Namespaces(namespaces));
        };
        let reader = resource
            .as_str()
            .and_then(|name| self.capabilities.iter().find(|b| b.name == name))
            .ok_or_else(|| format!("\"{}\" names no declared reader", binding.name))?;
        let selector = if crate::builtin_reader_identity(&reader.target).is_some() {
            WORKLOAD_SELECTOR.to_owned()
        } else {
            binding
                .arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("\"{}\" names no selector", binding.name))?
                .to_owned()
        };
        Ok(PodScope::Selected { reader, selector })
    }

    /// An exec binding's command, as the host runs it: each argument as written.
    pub fn exec_command(&self, binding: &Binding) -> Result<Vec<String>, String> {
        if binding.target != POD_EXEC {
            return Err(format!("\"{}\" is not an exec binding", binding.name));
        }
        binding
            .arguments
            .get("command")
            .and_then(Value::as_array)
            .and_then(|args| {
                args.iter()
                    .map(|arg| arg.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| format!("\"{}\" binds no command", binding.name))
    }
}

/// The container a pod binding fixes, if it fixes one.
pub fn pod_container(binding: &Binding) -> Option<&str> {
    binding.arguments.get("container").and_then(Value::as_str)
}

/// A port-forward binding's remote port.
pub fn forward_port(binding: &Binding) -> Option<u16> {
    binding
        .arguments
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
}

/// Whether a port-forward binding reaches its pod through a Service.
pub fn forward_via_service(binding: &Binding) -> bool {
    binding.arguments.get("service") == Some(&Value::Bool(true))
}

/// Every problem with the manifest's pod bindings.
pub(super) fn pod_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    for (index, binding) in manifest.capabilities.iter().enumerate() {
        if !is_pod_target(&binding.target) {
            continue;
        }
        let at = format!("capabilities[{index}]");
        for position in 0..binding.inputs.len() {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.inputs[{position}]"),
                "The host supplies a pod binding's cluster, object, pod and container; it declares no inputs",
            );
        }
        let known = arguments_of(&binding.target);
        for key in binding.arguments.keys() {
            if key == "namespaces" {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.arguments.{key}"),
                    format!(
                        "Namespaces are granted on the {} permission, not bound",
                        binding.target
                    ),
                );
            } else if !known.contains(&key.as_str()) {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.arguments.{key}"),
                    format!("{} takes no argument \"{key}\"", binding.target),
                );
            }
        }
        scope_problems(manifest, &at, binding, problems);
        if let Some(container) = binding.arguments.get("container") {
            if !container.as_str().is_some_and(namespace_name) {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.arguments.container"),
                    "A container name: 1–63 lowercase letters, digits and -",
                );
            }
        }
        match binding.target.as_str() {
            POD_EXEC => command_problems(&at, binding.arguments.get("command"), problems),
            POD_FORWARD => {
                if forward_port(binding).is_none() {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.arguments.port"),
                        "Bind the remote port to forward to: a number from 1 to 65535",
                    );
                }
                if binding
                    .arguments
                    .get("service")
                    .is_some_and(|service| !service.is_boolean())
                {
                    problems.push(
                        Code::InvalidValue,
                        format!("{at}.arguments.service"),
                        "service is true, to forward through a Service, or false",
                    );
                }
            }
            _ => {}
        }
    }
}

/// Where the binding's pods come from: a reader whose objects select them, or
/// the namespaces its permission grants.
fn scope_problems(
    manifest: &Manifest,
    at: &str,
    binding: &Binding,
    problems: &mut ValidationErrors,
) {
    let selector = binding.arguments.get("selector");
    let Some(resource) = binding.arguments.get("resource") else {
        if selector.is_some() {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments.selector"),
                "A selector is read from the object `resource` names; name one",
            );
        }
        if manifest.pod_namespaces(&binding.target).is_empty() {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments"),
                format!(
                    "Scope the pods: name a reader whose objects select them in `resource`, or grant namespaces on the {} permission",
                    binding.target
                ),
            );
        }
        return;
    };
    let Some(reader) = resource
        .as_str()
        .and_then(|name| manifest.capabilities.iter().find(|b| b.name == name))
    else {
        problems.push(
            Code::UnresolvedCapability,
            format!("{at}.arguments.resource"),
            format!("Capability {resource} is not declared"),
        );
        return;
    };
    const SELECTS: &str = "A pod scope names a Deployment, StatefulSet or DaemonSet reader, or a namespaced custom-resource reader, whose objects select pods";
    if let Some(identity) = crate::builtin_reader_identity(&reader.target) {
        if identity["namespaced"] != true {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments.resource"),
                format!(
                    "{SELECTS}; \"{}\" lists {}s",
                    reader.name,
                    identity["kind"].as_str().unwrap_or("objects")
                ),
            );
            return;
        }
        if selector.is_some() {
            let kind = identity["kind"].as_str().unwrap_or("workload");
            problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments.selector"),
                format!("The host reads a {kind}'s pod selector at {WORKLOAD_SELECTOR}; a built-in reader's selector is not the app's to name"),
            );
        }
        return;
    }
    if reader.target != "k8s.listCustomResource" {
        problems.push(
            Code::InvalidBinding,
            format!("{at}.arguments.resource"),
            format!(
                "{SELECTS}; \"{}\" is a {} binding",
                reader.name, reader.target
            ),
        );
        return;
    }
    if reader.arguments.get("namespaced") != Some(&Value::Bool(true)) {
        problems.push(
            Code::InvalidBinding,
            format!("{at}.arguments.resource"),
            format!("{SELECTS}; \"{}\" lists a kind that is not namespaced, so its objects have no namespace for their pods to be in", reader.name),
        );
        return;
    }
    let kind = reader
        .arguments
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("resource");
    match selector {
        None => problems.push(
            Code::InvalidBinding,
            format!("{at}.arguments.selector"),
            format!("Name where a {kind} keeps its pod selector, e.g. {WORKLOAD_SELECTOR}"),
        ),
        Some(path) if !path.as_str().is_some_and(condition_json_path) => problems.push(
            Code::InvalidValue,
            format!("{at}.arguments.selector"),
            format!("A selector path is plain dot-separated object keys, e.g. {WORKLOAD_SELECTOR}"),
        ),
        Some(_) => {}
    }
}

/// An exec command: 1–32 literal arguments, the first a program that is not a shell.
fn command_problems(at: &str, command: Option<&Value>, problems: &mut ValidationErrors) {
    let path = format!("{at}.arguments.command");
    let Some(command) = command else {
        problems.push(
            Code::InvalidBinding,
            path,
            "Bind the command to run, as a list of arguments, e.g. [\"cmctl\", \"status\"]",
        );
        return;
    };
    let Some(arguments) = command.as_array() else {
        problems.push(
            Code::InvalidBinding,
            path,
            "Write the command as a list of arguments, e.g. [\"cmctl\", \"status\"]: the host runs it without a shell",
        );
        return;
    };
    if arguments.is_empty() || arguments.len() > MAX_COMMAND_ARGS {
        problems.push(
            Code::InvalidValue,
            path,
            format!("A command has 1–{MAX_COMMAND_ARGS} arguments"),
        );
        return;
    }
    let mut text = Vec::new();
    for (position, argument) in arguments.iter().enumerate() {
        let ok = argument.as_str().filter(|arg| {
            !arg.is_empty()
                && arg.chars().count() <= MAX_COMMAND_ARG_CHARS
                && !arg
                    .chars()
                    .any(|c| c.is_control() || crate::is_format_character(c))
        });
        match ok {
            Some(arg) => text.push(arg),
            None => problems.push(
                Code::InvalidValue,
                format!("{path}[{position}]"),
                format!("Each argument is text of 1–{MAX_COMMAND_ARG_CHARS} characters with no control or invisible format characters"),
            ),
        }
    }
    if runs_a_shell(&text) {
        problems.push(
            Code::InvalidBinding,
            path,
            "A command runs one program, never a shell: `sh -c` would turn the command a person reviewed into whatever its script says",
        );
    }
}

/// The rules for a namespace grant on a pod permission. Called for a scoped
/// entry whose capability is a pod capability.
pub(super) fn namespace_problems(at: &str, namespaces: &[String], problems: &mut ValidationErrors) {
    if namespaces.is_empty() || namespaces.len() > MAX_POD_NAMESPACES {
        problems.push(
            Code::InvalidValue,
            format!("{at}.namespaces"),
            format!("List 1–{MAX_POD_NAMESPACES} namespaces, or grant the capability by its id to scope every binding by `resource`"),
        );
    }
    unique(
        problems,
        namespaces.iter().enumerate().map(|(position, namespace)| {
            (format!("{at}.namespaces[{position}]"), namespace.as_str())
        }),
    );
    for (position, namespace) in namespaces.iter().enumerate() {
        if !namespace_name(namespace) {
            problems.push(
                Code::InvalidValue,
                format!("{at}.namespaces[{position}]"),
                format!("\"{namespace}\" is not a Kubernetes namespace name"),
            );
        }
    }
}
