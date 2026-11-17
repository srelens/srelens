//! Install Kubecost into a cluster, for the `:cost` dashboard to read.
//!
//! This only builds the Helm commands and drives the sequence. It never runs
//! anything on its own: the caller supplies the function that runs a Helm
//! command, and is expected to have asked the user first. That keeps the
//! sequence testable without a cluster, a Helm binary or a network.
//!
//! Pinned to chart 2.8.7. The 2.9 line is a stepping stone to Kubecost v3 and
//! refuses to install without a federated object store, which is not a thing
//! to ask of someone who just wants to see their spend.

use std::future::Future;

pub const REPO_NAME: &str = "kubecost";
pub const REPO_URL: &str = "https://kubecost.github.io/cost-analyzer/";
pub const CHART: &str = "kubecost/cost-analyzer";
pub const CHART_VERSION: &str = "2.8.7";
pub const RELEASE: &str = "kubecost";
pub const NAMESPACE: &str = "kubecost";
/// How long `helm --wait` waits for the pods, which pull a few images.
pub const WAIT_TIMEOUT: &str = "8m";

/// Where in the sequence we are, for the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    AddingRepo,
    UpdatingRepo,
    Installing,
    /// Installed; waiting for Kubecost to report its first figures.
    WaitingForData,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::AddingRepo => "Adding the Kubecost Helm repository",
            Step::UpdatingRepo => "Updating the Kubecost chart index",
            Step::Installing => "Installing Kubecost and waiting for its pods (a few minutes)",
            Step::WaitingForData => "Installed. Waiting for Kubecost's first figures",
        }
    }
}

/// Which cluster a Helm command is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Repository commands; they never touch a cluster.
    Local,
    /// Release commands, run against the cluster being installed into.
    Cluster,
}

