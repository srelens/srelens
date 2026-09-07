//! Local port-forwarding via kube-rs. Binds a loopback TCP listener and pipes
//! each inbound connection through its own port-forward stream to a pod —
//! Tauri-agnostic so the listener/stream plumbing stays reusable and testable.

use std::fmt;
use std::io::ErrorKind;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use k8s_openapi::api::core::v1::{Pod, Service, ServicePort};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::ListParams;
use kube::Api;
use tokio::io::{copy_bidirectional, AsyncRead, AsyncWrite, ReadBuf};
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

/// Bytes that have crossed one forward, counted as they pass rather than
/// when a connection closes. `copy_bidirectional` reports its totals too, but
/// only once the connection is over: a tunnel holding one long-lived
/// connection — a database session, a websocket — would report nothing for as
/// long as it stays open, so a busy tunnel would read as idle for hours.
///
/// The count belongs to the *forward*, not to a connection: `accept_loop`
/// spawns a task per accepted connection, so the caller owns the `Arc` and a
/// tunnel's total survives any one connection closing.
///
/// `Relaxed` is deliberate. A reporter task in another crate reads this while
/// the pipes write to it, but it is a monotonic statistic rather than a
/// synchronisation point — nothing is published through it, so no ordering
/// beyond the atomicity of the add itself is needed.
#[derive(Debug, Default)]
pub struct TrafficCounter(AtomicU64);

