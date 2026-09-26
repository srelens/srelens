//! The pod sources (#567): `logs`, `exec` and `portForward`, on the same wire
//! as `read` and `watch`.
//!
//! Every open is authorized as a read is (installed, enabled, this revision,
//! this cluster, the manifest valid against its grants), then held to the
//! binding's pod scope: the object the view names is read, with its own label
//! selector, and the pod the view names must be one it selects (or in a
//! namespace the permission grants). What runs is the manifest's: an exec
//! binding's command, a port-forward's port. A session ends with its view,
//! like every stream; a port-forward's listener and every connection through
//! it go with it.
//!
//! Frames are batched, at most one per [`PodTiming::batch`] a stream, so a
//! chatty container cannot take the app past its message rate.

use super::super::pods::{
    self as scope, ExecAsk, LogEvent, LogsAsk, PodCluster, PodsIn, PodsOut, Scope,
};
use super::{ExtensionStreams, OpenStreamIn, OpenStreamOut, StreamSourceIn};
use serde::Deserialize;
use serde_json::{json, Value};
use srelens_capability::audit::{AppRef, AuditRecord, AuditSink, Source};
use srelens_capability::audit::{OUTCOME_OK, OUTCOME_REJECTED};
use srelens_kube::app_pods::{Output, PodFacts};
use srelens_plugin_host::{Binding, Manifest, POD_EXEC, POD_FORWARD, POD_LOGS};
use srelens_streams::app::{StreamEmitter, StreamOwner, StreamWindow};
use srelens_streams::EventSink;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::Instant;

/// Lines of history a log stream starts with when the view names none.
const DEFAULT_TAIL_LINES: i64 = 200;
/// The most history a log stream may ask for.
pub const MAX_TAIL_LINES: i64 = 5000;
/// The most lines one `lines` frame carries.
pub const MAX_LINES_PER_FRAME: usize = 500;
/// Lines held for the next frames at most; beyond it, lines are dropped and counted.
pub const MAX_PENDING_LINES: usize = 5000;
/// Bytes of line text held for the next frames at most; beyond it, likewise.
pub const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
/// Bytes of line text one `lines` frame carries at most (one longer line goes alone).
pub const MAX_FRAME_BYTES: usize = 512 * 1024;
/// One log line is cut after this many bytes, on a character boundary, and marked
/// `truncated`.
pub const MAX_LINE_BYTES: usize = 16 * 1024;
/// The most output one exec session may write, stdout and stderr together.
pub const MAX_EXEC_OUTPUT: usize = 1024 * 1024;

/// The pod sources' clock.
#[derive(Clone, Copy, Debug)]
pub struct PodTiming {
    /// How long lines or output are gathered before they are sent as one frame.
    pub batch: Duration,
    /// How long a lost log stream waits before it follows again.
    pub reconnect: Duration,
    /// How long one exec command may run.
    pub exec_limit: Duration,
    /// How often a port-forward checks its pod is still there and in scope.
    pub monitor: Duration,
}

impl Default for PodTiming {
    fn default() -> Self {
        Self {
            batch: Duration::from_millis(250),
            reconnect: Duration::from_secs(2),
            exec_limit: Duration::from_secs(300),
            monitor: Duration::from_secs(2),
        }
    }
}

/// What the view confirmed for an exec session: the host confirmation named
/// exactly this pod, container and command, and the person approved it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecConfirmed {
    pub pod: String,
    pub container: String,
    pub command: Vec<String>,
}

/// One pod source's authority, rechecked whenever the inventory changes.
struct Authority {
    streams: Arc<ExtensionStreamsParts>,
    id: String,
    revision: u64,
    capability: String,
    context: String,
    /// The one pod target the binding must have; `None` for any of them.
    target: Option<&'static str>,
}

/// What a pod source needs of its stream manager, shared with the source task.
pub(super) struct ExtensionStreamsParts {
    pub path: super::Store,
    pub core: Arc<srelens_capability::Registry>,
    pub cache: Arc<srelens_kube::client_cache::ClientCache>,
}

