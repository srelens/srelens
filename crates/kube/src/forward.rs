//! Local port-forwarding via kube-rs. Binds a loopback TCP listener and pipes
//! each inbound connection through its own port-forward stream to a pod —
//! Tauri-agnostic so the listener/stream plumbing stays reusable and testable.

use std::fmt;
use std::io::ErrorKind;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{Pod, Service, ServicePort};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::ListParams;
use kube::Api;
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;

use crate::client_cache::ClientCache;

/// Why `bind_local` failed. `InUse` carries a `suggested` free port (found by
/// binding `:0` once) so callers can offer it to the user instead of just
/// failing outright.
#[derive(Debug)]
pub enum BindError {
    InUse { requested: u16, suggested: u16 },
    Io(String),
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindError::InUse { requested, suggested } => write!(
                f,
                "port {requested} is already in use; {suggested} is free"
            ),
            BindError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for BindError {}

/// Bind a loopback TCP listener. A `port` of 0 lets the OS pick a free port;
/// read `local_addr()` on the returned listener for the chosen port. If
/// `port` is already taken, binds `:0` to find a free port to suggest and
/// returns `BindError::InUse` with that suggestion.
pub async fn bind_local(port: u16) -> Result<TcpListener, BindError> {
    match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            let suggested = TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| BindError::Io(e.to_string()))?
                .local_addr()
                .map_err(|e| BindError::Io(e.to_string()))?
                .port();
            Err(BindError::InUse {
                requested: port,
                suggested,
            })
        }
        Err(e) => Err(BindError::Io(e.to_string())),
    }
}

/// Build the port-forward API client for a context/namespace. Split out from
/// `serve_pod_forward` so callers (the reconnect loop) can tell "the session
/// established" (this succeeded) apart from "the accept loop ended" (below).
pub async fn connect_pod_api(
    cache: Arc<ClientCache>,
    context: &str,
    namespace: &str,
) -> Result<Api<Pod>, String> {
    let client = cache.get(context).await?;
    Ok(Api::namespaced(client, namespace))
}

/// Accept loop for a bound listener: every inbound local connection opens its
/// own port-forward stream to `pod:remote_port` and is piped bidirectionally.
/// Runs until the listener errors or the spawning task is aborted. Takes the
/// listener by reference so a reconnect loop can keep the same bound local
/// port across attempts and re-accept on it (`TcpListener::accept` only needs
/// `&self`, so no re-bind is required).
pub async fn serve_pod_forward(
    listener: &TcpListener,
    api: Api<Pod>,
    pod: String,
    remote_port: u16,
) -> Result<(), String> {
    loop {
        let (mut local, _peer) = listener.accept().await.map_err(|e| e.to_string())?;
        let api = api.clone();
        let pod = pod.clone();
        tokio::spawn(async move {
            let mut pf = match api.portforward(&pod, &[remote_port]).await {
                Ok(pf) => pf,
                Err(_) => return,
            };
            if let Some(mut upstream) = pf.take_stream(remote_port) {
                let _ = copy_bidirectional(&mut local, &mut upstream).await;
            }
        });
    }
}

/// Resolve a Service to a concrete `(pod, container_port)` to forward to: pick
/// the matching service port, then the first ready pod behind its selector, and
/// map the service's target port onto that pod.
pub async fn resolve_service_target(
    cache: Arc<ClientCache>,
    context: &str,
    namespace: &str,
    service: &str,
    service_port: Option<i32>,
) -> Result<(String, u16), String> {
    let client = cache.get(context).await?;
    let svc_api: Api<Service> = Api::namespaced(client.clone(), namespace);
    let svc = svc_api.get(service).await.map_err(|e| e.to_string())?;
    let spec = svc.spec.ok_or("service has no spec")?;

    let ports = spec.ports.unwrap_or_default();
    let sp = select_service_port(&ports, service_port)
        .ok_or("service has no ports")?
        .clone();

    let selector = spec.selector.unwrap_or_default();
    if selector.is_empty() {
        return Err("service has no selector (headless or external)".into());
    }
    let label = selector
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",");

    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    let pods = pod_api
        .list(&ListParams::default().labels(&label))
        .await
        .map_err(|e| e.to_string())?;
    let pod = pick_ready_pod(&pods.items).ok_or("service has no running pods")?;
    let target = resolve_target_port(&sp, pod).ok_or("could not resolve target port")?;
    let name = pod.metadata.name.clone().unwrap_or_default();
    Ok((name, target))
}

