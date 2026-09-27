//! The Extension Inspector and per-app logs (#575): what an installed app is
//! doing now, read by Settings → Apps → app → Inspector and Logs.
//!
//! [`AppRuntime`] is the process's record of its apps at run time, one per
//! inventory, beside the app streams: each app's log ([`AppLog`]), and the
//! supervisor of each app that runs a sidecar, held as [`Inspect`] so that
//! only its metrics are read. Executable apps are started by #574, which asks
//! [`AppRuntime::log`] for the log to give the supervisor and hands it back
//! with [`AppRuntime::attach`]. Until then no app has a process, and the
//! Inspector says so.
//!
//! **Local only.** Both capabilities are UI-only (`Capability::ui_only`): the
//! MCP server never lists or calls them, because an agent's context goes to
//! its LLM provider and a sidecar's stderr is text a third party wrote.
//! Nothing here is written to disk, sent anywhere, or recorded in the audit
//! trail; it lives in this process's memory, bounded, and goes with it.

use super::streams::ExtensionStreams;
use super::{read, Inventory, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_plugin_host::app_log::{AppLog, LogLevel, LogLine, LogSource, LOG_LINES};
use srelens_plugin_host::sidecar::{Action, Enforcement, Inspect, SidecarMetrics, SidecarStatus};
use srelens_streams::app::StreamMetrics;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The apps of one inventory at run time: their logs and their sidecars.
#[derive(Default)]
pub struct AppRuntime {
    logs: Mutex<HashMap<String, AppLog>>,
    sidecars: Mutex<HashMap<String, Arc<dyn Inspect>>>,
}

impl AppRuntime {
    /// The log of the app `id`, started empty the first time it is asked for.
    /// The one to pass to `Supervisor::start_with_log`.
    pub fn log(&self, id: &str) -> AppLog {
        self.logs
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    /// Show `sidecar` as the process of the app `id`, in place of any before.
    pub fn attach(&self, id: &str, sidecar: Arc<dyn Inspect>) {
        self.sidecars.lock().unwrap().insert(id.to_owned(), sidecar);
    }

    /// The app `id` has no process any more. Its log stays.
    pub fn detach(&self, id: &str) {
        self.sidecars.lock().unwrap().remove(id);
    }

    fn sidecar(&self, id: &str) -> Option<Arc<dyn Inspect>> {
        self.sidecars.lock().unwrap().get(id).cloned()
    }

    fn existing_log(&self, id: &str) -> Option<AppLog> {
        self.logs.lock().unwrap().get(id).cloned()
    }

    /// Forget every app `state` no longer holds: its log goes with it, so a
    /// reinstall starts with an empty one.
    pub(super) fn forget_removed(&self, state: &Inventory) {
        let installed = |id: &String| state.plugins.iter().any(|p| &p.manifest.id == id);
        self.logs.lock().unwrap().retain(|id, _| installed(id));
        self.sidecars.lock().unwrap().retain(|id, _| installed(id));
    }
}

/// What `@srelens/core`'s `inspectExtension` sends.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectIn {
    /// The app's ID.
    pub id: String,
}

/// What `@srelens/core`'s `extensionLogs` sends. `minLevel` is the wrapper's
/// spelling; the snake_case one is refused with every other unknown field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogsIn {
    pub id: String,
    /// Only lines after this one (a `seq`); 0, the default, for all of them.
    #[serde(default)]
    pub after: u64,
    /// Only lines at this level or above; every level by default.
    #[serde(default, rename = "minLevel")]
    pub min_level: Option<LogLevel>,
}

/// Whether the app runs a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    /// Nothing of its own runs: the host reads and renders for it.
    Declarative,
    /// A supervised sidecar process.
    Sidecar,
}

/// One log line.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LineOut {
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    pub level: LogLevel,
    pub source: LogSource,
    /// Redacted on its way into the log.
    pub text: String,
}