impl Authority {
    /// The app may still reach this binding: installed, enabled, this
    /// revision, this cluster, the manifest valid against its grants, and the
    /// binding one of `target`'s. Answers the pinned context, the manifest and
    /// the binding.
    async fn check(&self) -> Result<(String, Manifest, Binding), String> {
        if self.context.trim().is_empty() {
            return Err("An explicit cluster context is required".into());
        }
        let (state, index, context) = super::resolver_app(
            self.streams.path.clone(),
            &self.streams.core,
            &self.streams.cache,
            &self.id,
            self.revision,
            self.context.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let manifest = state.plugins[index].manifest.clone();
        let binding = manifest
            .capabilities
            .iter()
            .find(|binding| binding.name == self.capability)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "App {} declares no binding named \"{}\"",
                    self.id, self.capability
                )
            })?;
        match self.target {
            Some(target) if binding.target != target => {
                return Err(format!(
                    "\"{}\" is a {} binding, not a {target} one",
                    binding.name, binding.target
                ))
            }
            None if !srelens_plugin_host::is_pod_target(&binding.target) => {
                return Err(format!(
                    "\"{}\" is a {} binding, not a pod binding",
                    binding.name, binding.target
                ))
            }
            _ => {}
        }
        Ok((context, manifest, binding))
    }
}

/// One audit record for a pod session (#555): who, through which app, on
/// which cluster and pod, and whether it started.
struct Record<'a> {
    audit: &'a dyn AuditSink,
    tool: &'static str,
    app: &'a str,
    revision: u64,
    context: &'a str,
    namespace: &'a str,
}

impl Record<'_> {
    fn write(&self, pod: Option<&str>, args: Value, decision: &'static str, refused: Option<&str>) {
        self.audit.record(AuditRecord {
            source: Source::Ui,
            tool: self.tool.to_owned(),
            args,
            app: Some(AppRef {
                id: self.app.to_owned(),
                revision: self.revision,
            }),
            cluster: Some(self.context.to_owned()),
            resource: Some(match pod {
                Some(pod) => format!("{}/{pod}", self.namespace),
                None => self.namespace.to_owned(),
            }),
            decision,
            outcome: if refused.is_some() {
                OUTCOME_REJECTED
            } else {
                OUTCOME_OK
            },
            error: refused.map(str::to_owned),
        });
    }
}

/// The names a pod source's request carries, held to Kubernetes' own syntax
/// before anything else reads them — the audit trail among them, so nothing a
/// view sends reaches it unbounded.
fn check_names(
    namespace: &str,
    pod: Option<&str>,
    container: Option<&str>,
    service: Option<&str>,
) -> Result<(), String> {
    if !super::super::cards::namespace_name(namespace) {
        return Err("Name the namespace the pods are in: a Kubernetes namespace name".into());
    }
    if pod.is_some_and(|pod| !scope::pod_name(pod)) {
        return Err("Name the pod: a Kubernetes pod name".into());
    }
    if container.is_some_and(|container| !srelens_plugin_host::namespace_name(container)) {
        return Err("Name the container: 1–63 lowercase letters, digits and -".into());
    }
    if service.is_some_and(|service| !srelens_plugin_host::namespace_name(service)) {
        return Err("Name the Service: 1–63 lowercase letters, digits and -".into());
    }
    Ok(())
}

/// Cut one log line to [`MAX_LINE_BYTES`], on a character boundary, saying so.
fn line_frame(source: &str, mut line: String) -> Value {
    if line.len() > MAX_LINE_BYTES {
        let mut end = MAX_LINE_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
        json!({"source": source, "line": line, "truncated": true})
    } else {
        json!({"source": source, "line": line})
    }
}

impl ExtensionStreams {
    /// The parts a pod source keeps, so the task outlives no manager.
    fn parts(&self) -> Arc<ExtensionStreamsParts> {
        Arc::new(ExtensionStreamsParts {
            path: self.path.clone(),
            core: self.core.clone(),
            cache: self.cache.clone(),
        })
    }

    fn authority(&self, input: &OpenStreamIn, capability: &str, target: &'static str) -> Authority {
        Authority {
            streams: self.parts(),
            id: input.id.clone(),
            revision: input.revision,
            capability: capability.to_owned(),
            context: input.context.clone(),
            target: Some(target),
        }
    }

    /// `extensions.pods`: the pods (and, for a forward through a Service, the
    /// Services) one pod binding may reach now, for a view to offer. Held to
    /// the same authority and scope an open is.
    pub(super) async fn pod_targets(&self, input: PodsIn) -> Result<PodsOut, String> {
        let authority = Authority {
            streams: self.parts(),
            id: input.id.clone(),
            revision: input.revision,
            capability: input.capability.clone(),
            context: input.context.clone(),
            target: None,
        };
        let (context, manifest, binding) = authority.check().await?;
        let cluster = self.cluster();
        let scope = scope::scope(
            cluster.as_ref(),
            &self.core,
            &context,
            &manifest,
            &binding,
            &input.namespace,
            input.name.as_deref(),
        )
        .await?;
        scope::targets(cluster.as_ref(), &context, &binding, &scope).await
    }