impl TrafficCounter {
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn total(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A stream that adds whatever each poll actually moved to a `TrafficCounter`.
///
/// Only the *local* side is wrapped, and that counts both directions: every
/// byte the client sends arrives as a read on it, and every byte the client
/// receives leaves as a write to it. Wrapping the upstream too would double
/// the number.
struct Counted<S> {
    inner: S,
    counter: Arc<TrafficCounter>,
}

impl<S> Counted<S> {
    fn new(inner: S, counter: Arc<TrafficCounter>) -> Self {
        Self { inner, counter }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let res = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(res, Poll::Ready(Ok(()))) {
            this.counter
                .add(buf.filled().len().saturating_sub(before) as u64);
        }
        res
    }
}

// `is_write_vectored` is left at its `false` default on purpose: reporting
// true would route writes through `poll_write_vectored`, and a default
// implementation of that would move bytes this wrapper never sees.
impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = res {
            this.counter.add(n as u64);
        }
        res
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Accept loop for a bound listener: every inbound local connection opens its
/// own port-forward stream to `pod:remote_port` and is piped bidirectionally.
/// Runs until the listener errors, the target pod monitor detects the pod is
/// gone (see below), or the spawning task is aborted. Takes the listener by
/// reference so a reconnect loop can keep the same bound local port across
/// attempts and re-accept on it (`TcpListener::accept` only needs `&self`, so
/// no re-bind is required).
///
/// Per-connection port-forward failures (e.g. a single `portforward` call
/// failing) are swallowed inside the detached per-connection task and do not
/// end the session by themselves — only the accept loop erroring or the
/// target-pod monitor detecting pod loss does. This matters for Service
/// targets: without the monitor, a deleted target pod would leave the accept
/// loop parked in `listener.accept()` forever, so the outer reconnect loop
/// (which re-resolves a Service to its current backing pod on every attempt)
/// would never get a chance to run again.
pub async fn serve_pod_forward(
    listener: &TcpListener,
    api: Api<Pod>,
    pod: String,
    remote_port: u16,
    traffic: Arc<TrafficCounter>,
) -> Result<(), String> {
    tokio::select! {
        res = accept_loop(listener, api.clone(), pod.clone(), remote_port, traffic) => res,
        res = monitor_target_pod(api, pod) => res,
    }
}

async fn accept_loop(
    listener: &TcpListener,
    api: Api<Pod>,
    pod: String,
    remote_port: u16,
    traffic: Arc<TrafficCounter>,
) -> Result<(), String> {
    loop {
        let (local, _peer) = listener.accept().await.map_err(|e| e.to_string())?;
        let api = api.clone();
        let pod = pod.clone();
        // Cloned per connection so every connection of this forward adds to
        // the one total the caller holds.
        let traffic = traffic.clone();
        tokio::spawn(async move {
            let mut pf = match api.portforward(&pod, &[remote_port]).await {
                Ok(pf) => pf,
                Err(_) => return,
            };
            if let Some(mut upstream) = pf.take_stream(remote_port) {
                let mut local = Counted::new(local, traffic);
                let _ = copy_bidirectional(&mut local, &mut upstream).await;
            }
        });
    }
}

/// How often the target-pod monitor polls the cluster for the forwarded
/// pod's liveness.
const POD_MONITOR_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Poll `pod` on a short interval and return an error once it's gone (deleted
/// or no longer `Running`), ending the `serve_pod_forward` session so the
/// outer reconnect loop gets a chance to re-resolve (a Service target picks
/// up its replacement pod). A transient API error while polling (e.g. a
/// momentary network blip) is not treated as "gone" — only an authoritative
/// answer from the API (not found, or an object that fails `pod_is_gone`)
/// ends the session, so we don't tear down a healthy forward on a hiccup.
///
/// The first check is skipped by `POD_MONITOR_INTERVAL` (rather than firing
/// immediately) since a freshly resolved target may still be transitioning
/// into `Running` right as this session starts.
async fn monitor_target_pod(api: Api<Pod>, pod: String) -> Result<(), String> {
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + POD_MONITOR_INTERVAL,
        POD_MONITOR_INTERVAL,
    );
    loop {
        interval.tick().await;
        match api.get_opt(&pod).await {
            Ok(Some(p)) if !pod_is_gone(&p) => continue,
            Ok(_) => return Err(format!("target pod {pod} is no longer available")),
            Err(_) => continue,
        }
    }
}

/// Whether `pod` should be treated as no longer usable as a forward target:
/// it's mid-deletion (`deletionTimestamp` set) or its phase isn't `Running`
/// (e.g. `Failed`, `Succeeded`, or missing status entirely).
pub fn pod_is_gone(pod: &Pod) -> bool {
    if pod.metadata.deletion_timestamp.is_some() {
        return true;
    }
    pod.status.as_ref().and_then(|s| s.phase.as_deref()) != Some("Running")
}

/// One-shot readiness probe: true only when the target pod currently exists and
/// is Running (not terminating). A missing pod or an API error reads as
/// not-ready. Used to decide whether a reconnect attempt actually *established*
/// — building an `Api<Pod>` handle does no I/O and always "succeeds", so without
/// this probe a permanently-dead target would reconnect forever and never reach
/// the give-up threshold. Mirrors the monitor's own liveness check.
pub async fn pod_is_ready(api: &Api<Pod>, pod: &str) -> bool {
    matches!(api.get_opt(pod).await, Ok(Some(p)) if !pod_is_gone(&p))
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::task::JoinHandle;

    /// A loopback echo server, standing in for whatever listens on the far
    /// side of the tunnel: it sends back every byte it is given, so a client
    /// that writes N and reads N back has moved 2N across the local stream.
    async fn echo_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = sock.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        addr
    }

    /// The per-connection body of `accept_loop`, with a real echo upstream in
    /// place of a port-forward stream: an accepted local connection wrapped in
    /// `Counted` and piped bidirectionally. Returns the client end and the
    /// pipe's handle so a test can decide whether to close the connection.
    async fn counted_pipe(counter: Arc<TrafficCounter>) -> (TcpStream, JoinHandle<()>) {
        let upstream_addr = echo_server().await;
        let listener = bind_local(0).await.expect("bind");
        let local_addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(local_addr).await.expect("connect");
        let (local, _peer) = listener.accept().await.expect("accept");
        let mut upstream = TcpStream::connect(upstream_addr).await.expect("upstream");
        let handle = tokio::spawn(async move {
            let mut local = Counted::new(local, counter);
            let _ = copy_bidirectional(&mut local, &mut upstream).await;
        });
        (client, handle)
    }

