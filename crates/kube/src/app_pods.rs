//! Pod access on an app's behalf (#567): the reads a pod scope needs, one
//! non-interactive exec, and one port-forward connection.
//!
//! Tauri-agnostic and policy-free on purpose. Which pods an app may reach, and
//! whether a session may start at all, is the extension broker's to decide
//! (`crates/registry/src/extensions/pods.rs`); this module only carries out
//! what it decided, with the user's own credentials, so every call is still
//! held to the cluster's RBAC.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{AttachParams, DynamicObject, ListParams, Portforwarder};
use kube::Api;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// What the broker needs to know about one pod: enough to hold it to a scope,
/// offer its containers, and map a Service's target port onto it. Never its
/// spec or its environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PodFacts {
    pub name: String,
    pub namespace: String,
    pub labels: BTreeMap<String, String>,
    /// Its containers, in the order the pod declares them. Init and ephemeral
    /// containers are not offered.
    pub containers: Vec<String>,
    /// Every container port, for resolving a Service's named target port.
    pub ports: Vec<ContainerPort>,
    pub phase: Option<String>,
    /// Every container reports ready.
    pub ready: bool,
    /// It has a deletion timestamp.
    pub deleting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerPort {
    pub name: Option<String>,
    pub port: u16,
}

impl PodFacts {
    /// Running, and not on its way out.
    pub fn running(&self) -> bool {
        !self.deleting && self.phase.as_deref() == Some("Running")
    }
}

/// What the broker needs to know about one Service.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceFacts {
    pub name: String,
    pub namespace: String,
    /// Its selector; empty for a Service that selects no pods (headless with
    /// hand-made endpoints, or an ExternalName).
    pub selector: BTreeMap<String, String>,
    pub ports: Vec<ServicePort>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePort {
    pub name: Option<String>,
    pub port: u16,
    pub target: PortTarget,
}

/// Where a Service port sends its traffic on a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortTarget {
    Number(u16),
    Name(String),
}

impl ServicePort {
    /// The port on `pod` this Service port sends to, if the pod has it.
    pub fn on(&self, pod: &PodFacts) -> Option<u16> {
        match &self.target {
            PortTarget::Number(port) => Some(*port),
            PortTarget::Name(name) => pod
                .ports
                .iter()
                .find(|p| p.name.as_deref() == Some(name.as_str()))
                .map(|p| p.port),
        }
    }
}

/// The facts of `pod`.
pub fn pod_facts(pod: &Pod) -> PodFacts {
    let spec = pod.spec.as_ref();
    let status = pod.status.as_ref();
    let containers = spec
        .map(|spec| spec.containers.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default();
    let ports = spec
        .into_iter()
        .flat_map(|spec| spec.containers.iter())
        .flat_map(|c| c.ports.iter().flatten())
        .filter_map(|p| {
            Some(ContainerPort {
                name: p.name.clone(),
                port: u16::try_from(p.container_port).ok()?,
            })
        })
        .collect();
    let ready = status
        .and_then(|s| s.container_statuses.as_ref())
        .is_some_and(|statuses| !statuses.is_empty() && statuses.iter().all(|c| c.ready));
    PodFacts {
        name: pod.metadata.name.clone().unwrap_or_default(),
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        labels: pod.metadata.labels.clone().unwrap_or_default(),
        containers,
        ports,
        phase: status.and_then(|s| s.phase.clone()),
        ready,
        deleting: pod.metadata.deletion_timestamp.is_some(),
    }
}

/// The facts of `service`.
pub fn service_facts(service: &Service) -> ServiceFacts {
    let spec = service.spec.as_ref();
    let ports = spec
        .and_then(|spec| spec.ports.as_ref())
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let port = u16::try_from(p.port).ok()?;
            let target = match &p.target_port {
                Some(IntOrString::Int(n)) => PortTarget::Number(u16::try_from(*n).ok()?),
                Some(IntOrString::String(name)) => match name.parse::<u16>() {
                    Ok(n) => PortTarget::Number(n),
                    Err(_) => PortTarget::Name(name.clone()),
                },
                None => PortTarget::Number(port),
            };
            Some(ServicePort {
                name: p.name.clone(),
                port,
                target,
            })
        })
        .collect();
    ServiceFacts {
        name: service.metadata.name.clone().unwrap_or_default(),
        namespace: service.metadata.namespace.clone().unwrap_or_default(),
        selector: spec
            .and_then(|spec| spec.selector.clone())
            .unwrap_or_default(),
        ports,
    }
}