    fn pod_owner(input: &OpenStreamIn, window: Option<StreamWindow>) -> StreamOwner {
        StreamOwner {
            app: input.id.clone(),
            revision: input.revision,
            view: input.view.clone(),
            window,
        }
    }

    /// Open a `logs` source: follow one container of one pod in scope.
    pub(super) async fn open_logs(
        &self,
        sink: Arc<dyn EventSink>,
        window: Option<StreamWindow>,
        input: OpenStreamIn,
    ) -> Result<OpenStreamOut, String> {
        let StreamSourceIn::Logs {
            capability,
            name,
            pod,
            container,
            tail_lines,
            since_seconds,
            timestamps,
        } = &input.source
        else {
            unreachable!("open_logs is called for a logs source");
        };
        let tail_lines = tail_lines.unwrap_or(DEFAULT_TAIL_LINES);
        if !(0..=MAX_TAIL_LINES).contains(&tail_lines) {
            return Err(format!(
                "A log stream starts with 0–{MAX_TAIL_LINES} lines of history, not {tail_lines}"
            ));
        }
        if since_seconds.is_some_and(|since| since < 1) {
            return Err("sinceSeconds is a positive number of seconds".into());
        }
        check_names(&input.namespace, Some(pod), container.as_deref(), None)?;
        let authority = self.authority(&input, capability, POD_LOGS);
        let (context, manifest, binding) = authority.check().await?;
        let cluster = self.cluster();
        let scope = scope::scope(
            cluster.as_ref(),
            &self.core,
            &context,
            &manifest,
            &binding,
            &input.namespace,
            name.as_deref(),
        )
        .await?;
        let facts = scope::admitted_pod(cluster.as_ref(), &context, &scope, &input.id, pod).await?;
        let container = scope::container(&binding, &facts, container.as_deref())?;
        let follow = LogFollow {
            ask: LogsAsk {
                context,
                namespace: scope.namespace.clone(),
                pod: facts.name.clone(),
                container: container.clone(),
                tail_lines,
                since_seconds: *since_seconds,
                timestamps: *timestamps,
            },
            source: format!("{}/{container}", facts.name),
            object: name.clone(),
            manifest,
            binding,
            scope,
            cluster,
            core: self.core.clone(),
            authority,
            timing: *self.pod_timing.lock().unwrap(),
            inventory: self.inventory.subscribe(),
        };
        let stream = self
            .streams
            .open(
                Self::pod_owner(&input, window),
                "logs",
                sink,
                input.channel.clone(),
                move |tx| follow.run(tx),
            )
            .map_err(|e| e.to_string())?;
        Ok(OpenStreamOut {
            stream,
            channel: input.channel,
        })
    }

