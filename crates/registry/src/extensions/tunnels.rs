//! Pod tunnels for an app's program (#1091): the registry's answer to a
//! sidecar's `host/connect`, which the broker has already held to its shape
//! and rate.
//!
//! Each connect is decided in order: the app (installed, enabled, this
//! revision, this cluster, its manifest valid against its grants, executable),
//! then the binding (declared, a `k8s.connectPod` one, asked the way its scope
//! reads), then the scope, then the pod — named, or the one a Service resolves
//! to now — running and admitted, and only then the cluster's port-forward with
//! the user's own credentials. Every connect that reaches here is recorded,
//! opened or refused; no byte of the tunnel ever is.
//!
//! While open, the app is rechecked and the pod read again every
//! [`PodTiming::monitor`], and every inventory write announced in this process
//! rechecks the app too. The tunnel
//! never moves, not even through a Service: when its pod leaves the scope or
//! stops, it ends. The monitor is the tunnel's `ended` future itself, so it
//! stops when the tunnel is dropped.

use super::pods::{self as scope, KubePods, PodCluster, ScopeError};
use super::streams::{
    pod_still_there, Authority, ExtensionStreams, ExtensionStreamsParts, PodTiming, Record,
};
use serde_json::json;
use srelens_capability::audit::{AuditSink, Source};
use srelens_plugin_host::sidecar::protocol::{code, RpcError};
use srelens_plugin_host::sidecar::{AppIdentity, PodConnector, Tunnel};
use srelens_plugin_host::{ManifestKind, PodScope, POD_CONNECT};
use srelens_sidecar_protocol::HostConnectParams;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::Instant;

/// The registry's [`PodConnector`]: opens a sidecar's tunnels to pods its app
/// may reach, and records each.
pub struct PodTunnels {
    parts: Arc<ExtensionStreamsParts>,
    cluster: Arc<dyn PodCluster>,
    audit: Arc<dyn AuditSink>,
    timing: PodTiming,
    inventory: watch::Receiver<u64>,
}

/// Why a connect is refused: what the sidecar is told, and what the audit
/// record says, which may name more than the app may learn.
struct Refusal {
    told: RpcError,
    why: String,
}

fn refused(why: String) -> Refusal {
    Refusal {
        told: RpcError::new(code::CAPABILITY_FAILED, why.clone()),
        why,
    }
}

fn invalid(why: String) -> Refusal {
    Refusal {
        told: RpcError::new(code::INVALID_PARAMS, why.clone()),
        why,
    }
}

/// The app's authority refusing, as a program reads it: the shared check
/// words it for a view, which a program has none of to refresh.
fn for_program(why: String) -> String {
    let why = why.strip_prefix("handler error: ").unwrap_or(&why);
    why.strip_suffix("; refresh the view")
        .unwrap_or(why)
        .to_owned()
}

/// What a connect learned on its way, for its record: the context it resolved
/// to, the binding's port and, through a Service, the pod's port it reached.
#[derive(Default)]
struct Seen {
    context: Option<String>,
    port: Option<u16>,
    target_port: Option<u16>,
}

impl PodTunnels {
    /// The host's: the cluster through `streams`' client cache, rechecked on
    /// `streams`' inventory writes, recorded to `audit`.
    pub fn new(streams: &ExtensionStreams, audit: Arc<dyn AuditSink>) -> Self {
        let cluster = Arc::new(KubePods(streams.parts().cache.clone()));
        Self::with_cluster(streams, audit, cluster, PodTiming::default())
    }

    /// Over `cluster`, on `timing`'s clock. Test support, and what
    /// [`PodTunnels::new`] builds on.
    pub(crate) fn with_cluster(
        streams: &ExtensionStreams,
        audit: Arc<dyn AuditSink>,
        cluster: Arc<dyn PodCluster>,
        timing: PodTiming,
    ) -> Self {
        Self {
            parts: streams.parts(),
            cluster,
            audit,
            timing,
            inventory: streams.inventory_changes(),
        }
    }