impl From<LogLine> for LineOut {
    fn from(line: LogLine) -> LineOut {
        LineOut {
            seq: line.seq,
            at: millis(line.at),
            level: line.level,
            source: line.source,
            text: line.text,
        }
    }
}

fn millis(at: SystemTime) -> u64 {
    at.duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// How much of the app's log is kept.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LogSummary {
    pub lines: usize,
    pub capacity: usize,
    /// Lines dropped to keep it at `capacity`.
    pub dropped: u64,
}

/// What `extensions.logs` answers.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LogsOut {
    pub runtime: Runtime,
    pub lines: Vec<LineOut>,
    pub capacity: usize,
    pub dropped: u64,
}

/// What `extensions.inspect` answers.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectOut {
    pub id: String,
    pub runtime: Runtime,
    /// `None` for an app with no process.
    pub process: Option<ProcessOut>,
    pub streams: StreamsOut,
    /// The last errors in the app's log, oldest first.
    pub recent_errors: Vec<LineOut>,
    pub log: LogSummary,
}

/// The streams the app's views opened in this process.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StreamsOut {
    /// Open now, except watches.
    pub open: Vec<StreamMetrics>,
    /// Open now, following a kind (the `watch` source).
    pub watches: Vec<StreamMetrics>,
    pub opened: u64,
    pub messages: u64,
    pub bytes: u64,
    pub rate_limited: u64,
    pub refused: u64,
    pub window_ended: u64,
    pub max_open: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState {
    Starting,
    Running,
    Restarting,
    Disabled,
    Refused,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ActionOut {
    Restart,
    ViewLogs,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnforcementOut {
    Kernel,
    Host,
    Missing,
}

/// The app's sidecar.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOut {
    pub state: ProcessState,
    /// Why, for a restarting, disabled or refused sidecar. Redacted.
    pub reason: Option<String>,
    /// The headline, for a disabled one: "Extension process exited unexpectedly".
    pub message: Option<String>,
    /// What a person can do from this state: the supervisor's own list.
    pub actions: Vec<ActionOut>,
    /// The sidecar API version negotiated at start, while it runs.
    pub api_version: Option<String>,
    pub pid: Option<u32>,
    /// Milliseconds since the Unix epoch, when the running process came up.
    pub started_at: Option<u64>,
    pub restart: Option<RestartOut>,
    pub launches: u64,
    pub unexpected_exits: u64,
    pub memory: MemoryOut,
    pub cpus: f64,
    pub rpc: RpcOut,
    pub streams: SidecarStreamsOut,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestartOut {
    pub attempt: usize,
    pub of: usize,
    pub delay_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryOut {
    /// `None` when the sandbox backend does not measure it, or nothing runs.
    pub bytes: Option<u64>,
    pub limit_bytes: u64,
    pub enforcement: EnforcementOut,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcOut {
    pub answered: u64,
    pub failed: u64,
    pub timed_out: u64,
    pub refused: u64,
    pub in_flight: usize,
    pub latency: LatencyOut,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LatencyOut {
    pub samples: usize,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SidecarStreamsOut {
    pub open: usize,
    pub opened: u64,
    pub limit: usize,
}

impl From<SidecarMetrics> for ProcessOut {
    fn from(metrics: SidecarMetrics) -> ProcessOut {
        let status = &metrics.status;
        let (state, reason, api_version, pid, restart) = match status {
            SidecarStatus::Starting => (ProcessState::Starting, None, None, None, None),
            SidecarStatus::Running { api_version, pid } => (
                ProcessState::Running,
                None,
                Some(api_version.to_string()),
                *pid,
                None,
            ),
            SidecarStatus::Restarting {
                attempt,
                of,
                delay,
                reason,
            } => (
                ProcessState::Restarting,
                Some(reason.clone()),
                None,
                None,
                Some(RestartOut {
                    attempt: *attempt,
                    of: *of,
                    delay_ms: delay.as_millis() as u64,
                }),
            ),
            SidecarStatus::Disabled { reason } => (
                ProcessState::Disabled,
                Some(reason.clone()),
                None,
                None,
                None,
            ),
            SidecarStatus::Refused { reason } => (
                ProcessState::Refused,
                Some(reason.clone()),
                None,
                None,
                None,
            ),
            SidecarStatus::Stopping => (ProcessState::Stopping, None, None, None, None),
            SidecarStatus::Stopped => (ProcessState::Stopped, None, None, None, None),
        };
        let requests = &metrics.requests;
        ProcessOut {
            state,
            reason,
            message: status.message().map(str::to_owned),
            actions: status
                .actions()
                .iter()
                .map(|action| match action {
                    Action::Restart => ActionOut::Restart,
                    Action::ViewLogs => ActionOut::ViewLogs,
                    Action::Disable => ActionOut::Disable,
                })
                .collect(),
            api_version,
            pid,
            started_at: metrics.started_at.map(millis),
            restart,
            launches: metrics.launches,
            unexpected_exits: metrics.unexpected_exits,
            memory: MemoryOut {
                bytes: metrics.memory_bytes,
                limit_bytes: metrics.limits.memory_bytes,
                enforcement: match metrics.enforcement {
                    Enforcement::Kernel => EnforcementOut::Kernel,
                    Enforcement::Host => EnforcementOut::Host,
                    Enforcement::Missing(_) => EnforcementOut::Missing,
                },
            },
            cpus: metrics.limits.cpus,
            rpc: RpcOut {
                answered: requests.answered,
                failed: requests.failed,
                timed_out: requests.timed_out,
                refused: requests.refused,
                in_flight: requests.in_flight,
                latency: LatencyOut {
                    samples: requests.latency.samples,
                    p50_ms: requests.latency.p50.map(ms),
                    p95_ms: requests.latency.p95.map(ms),
                    max_ms: requests.latency.max.map(ms),
                },
            },
            streams: SidecarStreamsOut {
                open: metrics.open_streams,
                opened: metrics.streams_opened,
                limit: metrics.limits.max_streams,
            },
        }
    }
}

/// The installed app `id`, refused when there is none.
async fn installed(path: &Store, id: &str) -> Result<(), CapabilityError> {
    let path = path.clone();
    let state = tokio::task::spawn_blocking(move || read(&*path))
        .await
        .map_err(|e| CapabilityError::Handler(e.to_string()))?
        .map_err(CapabilityError::Handler)?;
    if state.plugins.iter().any(|p| p.manifest.id == id) {
        Ok(())
    } else {
        Err(CapabilityError::InvalidInput(format!(
            "No app with the ID {id} is installed"
        )))
    }
}

/// Everything the Inspector shows about the app `id`, but what the inventory
/// already says (its manifest, grants and source).
pub(super) fn inspect(streams: &ExtensionStreams, id: &str) -> InspectOut {
    let runtime = streams.runtime();
    let sidecar = runtime.sidecar(id);
    let log = runtime.existing_log(id);
    let apps = streams.metrics();
    let app = apps.into_iter().find(|app| app.app == id);
    let (watches, open) = app
        .as_ref()
        .map(|app| app.streams.clone())
        .unwrap_or_default()
        .into_iter()
        .partition(|stream| stream.source == "watch");
    InspectOut {
        id: id.to_owned(),
        runtime: if sidecar.is_some() {
            Runtime::Sidecar
        } else {
            Runtime::Declarative
        },
        process: sidecar.map(|sidecar| sidecar.metrics().into()),
        streams: StreamsOut {
            open,
            watches,
            opened: app.as_ref().map_or(0, |app| app.opened),
            messages: app.as_ref().map_or(0, |app| app.messages),
            bytes: app.as_ref().map_or(0, |app| app.bytes),
            rate_limited: app.as_ref().map_or(0, |app| app.rate_limited),
            refused: app.as_ref().map_or(0, |app| app.refused),
            window_ended: app.as_ref().map_or(0, |app| app.window_ended),
            max_open: streams.stream_limits().max_open_per_app,
        },
        recent_errors: log
            .as_ref()
            .map(|log| log.recent_errors().into_iter().map(LineOut::from).collect())
            .unwrap_or_default(),
        log: LogSummary {
            lines: log.as_ref().map_or(0, AppLog::len),
            capacity: LOG_LINES,
            dropped: log.as_ref().map_or(0, AppLog::dropped),
        },
    }
}

/// The lines of the app `id`'s log a reader asked for.
pub(super) fn logs(streams: &ExtensionStreams, input: &LogsIn) -> LogsOut {
    let runtime = streams.runtime();
    let log = runtime.existing_log(&input.id);
    LogsOut {
        runtime: if runtime.sidecar(&input.id).is_some() {
            Runtime::Sidecar
        } else {
            Runtime::Declarative
        },
        lines: log
            .as_ref()
            .map(|log| {
                log.lines_since(input.after, input.min_level.unwrap_or(LogLevel::Trace))
                    .into_iter()
                    .map(LineOut::from)
                    .collect()
            })
            .unwrap_or_default(),
        capacity: LOG_LINES,
        dropped: log.as_ref().map_or(0, AppLog::dropped),
    }
}

/// Register `extensions.inspect` and `extensions.logs`, UI-only, over the
/// streams and runtime of the inventory at `path`.
pub(super) fn register(reg: &mut Registry, path: Store, streams: Arc<ExtensionStreams>) {
    let (p, s) = (path.clone(), streams.clone());
    reg.register(
        Capability::typed::<InspectIn, InspectOut, _, _>(
            "extensions.inspect",
            "Report what an installed app is doing now: its process, memory, requests, open streams and watches, and recent errors. For srelens's own UI only; never offered to MCP",
            Annotations::READ_ONLY,
            move |input: InspectIn| {
                let (p, s) = (p.clone(), s.clone());
                async move {
                    installed(&p, &input.id).await?;
                    Ok::<_, CapabilityError>(inspect(&s, &input.id))
                }
            },
        )
        .only_in_the_ui(),
    );
    reg.register(
        Capability::typed::<LogsIn, LogsOut, _, _>(
            "extensions.logs",
            "Read an installed app's log, from trace to error, redacted. For srelens's own UI only; never offered to MCP",
            Annotations::READ_ONLY,
            move |input: LogsIn| {
                let (p, s) = (path.clone(), streams.clone());
                async move {
                    installed(&p, &input.id).await?;
                    Ok::<_, CapabilityError>(logs(&s, &input))
                }
            },
        )
        .only_in_the_ui(),
    );
}

#[cfg(test)]
mod tests {
    use super::super::tests::{configure, fake_core, install};
    use super::*;
    use serde_json::{json, Value};
    use srelens_plugin_host::sidecar::{Latency, Limits, RequestMetrics, UNEXPECTED_EXIT};
    use std::path::{Path, PathBuf};

    /// What `@srelens/core`'s `extensionLogs` sends, byte for byte:
    /// `extensions.test.ts` holds the wrapper to this same file.
    const LOGS_PAYLOAD: &str =
        include_str!("../../../../packages/core/src/lib/extension-logs-request.json");
    const APP: &str = "org.example.argocd";

    fn setup(dir: &Path) -> (PathBuf, Registry, Arc<ExtensionStreams>) {
        let path = dir.join("extensions.json");
        let mut reg = Registry::new();
        let streams = super::super::register(
            &mut reg,
            path.clone(),
            fake_core(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        (path, reg, streams)
    }

    struct Stub(SidecarMetrics);

    impl Inspect for Stub {
        fn metrics(&self) -> SidecarMetrics {
            self.0.clone()
        }
    }

    fn sidecar(status: SidecarStatus) -> SidecarMetrics {
        SidecarMetrics {
            status,
            started_at: None,
            launches: 4,
            unexpected_exits: 4,
            memory_bytes: None,
            limits: Limits::default(),
            enforcement: Enforcement::Kernel,
            requests: RequestMetrics::default(),
            open_streams: 0,
            streams_opened: 0,
        }
    }

    async fn inspect(reg: &Registry) -> Value {
        reg.invoke("extensions.inspect", json!({"id": APP}))
            .await
            .unwrap()
    }

    #[test]
    fn the_wrappers_payloads_deserialize_and_snake_case_is_rejected() {
        let payload: Value = serde_json::from_str(LOGS_PAYLOAD).unwrap();
        let input: LogsIn = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(input.id, APP);
        assert_eq!(input.after, 41);
        assert_eq!(input.min_level, Some(LogLevel::Warn));
        let snake = json!({"id": APP, "min_level": "warn"});
        let error = serde_json::from_value::<LogsIn>(snake)
            .unwrap_err()
            .to_string();
        assert!(error.contains("min_level"), "{error}");
        assert!(serde_json::from_value::<LogsIn>(json!({"id": APP, "minLevel": "loud"})).is_err());
        let bare: LogsIn = serde_json::from_value(json!({"id": APP})).unwrap();
        assert_eq!((bare.after, bare.min_level), (0, None));
        assert!(serde_json::from_value::<InspectIn>(json!({"id": APP, "app": APP})).is_err());
    }

    #[tokio::test]
    async fn a_declarative_app_has_no_process_and_nothing_logged() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, _streams) = setup(dir.path());
        install(&path, fake_core());
        assert_eq!(
            inspect(&reg).await,
            json!({
                "id": APP,
                "runtime": "declarative",
                "process": null,
                "streams": {"open": [], "watches": [], "opened": 0, "messages": 0, "bytes": 0,
                            "rateLimited": 0, "refused": 0, "windowEnded": 0, "maxOpen": 8},
                "recentErrors": [],
                "log": {"lines": 0, "capacity": 1000, "dropped": 0},
            })
        );
        let logs = reg
            .invoke("extensions.logs", json!({"id": APP}))
            .await
            .unwrap();
        assert_eq!(
            logs,
            json!({"runtime": "declarative", "lines": [], "capacity": 1000, "dropped": 0})
        );
    }

    #[tokio::test]
    async fn an_app_that_is_not_installed_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, _streams) = setup(dir.path());
        install(&path, fake_core());
        for id in ["extensions.inspect", "extensions.logs"] {
            let error = reg
                .invoke(id, json!({"id": "org.example.gone"}))
                .await
                .unwrap_err();
            assert!(
                matches!(&error, CapabilityError::InvalidInput(m) if m == "No app with the ID org.example.gone is installed"),
                "{id}: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_crashed_sidecar_is_shown_disabled_with_view_logs_and_its_errors() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        install(&path, fake_core());
        let runtime = streams.runtime();
        let reason =
            "The extension process exited unexpectedly: it was killed by signal 6 (SIGABRT)";
        runtime.log(APP).host(LogLevel::Error, reason);
        runtime.log(APP).host(
            LogLevel::Warn,
            "Restarting the extension in 1 s (attempt 1 of 3)",
        );
        runtime.attach(
            APP,
            Arc::new(Stub(sidecar(SidecarStatus::Disabled {
                reason: reason.into(),
            }))),
        );
        let out = inspect(&reg).await;
        assert_eq!(out["runtime"], "sidecar");
        let process = &out["process"];
        assert_eq!(process["state"], "disabled");
        assert_eq!(process["message"], UNEXPECTED_EXIT);
        assert_eq!(process["reason"], reason);
        assert_eq!(
            process["actions"],
            json!(["restart", "viewLogs", "disable"])
        );
        assert_eq!(process["pid"], Value::Null);
        assert_eq!(process["startedAt"], Value::Null);
        assert_eq!(
            (
                process["launches"].clone(),
                process["unexpectedExits"].clone()
            ),
            (json!(4), json!(4))
        );
        assert_eq!(
            process["memory"],
            json!({"bytes": null, "limitBytes": 268_435_456u64, "enforcement": "kernel"})
        );
        assert_eq!(
            process["rpc"],
            json!({"answered": 0, "failed": 0, "timedOut": 0, "refused": 0, "inFlight": 0,
                   "latency": {"samples": 0, "p50Ms": null, "p95Ms": null, "maxMs": null}})
        );
        assert_eq!(
            process["streams"],
            json!({"open": 0, "opened": 0, "limit": 5})
        );
        let errors = out["recentErrors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["text"], reason);
        assert_eq!(
            (errors[0]["level"].clone(), errors[0]["source"].clone()),
            (json!("error"), json!("host"))
        );
        assert_eq!(out["log"]["lines"], 2);

        // What View logs opens.
        let logs = reg
            .invoke("extensions.logs", json!({"id": APP}))
            .await
            .unwrap();
        assert_eq!(logs["runtime"], "sidecar");
        let levels: Vec<&str> = logs["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["level"].as_str().unwrap())
            .collect();
        assert_eq!(levels, ["error", "warn"]);
    }

    #[tokio::test]
    async fn a_running_sidecar_is_shown_with_its_memory_latency_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        install(&path, fake_core());
        let mut running = sidecar(SidecarStatus::Running {
            api_version: semver::Version::new(0, 1, 0),
            pid: Some(4242),
        });
        running.started_at = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000));
        running.memory_bytes = Some(48 * 1024 * 1024);
        running.enforcement = Enforcement::Host;
        running.open_streams = 2;
        running.streams_opened = 3;
        running.requests = RequestMetrics {
            answered: 10,
            failed: 1,
            timed_out: 1,
            refused: 2,
            in_flight: 1,
            latency: Latency {
                samples: 11,
                p50: Some(Duration::from_millis(12)),
                p95: Some(Duration::from_micros(80_500)),
                max: Some(Duration::from_millis(250)),
            },
        };
        streams.runtime().attach(APP, Arc::new(Stub(running)));
        let process = inspect(&reg).await["process"].clone();
        assert_eq!(process["state"], "running");
        assert_eq!(process["apiVersion"], "0.1.0");
        assert_eq!(process["pid"], 4242);
        assert_eq!(process["startedAt"], 1_800_000_000_000u64);
        assert_eq!(process["memory"]["bytes"], 48 * 1024 * 1024);
        assert_eq!(process["memory"]["enforcement"], "host");
        assert_eq!(process["cpus"], 1.0);
        assert_eq!(process["actions"], json!([]));
        assert_eq!(process["message"], Value::Null);
        assert_eq!(
            process["rpc"],
            json!({"answered": 10, "failed": 1, "timedOut": 1, "refused": 2, "inFlight": 1,
                   "latency": {"samples": 11, "p50Ms": 12.0, "p95Ms": 80.5, "maxMs": 250.0}})
        );
        assert_eq!(
            process["streams"],
            json!({"open": 2, "opened": 3, "limit": 5})
        );

        streams.runtime().attach(
            APP,
            Arc::new(Stub(sidecar(SidecarStatus::Restarting {
                attempt: 2,
                of: 3,
                delay: Duration::from_secs(5),
                reason: "it exited with status 1".into(),
            }))),
        );
        let process = inspect(&reg).await["process"].clone();
        assert_eq!(process["state"], "restarting");
        assert_eq!(
            process["restart"],
            json!({"attempt": 2, "of": 3, "delayMs": 5000})
        );
        streams.runtime().detach(APP);
        assert_eq!(inspect(&reg).await["process"], Value::Null);
    }

    /// AGENTS.md: a new ungated reader needs a test that the plaintext is
    /// absent from what it returns. Both readers, both kinds of rule.
    #[tokio::test]
    async fn neither_reader_returns_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        install(&path, fake_core());
        let log = streams.runtime().log(APP);
        log.scrub("registry-robot-hunter2");
        log.sidecar(b"ERROR login as registry-robot-hunter2 refused");
        log.sidecar(b"DEBUG GET https://robot:s3cr3t-pw@registry.example/v2/");
        log.sidecar(b"WARN Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln");
        let answers = format!(
            "{} {}",
            reg.invoke("extensions.logs", json!({"id": APP}))
                .await
                .unwrap(),
            inspect(&reg).await
        );
        for secret in [
            "registry-robot-hunter2",
            "s3cr3t-pw",
            "eyJhbGciOiJSUzI1NiJ9",
        ] {
            assert!(
                !answers.contains(secret),
                "{secret} was returned: {answers}"
            );
        }
        assert!(answers.contains("registry.example/v2/"), "{answers}");
    }

    #[tokio::test]
    async fn a_reader_asks_from_a_level_up_and_after_the_last_line_it_has() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        install(&path, fake_core());
        let log = streams.runtime().log(APP);
        for line in ["TRACE a", "DEBUG b", "INFO c", "WARN d", "ERROR e"] {
            log.sidecar(line.as_bytes());
        }
        let texts = |out: Value| -> Vec<String> {
            out["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l["text"].as_str().unwrap().to_owned())
                .collect()
        };
        let warn = reg
            .invoke("extensions.logs", json!({"id": APP, "minLevel": "warn"}))
            .await
            .unwrap();
        assert_eq!(texts(warn), ["d", "e"]);
        let after = reg
            .invoke("extensions.logs", json!({"id": APP, "after": 3}))
            .await
            .unwrap();
        assert_eq!(texts(after), ["d", "e"]);
        let all = reg
            .invoke("extensions.logs", json!({"id": APP}))
            .await
            .unwrap();
        assert_eq!(all["lines"][0]["seq"], 1);
        assert!(all["lines"][0]["at"].as_u64().unwrap() > 1_700_000_000_000);
        assert_eq!(all["lines"][0]["source"], "sidecar");
    }

    #[tokio::test]
    async fn a_removed_app_takes_its_log_and_process_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        install(&path, fake_core());
        streams
            .runtime()
            .log(APP)
            .host(LogLevel::Info, "The extension is running");
        streams
            .runtime()
            .attach(APP, Arc::new(Stub(sidecar(SidecarStatus::Stopped))));
        configure(&path, json!({"action": "remove", "id": APP})).unwrap();
        install(&path, fake_core());
        let out = inspect(&reg).await;
        assert_eq!(out["runtime"], "declarative");
        assert_eq!(out["log"]["lines"], 0);
    }

    /// Local only: nothing the Inspector or the log holds is written to disk.
    #[tokio::test]
    async fn nothing_the_inspector_reads_is_written_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        install(&path, fake_core());
        streams
            .runtime()
            .log(APP)
            .sidecar(b"ERROR marker-5f2c1e the scan failed");
        streams.runtime().attach(
            APP,
            Arc::new(Stub(sidecar(SidecarStatus::Disabled {
                reason: "marker-5f2c1e".into(),
            }))),
        );
        inspect(&reg).await;
        reg.invoke("extensions.logs", json!({"id": APP}))
            .await
            .unwrap();
        configure(
            &path,
            json!({"action": "enable", "id": APP, "enabled": false}),
        )
        .unwrap();
        for entry in walk(dir.path()) {
            let bytes = std::fs::read(&entry).unwrap_or_default();
            assert!(
                !String::from_utf8_lossy(&bytes).contains("marker-5f2c1e"),
                "{} holds the app's log",
                entry.display()
            );
        }
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }
}