/// One namespaced object of any kind, as the API serves it: `None` when the
/// cluster says there is none.
#[allow(clippy::too_many_arguments)]
pub async fn get_object(
    cache: &ClientCache,
    context: &str,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    namespace: &str,
    name: &str,
) -> Result<Option<Value>, String> {
    let client = cache.get(context).await?;
    let resource = crate::crds::custom_api_resource(group, version, kind, plural);
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &resource);
    let object = tokio::time::timeout(request_timeout(), api.get_opt(name))
        .await
        .map_err(|_| format!("reading {kind} {namespace}/{name} timed out"))?
        .map_err(|e| e.to_string())?;
    object
        .map(|object| serde_json::to_value(object).map_err(|e| e.to_string()))
        .transpose()
}

/// The pods in `namespace` that `labels` (a label selector query) selects, and
/// whether the list stopped at the host's cap with more left on the server.
pub async fn list_pods(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    labels: &str,
) -> Result<(Vec<PodFacts>, bool), String> {
    let client = cache.get(context).await?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let params = if labels.is_empty() {
        ListParams::default()
    } else {
        ListParams::default().labels(labels)
    };
    // Each page has its own time budget inside `list_capped`.
    let (pods, truncated) = crate::list_cap::list_capped(&api, params)
        .await
        .map_err(|e| e.into_capability_error("list pods").to_string())?;
    Ok((pods.iter().map(pod_facts).collect(), truncated))
}

/// One pod: `None` when the cluster says there is none.
pub async fn get_pod(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    name: &str,
) -> Result<Option<PodFacts>, String> {
    let client = cache.get(context).await?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let pod = tokio::time::timeout(request_timeout(), api.get_opt(name))
        .await
        .map_err(|_| format!("reading pod {namespace}/{name} timed out"))?
        .map_err(|e| e.to_string())?;
    Ok(pod.as_ref().map(pod_facts))
}

/// The Services in `namespace`.
pub async fn list_services(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
) -> Result<Vec<ServiceFacts>, String> {
    let client = cache.get(context).await?;
    let api: Api<Service> = Api::namespaced(client, namespace);
    let (services, _) = crate::list_cap::list_capped(&api, ListParams::default())
        .await
        .map_err(|e| e.into_capability_error("list services").to_string())?;
    Ok(services.iter().map(service_facts).collect())
}

/// Which of a command's output streams a chunk came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Stdout,
    Stderr,
}

impl Output {
    pub fn as_str(self) -> &'static str {
        match self {
            Output::Stdout => "stdout",
            Output::Stderr => "stderr",
        }
    }
}

/// The exit code the exec protocol's status reports: 0 for `Success`, the
/// code a `NonZeroExitCode` failure names, and an error for any other failure
/// — the command did not run to an exit at all (no such container, no such
/// program), and the cluster's own words say why.
///
/// A status that never arrived is not a success: the connection ended without
/// saying how the command did.
pub fn exit_code(status: Option<&Status>) -> Result<i32, String> {
    let Some(status) = status else {
        return Err("The connection ended before the command reported how it exited".into());
    };
    if status
        .status
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("Success"))
    {
        return Ok(0);
    }
    if status.reason.as_deref() == Some("NonZeroExitCode") {
        let code = status
            .details
            .as_ref()
            .and_then(|details| details.causes.as_ref())
            .into_iter()
            .flatten()
            .find(|cause| cause.reason.as_deref() == Some("ExitCode"))
            .and_then(|cause| cause.message.as_deref())
            .and_then(|message| message.trim().parse::<i32>().ok());
        if let Some(code) = code {
            return Ok(code);
        }
    }
    Err(crate::exec::status_error(Some(status))
        .unwrap_or_else(|| "The command failed without saying why".into()))
}

/// The text `bytes` completes, appended to what `carry` held back from the last
/// read. A read can end inside a multi-byte character; that incomplete tail is
/// kept in `carry` for the next read instead of being decoded as U+FFFD. Bytes
/// that are not UTF-8 at all are decoded lossily at once, never held.
fn decode(carry: &mut Vec<u8>, bytes: &[u8]) -> String {
    carry.extend_from_slice(bytes);
    let complete = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        // Only an incomplete sequence at the very end is worth waiting for.
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => carry.len(),
    };
    let text = String::from_utf8_lossy(&carry[..complete]).into_owned();
    carry.drain(..complete);
    text
}