    /// The tunnel `ask` names, for `app`, or why not; what it learned on the
    /// way is left in `seen`, for the record.
    async fn open(
        &self,
        app: &AppIdentity,
        ask: &HostConnectParams,
        seen: &mut Seen,
    ) -> Result<Tunnel, Refusal> {
        // Before the app is checked, so no inventory write after the check
        // goes unseen by the tunnel.
        let mut inventory = self.inventory.clone();
        inventory.borrow_and_update();
        let namespace = &ask.context.namespace;
        let authority = Authority::tunnel(
            self.parts.clone(),
            &app.id,
            app.revision,
            &ask.capability,
            &ask.context.cluster_id,
        );
        let (context, manifest) = authority
            .app()
            .await
            .map_err(|why| refused(for_program(why)))?;
        seen.context = Some(context.clone());
        if manifest.kind != ManifestKind::Executable {
            return Err(refused(format!(
                "App {} is not an executable app; only an app's program opens pod connections",
                app.id
            )));
        }
        let binding = authority.binding(&manifest).map_err(invalid)?;
        seen.port = srelens_plugin_host::forward_port(&binding);
        let port = seen
            .port
            .ok_or_else(|| refused(format!("\"{}\" binds no port", binding.name)))?;
        match (manifest.pod_scope(&binding).map_err(refused)?, &ask.name) {
            (PodScope::Namespaces(_), Some(_)) => {
                return Err(invalid(format!(
                    "\"{}\" reaches pods in the namespaces its permission grants; name no object",
                    binding.name
                )))
            }
            (PodScope::Selected { .. }, None) => {
                return Err(invalid(format!(
                    "\"{}\" reaches the pods of an object; name it",
                    binding.name
                )))
            }
            _ => {}
        }
        // `Ok` a pod, `Err` a Service: whichever the binding connects through.
        let named = match (
            srelens_plugin_host::forward_via_service(&binding),
            ask.pod.as_deref(),
            ask.service.as_deref(),
        ) {
            (false, Some(pod), None) => Ok(pod),
            (true, None, Some(service)) => Err(service),
            (true, _, _) => {
                return Err(invalid(format!(
                    "\"{}\" connects through a Service; name the Service, not a pod",
                    binding.name
                )))
            }
            (false, _, _) => {
                return Err(invalid(format!(
                    "\"{}\" connects to a pod; name the pod, not a Service",
                    binding.name
                )))
            }
        };
        let cluster = self.cluster.as_ref();
        let scope = scope::scope(
            cluster,
            &self.parts.core,
            &context,
            &manifest,
            &binding,
            namespace,
            ask.name.as_deref(),
        )
        .await
        .map_err(|why| refused(why.into()))?;
        let (pod, remote) = match named {
            Ok(pod) => {
                // Not `scope::admitted_pod`: it tells a pod that does not exist
                // from one outside the scope, and an app may learn neither.
                let facts = match cluster.pod(&context, &scope.namespace, pod).await {
                    Ok(Some(facts)) if scope.admits(&facts) => facts,
                    Ok(found) => {
                        let told = scope.refusal(&app.id, pod);
                        let why = match found {
                            None => format!("{told} (it does not exist)"),
                            Some(_) => told.clone(),
                        };
                        return Err(Refusal {
                            told: RpcError::new(code::CAPABILITY_FAILED, told),
                            why,
                        });
                    }
                    Err(error) => return Err(refused(ScopeError::from(error).into())),
                };
                if !facts.running() {
                    return Err(refused(format!("Pod {namespace}/{pod} is not running")));
                }
                (facts, port)
            }
            Err(service) => {
                let (facts, remote) =
                    scope::service_target(cluster, &context, &scope, &app.id, service, port)
                        .await
                        .map_err(|why| refused(why.into()))?;
                seen.target_port = Some(remote);
                (facts, remote)
            }
        };
        let upstream = self
            .cluster
            .connect(&context, &scope.namespace, &pod.name, remote)
            .await
            .map_err(|why| {
                refused(format!(
                    "The cluster refused the connection to pod {namespace}/{}: {why}",
                    pod.name
                ))
            })?;
        let watch = Monitor {
            cluster: self.cluster.clone(),
            context,
            scope,
            pod: pod.name.clone(),
            authority,
            timing: self.timing,
            inventory,
        };
        Ok(Tunnel {
            io: Box::new(upstream),
            pod: pod.name,
            ended: Box::pin(watch.run()),
        })
    }
}

impl PodConnector for PodTunnels {
    fn connect<'a>(
        &'a self,
        app: &'a AppIdentity,
        ask: &'a HostConnectParams,
    ) -> Pin<Box<dyn Future<Output = Result<Tunnel, RpcError>> + Send + 'a>> {
        Box::pin(async move {
            let mut seen = Seen::default();
            let opened = self.open(app, ask, &mut seen).await;
            // Written as the answer is, with no wait between: a connect the
            // sidecar cancels before this never opened, and one that opened
            // is recorded.
            let record = Record {
                audit: self.audit.as_ref(),
                source: Source::Sidecar,
                tool: POD_CONNECT,
                app: &app.id,
                revision: app.revision,
                // The pinned cluster once it resolved, as a view's forward
                // records; until then the name the sidecar sent.
                context: seen.context.as_deref().unwrap_or(&ask.context.cluster_id),
                namespace: &ask.context.namespace,
            };
            let args = |pod: Option<&str>| {
                let mut args = json!({"capability": ask.capability, "pod": pod, "service": ask.service, "port": seen.port});
                if let Some(target) = seen.target_port {
                    args["targetPort"] = json!(target);
                }
                args
            };
            match opened {
                Ok(tunnel) => {
                    record.write(Some(&tunnel.pod), args(Some(&tunnel.pod)), "auto", None);
                    Ok(tunnel)
                }
                Err(refusal) => {
                    let pod = ask.pod.as_deref();
                    record.write(pod, args(pod), "auto", Some(&refusal.why));
                    Err(refusal.told)
                }
            }
        })
    }
}

/// What an open tunnel is held to.
struct Monitor {
    cluster: Arc<dyn PodCluster>,
    context: String,
    scope: scope::Scope,
    pod: String,
    authority: Authority,
    timing: PodTiming,
    inventory: watch::Receiver<u64>,
}

impl Monitor {
    /// Resolves, naming the pod, when the tunnel must end; until then it
    /// rechecks the app and reads the pod each tick, and rechecks the app on
    /// each inventory write announced in this process. The tick's recheck reads
    /// the saved inventory, so a change another process made (a headless
    /// `--mcp-stdio` beside the app's window) ends the tunnel too.
    async fn run(mut self) -> String {
        let mut tick =
            tokio::time::interval_at(Instant::now() + self.timing.monitor, self.timing.monitor);
        let why = loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(why) = self.authority.check().await {
                        break for_program(why);
                    }
                    match pod_still_there(self.cluster.as_ref(), &self.context, &self.scope, &self.authority.id, &self.pod).await {
                        Ok(true) => {}
                        Ok(false) => break format!("Pod {} is no longer running", self.pod),
                        Err(why) => break why,
                    }
                }
                changed = self.inventory.changed() => {
                    if changed.is_err() {
                        break "srelens stopped serving apps".to_owned();
                    }
                    if let Err(why) = self.authority.check().await {
                        break for_program(why);
                    }
                }
            }
        };
        format!(
            "The connection to pod {}/{} ended: {why}",
            self.scope.namespace, self.pod
        )
    }
}