    /// Open an `exec` source: run the binding's command once in one pod in
    /// scope, after the view's host confirmation named exactly that session.
    /// Every session that reached the binding is recorded, refused or not.
    pub(super) async fn open_exec(
        &self,
        sink: Arc<dyn EventSink>,
        window: Option<StreamWindow>,
        audit: Arc<dyn AuditSink>,
        input: OpenStreamIn,
    ) -> Result<OpenStreamOut, String> {
        let StreamSourceIn::Exec {
            capability,
            name,
            pod,
            container,
            confirmed,
        } = &input.source
        else {
            unreachable!("open_exec is called for an exec source");
        };
        check_names(&input.namespace, Some(pod), container.as_deref(), None)?;
        let authority = self.authority(&input, capability, POD_EXEC);
        let (context, manifest, binding) = authority.check().await?;
        let record = Record {
            audit: audit.as_ref(),
            tool: POD_EXEC,
            app: &input.id,
            revision: input.revision,
            context: &context,
            namespace: &input.namespace,
        };
        let args = |container: Option<&str>, command: &[String]| json!({"capability": capability, "pod": pod, "container": container, "command": command});
        let command = manifest.exec_command(&binding)?;
        // Refused before anything runs; recorded with why, and whether a person
        // had approved what was asked. A confirmation that names another session
        // approved nothing this one would run.
        let approved = if confirmed.is_some() {
            "approved"
        } else {
            "denied"
        };
        let refuse = |why: String, container: Option<&str>, decision: &'static str| {
            record.write(Some(pod), args(container, &command), decision, Some(&why));
            why
        };
        let cluster = self.cluster();
        let scope = scope::scope(
            cluster.as_ref(),
            &self.core,
            &context,
            &manifest,
            &binding,
            &input.namespace,
            name.as_deref(),
        )
        .await
        .map_err(|why| refuse(why, container.as_deref(), approved))?;
        let facts = scope::admitted_pod(cluster.as_ref(), &context, &scope, &input.id, pod)
            .await
            .map_err(|why| refuse(why, container.as_deref(), approved))?;
        let container = scope::container(&binding, &facts, container.as_deref())
            .map_err(|why| refuse(why, container.as_deref(), approved))?;
        let asked = ExecConfirmed {
            pod: facts.name.clone(),
            container: container.clone(),
            command: command.clone(),
        };
        match confirmed {
            None => {
                return Err(refuse(
                    format!(
                        "Running \"{}\" needs the host confirmation naming pod {}, container {container} and its command; nothing ran",
                        binding.name, facts.name
                    ),
                    Some(&container),
                    "denied",
                ))
            }
            Some(confirmed) if *confirmed != asked => {
                return Err(refuse(
                    format!(
                        "The confirmation named pod {}, container {} and command {:?}, but this session would run {:?} in pod {}, container {container}; nothing ran",
                        confirmed.pod, confirmed.container, confirmed.command, command, facts.name
                    ),
                    Some(&container),
                    "denied",
                ))
            }
            Some(_) => {}
        }
        let run = ExecRun {
            ask: ExecAsk {
                context: context.clone(),
                namespace: scope.namespace.clone(),
                pod: facts.name.clone(),
                container: container.clone(),
                command: command.clone(),
            },
            cluster,
            authority,
            timing: *self.pod_timing.lock().unwrap(),
            inventory: self.inventory.subscribe(),
        };
        let opened = self.streams.open(
            Self::pod_owner(&input, window),
            "exec",
            sink,
            input.channel.clone(),
            move |tx| run.run(tx),
        );
        match opened {
            Ok(stream) => {
                record.write(
                    Some(pod),
                    args(Some(&container), &command),
                    "approved",
                    None,
                );
                Ok(OpenStreamOut {
                    stream,
                    channel: input.channel,
                })
            }
            Err(e) => Err(refuse(e.to_string(), Some(&container), "approved")),
        }
    }

    /// Open a `portForward` source: listen on a port of this computer the
    /// host picks, and forward each connection to the binding's port on one
    /// pod in scope — named directly, or through a Service that selects it.
    pub(super) async fn open_forward(
        &self,
        sink: Arc<dyn EventSink>,
        window: Option<StreamWindow>,
        audit: Arc<dyn AuditSink>,
        input: OpenStreamIn,
    ) -> Result<OpenStreamOut, String> {
        let StreamSourceIn::PortForward {
            capability,
            name,
            pod,
            service,
        } = &input.source
        else {
            unreachable!("open_forward is called for a portForward source");
        };
        check_names(&input.namespace, pod.as_deref(), None, service.as_deref())?;
        let authority = self.authority(&input, capability, POD_FORWARD);
        let (context, manifest, binding) = authority.check().await?;
        let record = Record {
            audit: audit.as_ref(),
            tool: POD_FORWARD,
            app: &input.id,
            revision: input.revision,
            context: &context,
            namespace: &input.namespace,
        };
        let port = srelens_plugin_host::forward_port(&binding)
            .ok_or_else(|| format!("\"{}\" binds no port", binding.name))?;
        let args = |target: Option<&str>| json!({"capability": capability, "pod": target, "service": service, "port": port});
        let refuse = |why: String| {
            record.write(pod.as_deref(), args(pod.as_deref()), "auto", Some(&why));
            why
        };
        let cluster = self.cluster();
        let scope = scope::scope(
            cluster.as_ref(),
            &self.core,
            &context,
            &manifest,
            &binding,
            &input.namespace,
            name.as_deref(),
        )
        .await
        .map_err(refuse)?;
        let via_service = srelens_plugin_host::forward_via_service(&binding);
        let (facts, remote) = match (via_service, pod, service) {
            (false, Some(pod), None) => {
                let facts = scope::admitted_pod(cluster.as_ref(), &context, &scope, &input.id, pod)
                    .await
                    .map_err(refuse)?;
                if !facts.running() {
                    return Err(refuse(format!("Pod {pod} is not running")));
                }
                (facts, port)
            }
            (true, None, Some(service)) => {
                scope::service_target(cluster.as_ref(), &context, &scope, &input.id, service, port)
                    .await
                    .map_err(refuse)?
            }
            (false, _, Some(_)) => {
                return Err(refuse(format!(
                    "\"{}\" forwards to a pod, not through a Service",
                    binding.name
                )))
            }
            (true, Some(_), _) => {
                return Err(refuse(format!(
                    "\"{}\" forwards through a Service; name the Service",
                    binding.name
                )))
            }
            (false, None, None) => return Err(refuse("Name the pod to forward to".into())),
            (true, None, None) => return Err(refuse("Name the Service to forward through".into())),
        };
        // Bound here, so a port the host cannot open refuses the open instead
        // of failing its first frame. Port 0: the host picks, never the app.
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| refuse(format!("The host could not open a local port: {e}")))?;
        let local = listener
            .local_addr()
            .map_err(|e| refuse(e.to_string()))?
            .port();
        let forward = Forward {
            listener,
            local,
            context: context.clone(),
            pod: facts,
            remote,
            port,
            service: service.clone(),
            object: name.clone(),
            manifest,
            binding,
            scope,
            cluster,
            core: self.core.clone(),
            app: input.id.clone(),
            authority,
            timing: *self.pod_timing.lock().unwrap(),
            inventory: self.inventory.subscribe(),
        };
        let target = forward.pod.name.clone();
        let opened = self.streams.open(
            Self::pod_owner(&input, window),
            "portForward",
            sink,
            input.channel.clone(),
            move |tx| forward.run(tx),
        );
        match opened {
            Ok(stream) => {
                let mut args = args(Some(&target));
                args["localPort"] = json!(local);
                record.write(Some(&target), args, "auto", None);
                Ok(OpenStreamOut {
                    stream,
                    channel: input.channel,
                })
            }
            Err(e) => Err(refuse(e.to_string())),
        }
    }
}