/// Pick the `ServicePort` matching `requested` (by port number), falling back
/// to the first declared port when unmatched or unspecified.
pub fn select_service_port(ports: &[ServicePort], requested: Option<i32>) -> Option<&ServicePort> {
    if let Some(p) = requested {
        if let Some(found) = ports.iter().find(|sp| sp.port == p) {
            return Some(found);
        }
    }
    ports.first()
}

/// Resolve a service port's `targetPort` to a concrete container port on `pod`.
/// Numeric targets pass through; named targets are looked up by container-port
/// name; an absent target defaults to the service port itself.
pub fn resolve_target_port(sp: &ServicePort, pod: &Pod) -> Option<u16> {
    match &sp.target_port {
        Some(IntOrString::Int(n)) => u16::try_from(*n).ok(),
        Some(IntOrString::String(name)) => container_port_by_name(pod, name),
        None => u16::try_from(sp.port).ok(),
    }
}

fn container_port_by_name(pod: &Pod, name: &str) -> Option<u16> {
    let spec = pod.spec.as_ref()?;
    for c in &spec.containers {
        for p in c.ports.iter().flatten() {
            if p.name.as_deref() == Some(name) {
                return u16::try_from(p.container_port).ok();
            }
        }
    }
    None
}

/// First `Running` pod in the list, falling back to the first pod overall.
pub fn pick_ready_pod(pods: &[Pod]) -> Option<&Pod> {
    pods.iter()
        .find(|p| {
            p.status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                == Some("Running")
        })
        .or_else(|| pods.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{Container, ContainerPort, PodSpec, PodStatus};

    fn svc_port(port: i32, target: Option<IntOrString>) -> ServicePort {
        ServicePort {
            port,
            target_port: target,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn bind_local_picks_a_free_port() {
        let listener = bind_local(0).await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        assert!(addr.ip().is_loopback());
        assert!(addr.port() > 0);
    }

    #[tokio::test]
    async fn bind_local_reports_conflict_with_a_free_suggestion() {
        let taken = bind_local(0).await.unwrap();
        let p = taken.local_addr().unwrap().port();
        match bind_local(p).await {
            Err(BindError::InUse { requested, suggested }) => {
                assert_eq!(requested, p);
                assert!(suggested != 0 && suggested != p);
                bind_local(suggested).await.expect("suggested port is free");
            }
            other => panic!("expected InUse, got {other:?}"),
        }
    }

    #[test]
    fn select_service_port_matches_by_number_else_first() {
        let ports = vec![svc_port(80, None), svc_port(443, None)];
        assert_eq!(select_service_port(&ports, Some(443)).unwrap().port, 443);
        // Unmatched / unspecified falls back to the first port.
        assert_eq!(select_service_port(&ports, Some(8080)).unwrap().port, 80);
        assert_eq!(select_service_port(&ports, None).unwrap().port, 80);
        assert!(select_service_port(&[], Some(80)).is_none());
    }

    #[test]
    fn resolve_target_port_handles_numeric_named_and_absent() {
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    ports: Some(vec![ContainerPort {
                        name: Some("http".into()),
                        container_port: 8080,
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        // Numeric target passes through.
        assert_eq!(
            resolve_target_port(&svc_port(80, Some(IntOrString::Int(9000))), &pod),
            Some(9000)
        );
        // Named target resolves against the pod's container ports.
        assert_eq!(
            resolve_target_port(&svc_port(80, Some(IntOrString::String("http".into()))), &pod),
            Some(8080)
        );
        // Unknown named target resolves to nothing.
        assert_eq!(
            resolve_target_port(&svc_port(80, Some(IntOrString::String("grpc".into()))), &pod),
            None
        );
        // Absent target defaults to the service port.
        assert_eq!(resolve_target_port(&svc_port(80, None), &pod), Some(80));
    }

    #[test]
    fn pick_ready_pod_prefers_running() {
        let pending = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("pending".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Pending".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let running = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("running".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let pods = vec![pending, running];
        assert_eq!(
            pick_ready_pod(&pods).unwrap().metadata.name.as_deref(),
            Some("running")
        );
        // With no running pod, the first is used.
        assert_eq!(
            pick_ready_pod(&pods[..1]).unwrap().metadata.name.as_deref(),
            Some("pending")
        );
        assert!(pick_ready_pod(&[]).is_none());
    }
}