/// What `carry` still holds when its stream closes, lossily.
fn flush(carry: &mut Vec<u8>) -> String {
    let text = String::from_utf8_lossy(carry).into_owned();
    carry.clear();
    text
}

/// Run `command` in `container` of `pod` once, with no stdin and no terminal,
/// handing each chunk of stdout and stderr to `on_output` as it arrives.
/// Returns the command's exit code; see [`exit_code`].
///
/// The command is passed as its arguments, never through a shell.
pub async fn exec_once<F>(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    pod: &str,
    container: &str,
    command: Vec<String>,
    mut on_output: F,
) -> Result<i32, String>
where
    F: FnMut(Output, String) + Send,
{
    let client = cache.get(context).await?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let params = AttachParams::default()
        .stdin(false)
        .stdout(true)
        .stderr(true)
        .tty(false)
        .container(container);
    let mut attached = tokio::time::timeout(request_timeout(), api.exec(pod, command, &params))
        .await
        .map_err(|_| format!("starting the command in {namespace}/{pod} timed out"))?
        .map_err(|e| e.to_string())?;
    let status = attached.take_status();
    let mut stdout = attached.stdout().ok_or("exec: no stdout")?;
    let mut stderr = attached.stderr().ok_or("exec: no stderr")?;
    let (mut out_buf, mut err_buf) = (vec![0u8; 8192], vec![0u8; 8192]);
    // Each stream's incomplete trailing character, until its next read.
    let (mut out_carry, mut err_carry) = (Vec::new(), Vec::new());
    let (mut out_open, mut err_open) = (true, true);
    let mut emit = |stream: Output, text: String| {
        if !text.is_empty() {
            on_output(stream, text);
        }
    };
    while out_open || err_open {
        tokio::select! {
            read = stdout.read(&mut out_buf), if out_open => match read {
                Ok(0) | Err(_) => {
                    out_open = false;
                    emit(Output::Stdout, flush(&mut out_carry));
                }
                Ok(n) => emit(Output::Stdout, decode(&mut out_carry, &out_buf[..n])),
            },
            read = stderr.read(&mut err_buf), if err_open => match read {
                Ok(0) | Err(_) => {
                    err_open = false;
                    emit(Output::Stderr, flush(&mut err_carry));
                }
                Ok(n) => emit(Output::Stderr, decode(&mut err_carry, &err_buf[..n])),
            },
        }
    }
    let _ = attached.join().await;
    let status = match status {
        Some(status) => status.await,
        None => None,
    };
    exit_code(status.as_ref())
}

/// One connection through a port-forward: bytes written go to the pod's port,
/// and what the pod sends back is read. Dropping it ends the connection and
/// the forwarding task behind it.
pub trait Upstream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Upstream for T {}

/// A port stream and the forwarder that carries it. The forwarder's task is a
/// detached `JoinHandle`, which dropping does not stop, so it is aborted here:
/// otherwise a connection a view's close dropped would keep its WebSocket to
/// the API server open.
struct Forwarded<S> {
    stream: S,
    forwarder: Portforwarder,
}