/// Why a source stops sending: its stream ended, so it returns quietly.
struct Ended;

fn send(tx: &StreamEmitter, frame: Value) -> Result<(), Ended> {
    tx.data(frame).map_err(|_| Ended)
}

/// One `logs` stream's source.
struct LogFollow {
    ask: LogsAsk,
    /// `pod/container`, on every line and status.
    source: String,
    object: Option<String>,
    manifest: Manifest,
    binding: Binding,
    scope: Scope,
    cluster: Arc<dyn PodCluster>,
    core: Arc<srelens_capability::Registry>,
    authority: Authority,
    timing: PodTiming,
    inventory: tokio::sync::watch::Receiver<u64>,
}

/// Lines waiting for the next frame, bounded by count and by bytes of text.
#[derive(Default)]
struct Pending {
    lines: Vec<Value>,
    /// Bytes of line text in `lines`.
    bytes: usize,
    dropped: u64,
}

/// Bytes of text one line frame carries.
fn text_bytes(line: &Value) -> usize {
    line["line"].as_str().map_or(0, str::len)
}

impl Pending {
    fn push(&mut self, frame: Value) {
        let size = text_bytes(&frame);
        if self.lines.len() >= MAX_PENDING_LINES || self.bytes + size > MAX_PENDING_BYTES {
            self.dropped += 1;
        } else {
            self.bytes += size;
            self.lines.push(frame);
        }
    }

    /// One `lines` frame of at most [`MAX_LINES_PER_FRAME`] lines and
    /// [`MAX_FRAME_BYTES`] of text, or `None` when nothing is waiting.
    fn frame(&mut self) -> Option<Value> {
        if self.lines.is_empty() && self.dropped == 0 {
            return None;
        }
        let mut take = 0;
        let mut size = 0;
        for line in self.lines.iter().take(MAX_LINES_PER_FRAME) {
            let next = text_bytes(line);
            // Always at least one line, however long, so nothing waits forever.
            if take > 0 && size + next > MAX_FRAME_BYTES {
                break;
            }
            take += 1;
            size += next;
        }
        self.bytes -= size;
        let lines: Vec<Value> = self.lines.drain(..take).collect();
        let mut frame = json!({"event": "lines", "lines": lines});
        if self.dropped > 0 {
            frame["dropped"] = json!(self.dropped);
            self.dropped = 0;
        }
        Some(frame)
    }
}

impl LogFollow {
    fn status(&self, status: &str, message: Option<&str>) -> Value {
        let mut frame = json!({"event": "status", "source": self.source, "status": status});
        if let Some(message) = message {
            frame["message"] = json!(message);
        }
        frame
    }