    /// Write `payload`, read the echo back, then close: the whole exchange,
    /// start to finish. Returns the number of bytes that came back.
    async fn echo_through_counted_stream(counter: Arc<TrafficCounter>, payload: &[u8]) -> usize {
        let (mut client, handle) = counted_pipe(counter).await;
        client.write_all(payload).await.expect("write");
        let mut back = vec![0u8; payload.len()];
        client.read_exact(&mut back).await.expect("read back");
        assert_eq!(back, payload, "the echo came back intact");
        drop(client);
        let _ = handle.await;
        back.len()
    }

    /// Write `payload` and read the echo back, but leave everything open —
    /// the client and the pipe are handed back alive.
    async fn write_through_counted_stream(
        counter: Arc<TrafficCounter>,
        payload: &[u8],
    ) -> (TcpStream, JoinHandle<()>) {
        let (mut client, handle) = counted_pipe(counter).await;
        client.write_all(payload).await.expect("write");
        let mut back = vec![0u8; payload.len()];
        client.read_exact(&mut back).await.expect("read back");
        (client, handle)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn counts_bytes_in_both_directions() {
        // An echo upstream, so a client that writes N and reads N back moves
        // 2N: counting only the local side sees both directions.
        let counter = Arc::new(TrafficCounter::default());
        let moved = echo_through_counted_stream(counter.clone(), b"hello world").await;
        assert_eq!(moved, 11, "wrote 11 bytes");
        assert_eq!(counter.total(), 22, "11 up and 11 back down");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_connection_still_open_has_already_counted_what_it_moved() {
        // The property the whole task exists for: a long-lived connection must
        // report before it closes, or a database tunnel reads 0 for hours.
        // `copy_bidirectional`'s return value cannot satisfy this — it only
        // arrives once the connection is over.
        let counter = Arc::new(TrafficCounter::default());
        let (_client, held) = write_through_counted_stream(counter.clone(), b"first").await;
        assert!(!held.is_finished(), "the connection is still open");
        assert!(
            counter.total() >= 5,
            "counted while the connection is still open, saw {}",
            counter.total()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_counter_spans_every_connection_of_a_forward() {
        // Counters belong to a forward, not to a connection: `accept_loop`
        // spawns per connection, so a tunnel's total must not reset when one
        // of its connections closes.
        let counter = Arc::new(TrafficCounter::default());
        echo_through_counted_stream(counter.clone(), b"one").await;
        let after_first = counter.total();
        echo_through_counted_stream(counter.clone(), b"two").await;
        assert_eq!(after_first, 6, "3 up and 3 back down");
        assert_eq!(counter.total(), 12, "the second connection added to the first");
    }

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

    fn pod_with(phase: Option<&str>, deleted: bool) -> Pod {
        let deletion_timestamp = deleted.then(|| {
            k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(k8s_openapi::jiff::Timestamp::now())
        });
        Pod {
            metadata: kube::core::ObjectMeta {
                deletion_timestamp,
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: phase.map(String::from),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn pod_is_gone_true_when_running_but_marked_for_deletion() {
        // A pod mid-termination is still (briefly) `Running` in status, but
        // the deletionTimestamp means it's on its way out — treat it as gone
        // so the forward doesn't keep serving through a terminating pod.
        assert!(pod_is_gone(&pod_with(Some("Running"), true)));
    }

    #[test]
    fn pod_is_gone_true_when_phase_is_not_running() {
        assert!(pod_is_gone(&pod_with(Some("Pending"), false)));
        assert!(pod_is_gone(&pod_with(Some("Failed"), false)));
        assert!(pod_is_gone(&pod_with(Some("Succeeded"), false)));
        assert!(pod_is_gone(&pod_with(None, false)));
    }

    #[test]
    fn pod_is_gone_false_when_running_and_not_deleted() {
        assert!(!pod_is_gone(&pod_with(Some("Running"), false)));
    }
}