impl<S> Drop for Forwarded<S> {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Forwarded<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Forwarded<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

/// Open one port-forward connection to `port` of `pod`.
pub async fn connect_port(
    cache: Arc<ClientCache>,
    context: &str,
    namespace: &str,
    pod: &str,
    port: u16,
) -> Result<Box<dyn Upstream>, String> {
    let client = cache.get(context).await?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let mut forwarder = tokio::time::timeout(request_timeout(), api.portforward(pod, &[port]))
        .await
        .map_err(|_| format!("forwarding to {namespace}/{pod}:{port} timed out"))?
        .map_err(|e| e.to_string())?;
    let stream = forwarder
        .take_stream(port)
        .ok_or_else(|| format!("the cluster opened no stream for port {port}"))?;
    Ok(Box::new(Forwarded { stream, forwarder }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(value: Value) -> Status {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_success_is_exit_zero_and_a_non_zero_exit_is_its_code() {
        assert_eq!(
            exit_code(Some(&status(json!({"status": "Success"})))),
            Ok(0)
        );
        let three = status(json!({
            "status": "Failure", "reason": "NonZeroExitCode",
            "message": "command terminated with non-zero exit code: error executing command [false], exit code 3",
            "details": {"causes": [{"reason": "ExitCode", "message": "3"}]}
        }));
        assert_eq!(exit_code(Some(&three)), Ok(3));
    }

    /// A command that never ran is not an exit code, and neither is silence.
    #[test]
    fn a_failure_to_run_says_why_and_silence_is_not_success() {
        let missing = status(json!({
            "status": "Failure", "reason": "InternalError",
            "message": "exec: \"cmctl\": executable file not found in $PATH"
        }));
        let why = exit_code(Some(&missing)).unwrap_err();
        assert!(why.contains("executable file not found"), "{why}");
        assert!(exit_code(None)
            .unwrap_err()
            .contains("before the command reported"));
        // A NonZeroExitCode whose code cannot be read is a failure, not a guess.
        let unreadable =
            status(json!({"status": "Failure", "reason": "NonZeroExitCode", "message": "exit"}));
        assert!(exit_code(Some(&unreadable)).is_err());
    }

    /// A character split across two reads is one character, not two U+FFFD.
    #[test]
    fn a_character_split_across_reads_decodes_whole() {
        let text = "réseau 日本";
        let bytes = text.as_bytes();
        for cut in 1..bytes.len() {
            let mut carry = Vec::new();
            let mut out = decode(&mut carry, &bytes[..cut]);
            out.push_str(&decode(&mut carry, &bytes[cut..]));
            out.push_str(&flush(&mut carry));
            assert_eq!(out, text, "cut at {cut}");
        }
        // Bytes that are not UTF-8 at all are still shown, lossily, not held forever.
        let mut carry = Vec::new();
        assert_eq!(decode(&mut carry, b"a\xffb"), "a\u{fffd}b");
        assert!(carry.is_empty());
        // An incomplete sequence at the end of the stream is flushed lossily.
        let mut carry = Vec::new();
        assert_eq!(decode(&mut carry, &"é".as_bytes()[..1]), "");
        assert_eq!(flush(&mut carry), "\u{fffd}");
    }

    #[test]
    fn pod_facts_keep_identity_containers_and_ports_only() {
        let pod: Pod = serde_json::from_value(json!({
            "metadata": {"name": "web-1", "namespace": "team", "labels": {"app": "web"}},
            "spec": {"containers": [
                {"name": "app", "image": "x", "ports": [{"name": "metrics", "containerPort": 9402}],
                 "env": [{"name": "TOKEN", "value": "s3cr3t"}]},
                {"name": "sidecar", "image": "y"}
            ]},
            "status": {"phase": "Running", "containerStatuses": [
                {"name": "app", "image": "x", "imageID": "", "ready": true, "restartCount": 0},
                {"name": "sidecar", "image": "y", "imageID": "", "ready": true, "restartCount": 0}
            ]}
        }))
        .unwrap();
        let facts = pod_facts(&pod);
        assert_eq!(facts.containers, ["app", "sidecar"]);
        assert_eq!(facts.labels["app"], "web");
        assert!(facts.ready && facts.running());
        assert_eq!(
            facts.ports,
            [ContainerPort {
                name: Some("metrics".into()),
                port: 9402
            }]
        );
        assert!(!format!("{facts:?}").contains("s3cr3t"));
    }

    #[test]
    fn a_service_port_maps_onto_a_pod_by_number_or_by_name() {
        let service: Service = serde_json::from_value(json!({
            "metadata": {"name": "web", "namespace": "team"},
            "spec": {"selector": {"app": "web"}, "ports": [
                {"name": "http", "port": 80, "targetPort": 8080},
                {"name": "metrics", "port": 9402, "targetPort": "metrics"},
                {"port": 443}
            ]}
        }))
        .unwrap();
        let facts = service_facts(&service);
        let pod = PodFacts {
            ports: vec![ContainerPort {
                name: Some("metrics".into()),
                port: 19402,
            }],
            ..PodFacts::default()
        };
        assert_eq!(facts.ports[0].on(&pod), Some(8080));
        assert_eq!(facts.ports[1].on(&pod), Some(19402));
        assert_eq!(facts.ports[2].on(&pod), Some(443));
        assert_eq!(facts.ports[1].on(&PodFacts::default()), None);
    }
}