    async fn run(mut self, tx: StreamEmitter) -> Result<(), String> {
        let mut pending = Pending::default();
        loop {
            let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
            let cluster = self.cluster.clone();
            let ask = self.ask.clone();
            let session = async move { cluster.logs(&ask, events_tx).await };
            tokio::pin!(session);
            let mut due: Option<Instant> = None;
            let ended = loop {
                tokio::select! {
                    biased;
                    Some(event) = events.recv() => match event {
                        LogEvent::Connected => {
                            if let Some(frame) = pending.frame() {
                                if send(&tx, frame).is_err() { return Ok(()); }
                            }
                            if send(&tx, self.status("live", None)).is_err() { return Ok(()); }
                        }
                        LogEvent::Line(line) => {
                            pending.push(line_frame(&self.source, line));
                            due.get_or_insert_with(|| Instant::now() + self.timing.batch);
                        }
                    },
                    () = async { tokio::time::sleep_until(due.unwrap()).await }, if due.is_some() => {
                        due = None;
                        if let Some(frame) = pending.frame() {
                            if send(&tx, frame).is_err() { return Ok(()); }
                        }
                        if !pending.lines.is_empty() {
                            due = Some(Instant::now() + self.timing.batch);
                        }
                    }
                    changed = self.inventory.changed() => {
                        if changed.is_err() {
                            return Ok(());
                        }
                        self.authority.check().await?;
                    }
                    result = &mut session => break result,
                }
            };
            // What arrived before the stream ended is still sent, in order and a
            // window apart: a follow that connected and ended in one poll still
            // said it was live.
            while let Ok(event) = events.try_recv() {
                match event {
                    LogEvent::Line(line) => pending.push(line_frame(&self.source, line)),
                    LogEvent::Connected => {
                        if let Some(frame) = pending.frame() {
                            if send(&tx, frame).is_err() {
                                return Ok(());
                            }
                        }
                        if send(&tx, self.status("live", None)).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
            while let Some(frame) = pending.frame() {
                if send(&tx, frame).is_err() {
                    return Ok(());
                }
                if !pending.lines.is_empty() {
                    tokio::time::sleep(self.timing.batch).await;
                }
            }
            if let Err(why) = &ended {
                if srelens_kube::watch::is_forbidden_kind_watch_error(why) {
                    return Err(why.clone());
                }
            }
            // Following again is reaching the pod again: only while the app
            // may, while the object still selects it, and while it is there.
            self.authority.check().await?;
            self.scope = scope::scope(
                self.cluster.as_ref(),
                &self.core,
                &self.ask.context,
                &self.manifest,
                &self.binding,
                &self.ask.namespace,
                self.object.as_deref(),
            )
            .await?;
            match self
                .cluster
                .pod(&self.ask.context, &self.ask.namespace, &self.ask.pod)
                .await
            {
                Ok(None) => {
                    let gone = format!("Pod {} no longer exists", self.ask.pod);
                    let _ = send(&tx, self.status("completed", Some(&gone)));
                    return Ok(());
                }
                Ok(Some(pod)) if !self.scope.admits(&pod) => {
                    return Err(self.scope.refusal(&self.authority.id, &self.ask.pod));
                }
                Ok(Some(pod)) if finished(&pod) => {
                    let done = format!(
                        "Pod {} has finished ({})",
                        self.ask.pod,
                        pod.phase.as_deref().unwrap_or("unknown")
                    );
                    let _ = send(&tx, self.status("completed", Some(&done)));
                    return Ok(());
                }
                Ok(Some(_)) | Err(_) => {}
            }
            let message = match &ended {
                Ok(()) => "The log stream ended; following again".to_owned(),
                Err(why) => why.clone(),
            };
            if send(&tx, self.status("reconnecting", Some(&message))).is_err() {
                return Ok(());
            }
            tokio::time::sleep(self.timing.reconnect).await;
            // Only what is new: the history was sent on the first follow.
            self.ask.tail_lines = 0;
            self.ask.since_seconds = None;
        }
    }
}

/// A pod that has run to its end and will not run again.
fn finished(pod: &PodFacts) -> bool {
    matches!(pod.phase.as_deref(), Some("Succeeded" | "Failed"))
}

/// One `exec` stream's source.
struct ExecRun {
    ask: ExecAsk,
    cluster: Arc<dyn PodCluster>,
    authority: Authority,
    timing: PodTiming,
    inventory: tokio::sync::watch::Receiver<u64>,
}

impl ExecRun {
    async fn run(mut self, tx: StreamEmitter) -> Result<(), String> {
        let (output_tx, mut output) = tokio::sync::mpsc::unbounded_channel();
        let cluster = self.cluster.clone();
        let ask = self.ask.clone();
        let session = async move { cluster.exec(&ask, output_tx).await };
        tokio::pin!(session);
        let limit = tokio::time::sleep(self.timing.exec_limit);
        tokio::pin!(limit);
        let mut chunks: Vec<(Output, String)> = Vec::new();
        let mut written = 0usize;
        let mut due: Option<Instant> = None;
        let flush = |chunks: &mut Vec<(Output, String)>, tx: &StreamEmitter| -> Result<(), Ended> {
            if chunks.is_empty() {
                return Ok(());
            }
            let frame: Vec<Value> = chunks
                .drain(..)
                .map(|(stream, text)| json!({"stream": stream.as_str(), "text": text}))
                .collect();
            send(tx, json!({"event": "output", "chunks": frame}))
        };
        let mut take = |chunks: &mut Vec<(Output, String)>, stream: Output, text: String| {
            written += text.len();
            match chunks.last_mut() {
                Some((last, joined)) if *last == stream => joined.push_str(&text),
                _ => chunks.push((stream, text)),
            }
            written
        };
        let code = loop {
            tokio::select! {
                biased;
                Some((stream, text)) = output.recv() => {
                    if take(&mut chunks, stream, text) > MAX_EXEC_OUTPUT {
                        let _ = flush(&mut chunks, &tx);
                        return Err(format!(
                            "The command wrote more than {} KiB of output, the most the host keeps for an app's command; the host stopped it",
                            MAX_EXEC_OUTPUT / 1024
                        ));
                    }
                    due.get_or_insert_with(|| Instant::now() + self.timing.batch);
                }
                () = async { tokio::time::sleep_until(due.unwrap()).await }, if due.is_some() => {
                    due = None;
                    if flush(&mut chunks, &tx).is_err() { return Ok(()); }
                }
                changed = self.inventory.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    self.authority.check().await?;
                }
                () = &mut limit => {
                    let _ = flush(&mut chunks, &tx);
                    return Err(format!(
                        "The command ran past {} seconds, the most an app's command may run; the host stopped it",
                        self.timing.exec_limit.as_secs()
                    ));
                }
                result = &mut session => break result?,
            }
        };
        while let Ok((stream, text)) = output.try_recv() {
            take(&mut chunks, stream, text);
        }
        if flush(&mut chunks, &tx).is_err() {
            return Ok(());
        }
        let _ = send(&tx, json!({"event": "exit", "code": code}));
        Ok(())
    }
}

/// One `portForward` stream's source. Owns the listener and, through a
/// `JoinSet`, every connection: when the stream ends the source is aborted,
/// and dropping it closes the port and every connection through it.
struct Forward {
    listener: TcpListener,
    local: u16,
    context: String,
    pod: PodFacts,
    /// The pod's port the connections reach.
    remote: u16,
    /// The binding's port: the pod's, or the Service's.
    port: u16,
    service: Option<String>,
    object: Option<String>,
    manifest: Manifest,
    binding: Binding,
    scope: Scope,
    cluster: Arc<dyn PodCluster>,
    core: Arc<srelens_capability::Registry>,
    app: String,
    authority: Authority,
    timing: PodTiming,
    inventory: tokio::sync::watch::Receiver<u64>,
}

impl Forward {
    fn ready(&self) -> Value {
        let mut frame = json!({
            "event": "ready", "localPort": self.local,
            "pod": self.pod.name, "port": self.remote,
        });
        if let Some(service) = &self.service {
            frame["service"] = json!(service);
            frame["servicePort"] = json!(self.port);
        }
        frame
    }

    async fn run(mut self, tx: StreamEmitter) -> Result<(), String> {
        if send(&tx, self.ready()).is_err() {
            return Ok(());
        }
        let mut connections = JoinSet::new();
        // A connection the cluster refuses closes at once on the client's side,
        // and is said here: how many failed since the last check, and why the
        // last one did. At most one frame a check, so a client retrying in a
        // loop cannot take the app past its message rate.
        let (failures, mut failed) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut unreported: Option<(u64, String)> = None;
        let mut monitor =
            tokio::time::interval_at(Instant::now() + self.timing.monitor, self.timing.monitor);
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (mut socket, _) = accepted.map_err(|e| format!("The local port stopped accepting: {e}"))?;
                    let cluster = self.cluster.clone();
                    let failures = failures.clone();
                    let (context, namespace, pod, port) = (
                        self.context.clone(), self.scope.namespace.clone(), self.pod.name.clone(), self.remote,
                    );
                    connections.spawn(async move {
                        match cluster.connect(&context, &namespace, &pod, port).await {
                            Ok(mut upstream) => {
                                let _ = tokio::io::copy_bidirectional(&mut socket, &mut upstream).await;
                            }
                            Err(why) => {
                                log::warn!("app port-forward to {namespace}/{pod}:{port} on {context} failed: {why}");
                                let _ = failures.send(why);
                            }
                        }
                    });
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
                Some(why) = failed.recv() => {
                    let count = unreported.as_ref().map_or(0, |(count, _)| *count);
                    unreported = Some((count + 1, why));
                }
                _ = monitor.tick() => {
                    if let Some((count, message)) = unreported.take() {
                        let frame = json!({"event": "connectionFailed", "count": count, "message": message});
                        if send(&tx, frame).is_err() {
                            return Ok(());
                        }
                    }
                    match self.cluster.pod(&self.context, &self.scope.namespace, &self.pod.name).await {
                        Ok(Some(pod)) if !self.scope.admits(&pod) => {
                            return Err(self.scope.refusal(&self.app, &self.pod.name));
                        }
                        Ok(Some(pod)) if pod.running() => {}
                        // A transient failure to ask is not the pod going away.
                        Err(_) => {}
                        Ok(_) => self.retarget(&tx).await?,
                    }
                }
                changed = self.inventory.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    self.authority.check().await?;
                }
            }
        }
    }

    /// The pod went away. A forward to a pod ends with why; one through a
    /// Service follows it to another pod in scope, and says which.
    async fn retarget(&mut self, tx: &StreamEmitter) -> Result<(), String> {
        let Some(service) = self.service.clone() else {
            return Err(format!(
                "Pod {} is no longer running; the forward ended",
                self.pod.name
            ));
        };
        self.authority.check().await?;
        self.scope = scope::scope(
            self.cluster.as_ref(),
            &self.core,
            &self.context,
            &self.manifest,
            &self.binding,
            &self.scope.namespace,
            self.object.as_deref(),
        )
        .await?;
        let (pod, remote) = scope::service_target(
            self.cluster.as_ref(),
            &self.context,
            &self.scope,
            &self.app,
            &service,
            self.port,
        )
        .await?;
        self.pod = pod;
        self.remote = remote;
        send(tx, self.ready()).map_err(|_| "The stream ended".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines are held by bytes as well as by count: a container writing long lines
    /// faster than the host sends them cannot grow the buffer past its budget, and
    /// what it let go is counted, as a line past the count is.
    #[test]
    fn pending_lines_are_bounded_by_bytes_and_each_frame_by_its_size() {
        let mut pending = Pending::default();
        let long = "x".repeat(MAX_LINE_BYTES);
        let fits = MAX_PENDING_BYTES / MAX_LINE_BYTES;
        for _ in 0..fits + 10 {
            pending.push(line_frame("web-1/app", long.clone()));
        }
        assert_eq!(pending.lines.len(), fits);
        assert_eq!(pending.dropped, 10);
        assert!(pending.bytes <= MAX_PENDING_BYTES);
        let mut carried = 0;
        let mut first = true;
        while let Some(frame) = pending.frame() {
            let size: usize = frame["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line["line"].as_str().unwrap().len())
                .sum();
            assert!(size <= MAX_FRAME_BYTES, "a frame of {size} bytes");
            // What was dropped is said once, on the next frame.
            assert_eq!(frame.get("dropped").is_some(), first);
            first = false;
            carried += frame["lines"].as_array().unwrap().len();
        }
        assert_eq!(carried, fits);
        assert_eq!(pending.bytes, 0);
        // One line over the frame's size still goes, alone, rather than never.
        let mut pending = Pending::default();
        pending.push(json!({"source": "s", "line": "y".repeat(MAX_FRAME_BYTES + 1)}));
        assert_eq!(
            pending.frame().unwrap()["lines"].as_array().unwrap().len(),
            1
        );
    }

    /// A long line is cut by bytes, on a character boundary, and says so.
    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let line = "é".repeat(MAX_LINE_BYTES);
        let frame = line_frame("s", line);
        let cut = frame["line"].as_str().unwrap();
        assert!(cut.len() <= MAX_LINE_BYTES && cut.len() > MAX_LINE_BYTES - 4);
        assert!(cut.chars().all(|c| c == 'é'));
        assert_eq!(frame["truncated"], true);
        assert_eq!(
            line_frame("s", "short".into()),
            json!({"source": "s", "line": "short"})
        );
    }
}