/// A failure, with the step it happened in so the screen can say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub step: Option<Step>,
    pub message: String,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.step {
            Some(step) => write!(f, "{}: {}", step.label(), self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// A context name made safe to hand to `helm --set`: kubeconfig context names
/// can hold `@`, `/`, `:` and `,`, and a comma splits a `--set` value. Kubecost
/// wants the same id in two places, so it is one function.
pub fn cluster_id(context: &str) -> String {
    let cleaned: String = context
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '-' || c == '.').to_string();
    let capped: String = trimmed.chars().take(63).collect();
    if capped.is_empty() {
        "cluster".to_string()
    } else {
        capped
    }
}

pub fn repo_add_args() -> Vec<String> {
    crate::helm_cli::repo_add_args(REPO_NAME, REPO_URL)
}

/// Only Kubecost's repository: `helm repo update` alone would also contact
/// every other repository the user has, and fail on one of those.
pub fn repo_update_args() -> Vec<String> {
    vec!["repo".into(), "update".into(), REPO_NAME.into()]
}

/// `helm upgrade --install`, with the cluster id Kubecost wants in both places
/// and `--wait` so success means the pods are up.
pub fn install_args(context: &str) -> Vec<String> {
    let id = cluster_id(context);
    [
        "upgrade",
        "--install",
        RELEASE,
        CHART,
        "--version",
        CHART_VERSION,
        "--namespace",
        NAMESPACE,
        "--create-namespace",
        "--set",
        &format!("global.clusterId={id}"),
        "--set",
        &format!("prometheus.server.global.external_labels.cluster_id={id}"),
        "--wait",
        "--timeout",
        WAIT_TIMEOUT,
        "-o",
        "json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The two commands that undo the install, one per line so each can be copied
/// whole.
pub fn removal_steps(context: &str) -> [String; 2] {
    [
        format!("helm uninstall {RELEASE} -n {NAMESPACE} --kube-context {context}"),
        format!("kubectl --context {context} delete namespace {NAMESPACE}"),
    ]
}

/// The same, as one line for a shell.
pub fn removal_commands(context: &str) -> String {
    removal_steps(context).join(" && ")
}

/// A context named like a production cluster gets a louder warning.
pub fn looks_like_production(context: &str) -> bool {
    let lower = context.to_lowercase();
    ["prod", "prd", "live"].iter().any(|w| lower.contains(w))
}

/// Run the install sequence.
///
/// `run` executes one Helm command (its arguments and where it runs) and
/// returns its output or an error; `progress` is told each step as it starts.
/// `existing` is the namespace of a Helm release already named `kubecost`, if
/// the caller found one: that is somebody's install, and it is left alone.
pub async fn install<R, Fut>(
    context: &str,
    existing: Option<&str>,
    run: R,
    mut progress: impl FnMut(Step),
) -> Result<(), InstallError>
where
    R: Fn(Vec<String>, Scope) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    if let Some(namespace) = existing {
        return Err(InstallError {
            step: None,
            message: format!(
                "A Helm release named '{RELEASE}' already exists in namespace '{namespace}'. \
                 Not touching it. If it is Kubecost, give it time to collect data; if it is not \
                 working, check its pods."
            ),
        });
    }
    let steps: [(Step, Vec<String>, Scope); 3] = [
        (Step::AddingRepo, repo_add_args(), Scope::Local),
        (Step::UpdatingRepo, repo_update_args(), Scope::Local),
        (Step::Installing, install_args(context), Scope::Cluster),
    ];
    for (step, args, scope) in steps {
        progress(step);
        run(args, scope).await.map_err(|message| InstallError { step: Some(step), message })?;
    }
    progress(Step::WaitingForData);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn the_cluster_id_is_safe_for_helm_set_and_never_empty() {
        assert_eq!(cluster_id("kind-srectl-test"), "kind-srectl-test");
        assert_eq!(cluster_id("arn:aws:eks:us-east-1:123:cluster/prod"), "arn-aws-eks-us-east-1-123-cluster-prod");
        assert_eq!(cluster_id("a,b=c d"), "a-b-c-d", "a comma would split the --set value");
        assert_eq!(cluster_id("@@@"), "cluster");
        assert_eq!(cluster_id(""), "cluster");
        assert!(cluster_id(&"x".repeat(200)).len() <= 63);
    }

    #[test]
    fn the_cluster_id_is_the_same_in_both_places_kubecost_checks() {
        let args = install_args("gke_proj_zone_my-cluster");
        let sets: Vec<&String> = args.iter().filter(|a| a.contains("cluster_id=") || a.contains("clusterId=")).collect();
        assert_eq!(sets.len(), 2);
        let ids: Vec<&str> = sets.iter().map(|s| s.split('=').nth(1).unwrap()).collect();
        assert_eq!(ids[0], ids[1]);
        assert_eq!(ids[0], "gke_proj_zone_my-cluster");
    }

    #[test]
    fn the_install_is_pinned_waits_and_creates_the_namespace() {
        let a = install_args("kind-x");
        let has = |w: &str| a.iter().any(|x| x == w);
        assert_eq!(&a[0..2], ["upgrade", "--install"]);
        assert!(has("kubecost") && has("kubecost/cost-analyzer"));
        let at = a.iter().position(|x| x == "--version").unwrap();
        assert_eq!(a[at + 1], CHART_VERSION, "never the floating latest chart");
        assert!(has("--create-namespace") && has("--wait") && has("--timeout"));
        let ns = a.iter().position(|x| x == "--namespace").unwrap();
        assert_eq!(a[ns + 1], NAMESPACE);
    }

    #[test]
    fn no_argument_can_be_mistaken_for_a_flag_by_a_hostile_context_name() {
        for hostile in ["--post-renderer=/tmp/x", "-f/etc/passwd", "--kubeconfig=/x"] {
            for arg in install_args(hostile) {
                if arg.contains("clusterId=") || arg.contains("cluster_id=") {
                    let value = arg.split('=').nth(1).unwrap();
                    assert!(!value.starts_with('-'), "{arg}");
                }
            }
        }
    }

    #[test]
    fn updating_touches_only_the_kubecost_repository() {
        assert_eq!(repo_update_args(), ["repo", "update", "kubecost"]);
    }

    #[test]
    fn the_repo_is_added_by_the_existing_builder() {
        let a = repo_add_args();
        assert!(a.contains(&REPO_URL.to_string()) && a.contains(&"kubecost".to_string()));
    }

    #[test]
    fn the_removal_note_names_the_cluster_and_both_commands() {
        let r = removal_commands("kind-x");
        assert!(r.contains("helm uninstall kubecost") && r.contains("--kube-context kind-x"));
        assert!(r.contains("delete namespace kubecost"));
    }

    #[test]
    fn each_removal_step_is_a_whole_command() {
        let [uninstall, delete] = removal_steps("kind-x");
        assert!(uninstall.starts_with("helm uninstall kubecost") && uninstall.contains("--kube-context kind-x"));
        assert!(delete.starts_with("kubectl --context kind-x delete namespace kubecost"));
        assert_eq!(removal_commands("kind-x"), format!("{uninstall} && {delete}"));
    }

    #[test]
    fn production_looking_clusters_are_recognised() {
        for yes in ["prod", "eks-PROD-1", "prd-east", "live-cluster"] {
            assert!(looks_like_production(yes), "{yes}");
        }
        for no in ["kind-srectl-test", "staging", "dev", "k3d-client-meta"] {
            assert!(!looks_like_production(no), "{no}");
        }
    }

    type Log = Rc<RefCell<Vec<(Vec<String>, Scope)>>>;

    fn recorder(log: Log, fail_at: Option<usize>) -> impl Fn(Vec<String>, Scope) -> std::future::Ready<Result<String, String>> {
        move |args, scope| {
            let n = log.borrow().len();
            log.borrow_mut().push((args, scope));
            std::future::ready(if Some(n) == fail_at { Err("boom".to_string()) } else { Ok("ok".to_string()) })
        }
    }

    #[tokio::test]
    async fn the_sequence_adds_the_repo_then_updates_it_then_installs_in_the_cluster() {
        let log: Log = Default::default();
        let mut steps = Vec::new();
        install("kind-x", None, recorder(log.clone(), None), |s| steps.push(s)).await.unwrap();
        assert_eq!(steps, [Step::AddingRepo, Step::UpdatingRepo, Step::Installing, Step::WaitingForData]);
        let log = log.borrow();
        assert_eq!(log.len(), 3);
        assert_eq!((log[0].0[0].as_str(), log[0].1), ("repo", Scope::Local));
        assert_eq!((log[1].0[1].as_str(), log[1].1), ("update", Scope::Local));
        assert_eq!((log[2].0[0].as_str(), log[2].1), ("upgrade", Scope::Cluster));
    }

    #[tokio::test]
    async fn a_failure_stops_the_sequence_and_names_the_step() {
        for (fail_at, step) in [(0, Step::AddingRepo), (1, Step::UpdatingRepo), (2, Step::Installing)] {
            let log: Log = Default::default();
            let err = install("kind-x", None, recorder(log.clone(), Some(fail_at)), |_| {}).await.unwrap_err();
            assert_eq!(err.step, Some(step));
            assert_eq!(log.borrow().len(), fail_at + 1, "nothing runs after the failure");
            assert!(err.to_string().contains("boom") && err.to_string().contains(step.label()));
        }
    }

    #[tokio::test]
    async fn an_existing_kubecost_release_is_never_touched() {
        let log: Log = Default::default();
        let mut steps = Vec::new();
        let err = install("kind-x", Some("monitoring"), recorder(log.clone(), None), |s| steps.push(s))
            .await
            .unwrap_err();
        assert!(log.borrow().is_empty() && steps.is_empty(), "not one helm command ran");
        assert!(err.message.contains("already exists") && err.message.contains("monitoring"));
        assert_eq!(err.step, None);
    }
}
