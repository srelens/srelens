//! Keeps one sidecar running: starts it through the sandbox, negotiates the
//! API version, admits requests while it runs, checks that it answers, and
//! restarts it after an unexpected exit on the policy's backoff until the
//! backoff runs out, when it is disabled.
//!
//! A sidecar can never take srelens down with it. Everything it does arrives
//! as bytes on a pipe or as an exit status: nothing it writes can panic the
//! host, a request never waits past its deadline, and when the process ends
//! every caller still waiting is answered at once.

use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::connection::{self, human, Broker, Connection, ReadEnd, RequestError, SidecarStream};
use super::data;
use super::metrics::{Inspect, Recorder, SidecarMetrics};
use super::protocol::{self, method, BoundedLines, Line, SIDECAR_API_VERSIONS};
use super::sandbox::{Enforcement, LaunchError, Launcher, MemoryProbe, Process, SidecarCommand};
use super::{Limits, Policy, LOG_LINE_BYTES};
use crate::app_log::{AppLog, LogLevel, LogLine};

/// What a person is told when the backoff has run out (#572).
pub const UNEXPECTED_EXIT: &str = "Extension process exited unexpectedly";

/// What a person can do about a disabled sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Start it again, with a fresh backoff: [`Supervisor::restart`].
    Restart,
    /// Read what it wrote and what happened to it: its app's log
    /// ([`Supervisor::log`]), in Settings → Apps → app → Logs (#575).
    ViewLogs,
    /// Turn the app off. The inventory is the registry's; the host calls
    /// [`Supervisor::stop`] and disables the app there.
    Disable,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Restart => "Restart",
            Action::ViewLogs => "View logs",
            Action::Disable => "Disable",
        }
    }
}

const DISABLED_ACTIONS: &[Action] = &[Action::Restart, Action::ViewLogs, Action::Disable];

/// Where a sidecar is in its life.
#[derive(Debug, Clone, PartialEq)]
pub enum SidecarStatus {
    /// Being launched, initialized and activated.
    Starting,
    /// Answering requests, under the API version it and srelens agreed on.
    Running {
        api_version: semver::Version,
        pid: Option<u32>,
    },
    /// It ended unexpectedly, or could not be started; it is started again
    /// after `delay`. `attempt` counts from 1 to `of`.
    Restarting {
        attempt: usize,
        of: usize,
        delay: Duration,
        reason: String,
    },
    /// It ended unexpectedly once more than the backoff allows. It stays
    /// stopped until a person restarts it. `reason` is the last exit.
    Disabled { reason: String },
    /// It cannot run here: no sandbox for this OS or a layer this machine
    /// lacks, limits nothing enforces, no API version in common, or a data
    /// directory that is over its limit or not private (#573). Starting it
    /// again would fail the same way, so nothing does unless asked.
    Refused { reason: String },
    /// Being stopped at the host's request.
    Stopping,
    /// Stopped at the host's request.
    Stopped,
}

impl SidecarStatus {
    /// The headline a surface shows for this state, when it has one.
    pub fn message(&self) -> Option<&'static str> {
        match self {
            SidecarStatus::Disabled { .. } => Some(UNEXPECTED_EXIT),
            _ => None,
        }
    }

    /// What a person can do from this state.
    pub fn actions(&self) -> &'static [Action] {
        match self {
            SidecarStatus::Disabled { .. } => DISABLED_ACTIONS,
            _ => &[],
        }
    }

    /// Why a request cannot be sent now, as a sentence.
    fn unavailable(&self) -> String {
        match self {
            SidecarStatus::Starting => "The extension is starting; try again in a moment".into(),
            SidecarStatus::Running { .. } => "The extension is running".into(),
            SidecarStatus::Restarting { attempt, of, delay, .. } => format!(
                "The extension stopped unexpectedly and is restarting (attempt {attempt} of {of}, in {})",
                human(*delay)
            ),
            SidecarStatus::Disabled { reason } => format!("{UNEXPECTED_EXIT}: {reason}"),
            SidecarStatus::Refused { reason } => reason.clone(),
            SidecarStatus::Stopping | SidecarStatus::Stopped => "The extension is stopped".into(),
        }
    }
}

/// One sidecar: what to run, its limits, and how it is kept running.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub command: SidecarCommand,
    pub limits: Limits,
    pub policy: Policy,
}

enum Control {
    /// Start again: from `Disabled`, `Refused` or `Stopped`, or restart one
    /// that is running. The backoff starts over.
    Restart,
    /// Stop, and say when done.
    Stop(oneshot::Sender<()>),
}

struct Shared {
    status: watch::Sender<SidecarStatus>,
    policy: Policy,
    limits: Limits,
    enforcement: Enforcement,
    /// The session, while the sidecar is running.
    connection: Mutex<Option<Connection>>,
    /// The latest session, from its launch until the next: what dropping the
    /// supervisor ends, while it starts and stops too, when `connection` is
    /// not set but the sidecar's calls to the host may be waiting (#573).
    session: Mutex<Option<Connection>>,
    /// The app's log: the host's, shared with the Inspector (#575).
    log: AppLog,
    recorder: Mutex<Recorder>,
    /// Reads the running process's memory, when its backend can.
    memory: Mutex<Option<MemoryProbe>>,
}

impl Shared {
    fn set(&self, status: SidecarStatus) {
        self.status.send_replace(status);
    }

    /// A line srelens writes about the sidecar, in its app's log.
    fn log(&self, level: LogLevel, text: &str) {
        self.log.host(level, text);
    }

    fn recorder(&self) -> std::sync::MutexGuard<'_, Recorder> {
        self.recorder.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Count one request's outcome, and write a failure to the log. A request
    /// refused because the sidecar is not running is counted, not logged: its
    /// state already says why, once.
    fn record<T>(&self, name: &str, took: Duration, outcome: &Result<T, RequestError>) {
        self.recorder().request(outcome, took);
        let level = match outcome {
            Ok(_) | Err(RequestError::Unavailable(_)) => return,
            Err(
                RequestError::Busy { .. }
                | RequestError::TooManyStreams { .. }
                | RequestError::Reserved { .. },
            ) => LogLevel::Warn,
            Err(_) => LogLevel::Error,
        };
        if let Err(error) = outcome {
            self.log(level, &format!("`{name}` failed: {error}"));
        }
    }

    /// The process now running came up, or ended.
    fn running(&self, process: Option<&Process>) {
        *self.memory.lock().unwrap_or_else(|p| p.into_inner()) = process.and_then(Process::memory);
        self.recorder().started_at = process.map(|_| std::time::SystemTime::now());
    }

    fn connection(&self) -> Option<Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn set_connection(&self, connection: Option<Connection>) {
        *self.connection.lock().unwrap_or_else(|p| p.into_inner()) = connection;
    }

    fn set_session(&self, session: Connection) {
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(session);
    }
}

/// Supervises one sidecar. Dropping it stops the sidecar at once.
pub struct Supervisor {
    shared: Arc<Shared>,
    control: mpsc::UnboundedSender<Control>,
    task: JoinHandle<()>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        // The session first: every call the sidecar made is cancelled, so no
        // confirmation it asked for is left open for a process that is gone,
        // whether it was running, starting or being stopped.
        let session = self
            .shared
            .session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(session) = session {
            session.end("The extension was stopped");
        }
        // Aborting drops the process handle, which kills the sidecar.
        self.task.abort();
    }
}

impl Supervisor {
    /// Start supervising: the sidecar is launched at once, on a task of its
    /// own, logging to a log of its own. Must be called within a tokio runtime.
    pub fn start(
        config: SidecarConfig,
        launcher: Arc<dyn Launcher>,
        broker: Arc<dyn Broker>,
    ) -> Supervisor {
        Supervisor::start_with_log(config, launcher, broker, AppLog::new())
    }

    /// [`Supervisor::start`], logging to `log`: the host's log for the app,
    /// which outlives this supervisor, so a sidecar started again after an
    /// update or a disable keeps its app's earlier lines (#575).
    pub fn start_with_log(
        config: SidecarConfig,
        launcher: Arc<dyn Launcher>,
        broker: Arc<dyn Broker>,
        log: AppLog,
    ) -> Supervisor {
        let (status, _) = watch::channel(SidecarStatus::Starting);
        let shared = Arc::new(Shared {
            status,
            policy: config.policy.clone(),
            limits: config.limits.clone(),
            enforcement: launcher.enforcement(),
            connection: Mutex::new(None),
            session: Mutex::new(None),
            log,
            recorder: Mutex::new(Recorder::default()),
            memory: Mutex::new(None),
        });
        let (control, controls) = mpsc::unbounded_channel();
        let task = tokio::spawn(supervise(
            shared.clone(),
            config,
            launcher,
            broker,
            controls,
        ));
        Supervisor {
            shared,
            control,
            task,
        }
    }

    pub fn status(&self) -> SidecarStatus {
        self.shared.status.borrow().clone()
    }

    /// Every change of status, from the current one.
    pub fn watch(&self) -> watch::Receiver<SidecarStatus> {
        self.shared.status.subscribe()
    }

    fn running(&self) -> Result<Connection, RequestError> {
        match self.shared.connection() {
            Some(connection) if !connection.is_ended() => Ok(connection),
            _ => Err(RequestError::Unavailable(self.status().unavailable())),
        }
    }

    /// Send the sidecar a request, under the request limits. Dropping the
    /// returned future cancels it.
    pub async fn request(&self, name: &str, params: Value) -> Result<Value, RequestError> {
        let began = Instant::now();
        let answer = match self.running() {
            Ok(connection) => connection.request(name, params).await,
            Err(refused) => Err(refused),
        };
        self.shared.record(name, began.elapsed(), &answer);
        answer
    }

    /// Open a stream, under the stream limit.
    pub async fn open_stream(
        &self,
        name: &str,
        params: Value,
    ) -> Result<SidecarStream, RequestError> {
        let began = Instant::now();
        let opened = match self.running() {
            Ok(connection) => connection.open_stream(name, params).await,
            Err(refused) => Err(refused),
        };
        self.shared.record(name, began.elapsed(), &opened);
        if opened.is_ok() {
            self.shared.recorder().stream_opened();
        }
        opened
    }

    /// Ask the sidecar `health` now, outside the request limit, with the
    /// policy's health timeout.
    pub async fn health(&self) -> Result<(), RequestError> {
        let timeout = self.shared.policy.health_timeout;
        self.running()?
            .call(method::HEALTH, &json!({}), timeout)
            .await
            .map(|_| ())
    }

    /// Start the sidecar again, with a fresh backoff. What the Restart action
    /// does from `Disabled`; it also restarts a running one.
    pub fn restart(&self) {
        let _ = self.control.send(Control::Restart);
    }

    /// Stop the sidecar: `deactivate`, then `shutdown`, then a grace period
    /// for it to exit before it is killed. Returns once it has ended.
    pub async fn stop(&self) {
        let (done, stopped) = oneshot::channel();
        if self.control.send(Control::Stop(done)).is_ok() {
            let _ = stopped.await;
        }
    }

    /// The recent log: what the sidecar wrote to stderr, and what the
    /// supervisor did. What View logs shows.
    pub fn logs(&self) -> Vec<LogLine> {
        self.shared.log.lines()
    }

    /// The app's log this supervisor writes to.
    pub fn log(&self) -> AppLog {
        self.shared.log.clone()
    }

    /// The sidecar now, as the Inspector reads it (#575).
    pub fn metrics(&self) -> SidecarMetrics {
        let log = &self.shared.log;
        let status = match self.status() {
            SidecarStatus::Restarting {
                attempt,
                of,
                delay,
                reason,
            } => SidecarStatus::Restarting {
                attempt,
                of,
                delay,
                reason: log.redact(&reason),
            },
            SidecarStatus::Disabled { reason } => SidecarStatus::Disabled {
                reason: log.redact(&reason),
            },
            SidecarStatus::Refused { reason } => SidecarStatus::Refused {
                reason: log.redact(&reason),
            },
            other => other,
        };
        let (in_flight, open_streams) = self
            .shared
            .connection()
            .filter(|connection| !connection.is_ended())
            .map(|connection| connection.load())
            .unwrap_or((0, 0));
        let probe = self
            .shared
            .memory
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let memory_bytes = match status {
            SidecarStatus::Running { .. } => probe.and_then(|read| read()),
            _ => None,
        };
        let recorder = self.shared.recorder();
        SidecarMetrics {
            status,
            started_at: recorder.started_at,
            launches: recorder.launches,
            unexpected_exits: recorder.unexpected_exits,
            memory_bytes,
            limits: self.shared.limits.clone(),
            enforcement: self.shared.enforcement.clone(),
            requests: recorder.requests(in_flight),
            open_streams,
            streams_opened: recorder.streams_opened(),
        }
    }
}

impl Inspect for Supervisor {
    fn metrics(&self) -> SidecarMetrics {
        Supervisor::metrics(self)
    }
}

/// How one start ended.
enum Started {
    Running(Running),
    /// It cannot run here; see [`SidecarStatus::Refused`].
    Refused(String),
    /// It did not come up. Counts toward the backoff like a crash.
    Failed(String),
}

/// A sidecar that has been initialized and activated.
struct Running {
    connection: Connection,
    process: Process,
    /// `None` once it has ended: a finished task is not polled again.
    reader: Option<JoinHandle<ReadEnd>>,
    api_version: semver::Version,
}

/// How a running sidecar's run ended.
enum Ended {
    /// Unexpectedly, for this reason.
    Crashed(String),
    /// The host asked it to stop, and it has.
    Stopped(oneshot::Sender<()>),
    /// The host asked for a restart, and it has stopped.
    Restart,
    /// srelens stopped it for what starting it again would not change: its
    /// data directory outgrew its limit or stopped being private.
    Refused(String),
    /// The supervisor is gone.
    Dropped,
}

async fn supervise(
    shared: Arc<Shared>,
    config: SidecarConfig,
    launcher: Arc<dyn Launcher>,
    broker: Arc<dyn Broker>,
    mut controls: mpsc::UnboundedReceiver<Control>,
) {
    let policy = &config.policy;
    // Unexpected exits since the sequence last started over.
    let mut failures = 0usize;
    loop {
        shared.set(SidecarStatus::Starting);
        let reason = match start(&shared, &config, &launcher, &broker).await {
            Started::Running(running) => {
                let started_at = Instant::now();
                // The session before the status: a caller waiting for the sidecar
                // to leave `Starting` calls it at once.
                shared.set_connection(Some(running.connection.clone()));
                shared.running(Some(&running.process));
                shared.set(SidecarStatus::Running {
                    api_version: running.api_version.clone(),
                    pid: running.process.pid(),
                });
                shared.log(LogLevel::Info, "The extension is running");
                let ended = run(&shared, &config, running, &mut controls).await;
                shared.running(None);
                match ended {
                    Ended::Crashed(reason) => {
                        if started_at.elapsed() >= policy.backoff_reset_after {
                            failures = 0;
                        }
                        reason
                    }
                    Ended::Stopped(done) => {
                        shared.set(SidecarStatus::Stopped);
                        let _ = done.send(());
                        match wait_for_restart(&shared, &mut controls).await {
                            true => {
                                failures = 0;
                                continue;
                            }
                            false => return,
                        }
                    }
                    Ended::Restart => {
                        failures = 0;
                        continue;
                    }
                    Ended::Refused(reason) => {
                        shared.log(LogLevel::Error, &reason);
                        shared.set(SidecarStatus::Refused { reason });
                        match wait_for_restart(&shared, &mut controls).await {
                            true => {
                                failures = 0;
                                continue;
                            }
                            false => return,
                        }
                    }
                    Ended::Dropped => return,
                }
            }
            Started::Refused(reason) => {
                shared.log(LogLevel::Error, &reason);
                shared.set(SidecarStatus::Refused { reason });
                match wait_for_restart(&shared, &mut controls).await {
                    true => {
                        failures = 0;
                        continue;
                    }
                    false => return,
                }
            }
            Started::Failed(reason) => reason,
        };
        failures += 1;
        shared.recorder().unexpected_exits += 1;
        shared.log(LogLevel::Error, &reason);
        let Some(&delay) = policy.backoff.get(failures - 1) else {
            shared.log(
                LogLevel::Error,
                &format!(
                    "{UNEXPECTED_EXIT}; srelens will not start it again until it is restarted"
                ),
            );
            shared.set(SidecarStatus::Disabled { reason });
            match wait_for_restart(&shared, &mut controls).await {
                true => {
                    failures = 0;
                    continue;
                }
                false => return,
            }
        };
        shared.log(
            LogLevel::Warn,
            &format!(
                "Restarting the extension in {} (attempt {failures} of {})",
                human(delay),
                policy.backoff.len()
            ),
        );
        shared.set(SidecarStatus::Restarting {
            attempt: failures,
            of: policy.backoff.len(),
            delay,
            reason,
        });
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            control = controls.recv() => match control {
                Some(Control::Restart) => failures = 0,
                Some(Control::Stop(done)) => {
                    shared.set(SidecarStatus::Stopped);
                    let _ = done.send(());
                    match wait_for_restart(&shared, &mut controls).await {
                        true => failures = 0,
                        false => return,
                    }
                }
                None => return,
            },
        }
    }
}

/// Wait in a state nothing leaves on its own. `true` to start again, `false`
/// once the supervisor is gone.
async fn wait_for_restart(
    shared: &Shared,
    controls: &mut mpsc::UnboundedReceiver<Control>,
) -> bool {
    loop {
        match controls.recv().await {
            Some(Control::Restart) => return true,
            // Already stopped: say so at once.
            Some(Control::Stop(done)) => {
                shared.set(SidecarStatus::Stopped);
                let _ = done.send(());
            }
            None => return false,
        }
    }
}

/// Launch, initialize and activate once.
async fn start(
    shared: &Arc<Shared>,
    config: &SidecarConfig,
    launcher: &Arc<dyn Launcher>,
    broker: &Arc<dyn Broker>,
) -> Started {
    if let Enforcement::Missing(why) = launcher.enforcement() {
        return Started::Refused(why);
    }
    if let Err(why) = check_data(config).await {
        return Started::Refused(format!("{why}, so srelens did not start it"));
    }
    // A backend blocks while it launches (systemd's scope, an AppContainer's
    // ACLs), so not on a runtime thread.
    let launch = {
        let (launcher, command, limits) = (
            launcher.clone(),
            config.command.clone(),
            config.limits.clone(),
        );
        tokio::task::spawn_blocking(move || launcher.launch(&command, &limits))
    };
    let launched = match launch.await.unwrap_or_else(|e| {
        Err(LaunchError::Failed(format!(
            "srelens could not start the extension: {e}"
        )))
    }) {
        Ok(launched) => {
            shared.recorder().launches += 1;
            launched
        }
        Err(LaunchError::Unavailable(why)) => return Started::Refused(why),
        Err(LaunchError::Failed(why)) => return Started::Failed(why),
    };
    let (connection, lines) = Connection::new(config.limits.clone());
    shared.set_session(connection.clone());
    tokio::spawn(connection::write(lines, launched.stdin));
    tokio::spawn(read_log(shared.clone(), launched.stderr));
    let mut reader = Some(tokio::spawn({
        let connection = connection.clone();
        let broker = broker.clone();
        let stdout = launched.stdout;
        async move { connection.read(stdout, broker).await }
    }));
    let mut process = launched.process;
    let kill = process.killer();
    let timeout = config.limits.request_timeout;

    let params = protocol::initialize_params(
        SIDECAR_API_VERSIONS,
        &config.limits,
        &config.command.data_dir,
    );
    let session = connection.clone();
    let handshake = async move {
        let initialized = session
            .call(method::INITIALIZE, &params, timeout)
            .await
            .map_err(|e| Failure::of(method::INITIALIZE, e))?;
        let api_version = protocol::negotiated(SIDECAR_API_VERSIONS, &initialized)
            .map_err(Failure::Incompatible)?;
        session.set_api_version(api_version.clone());
        session
            .call(method::ACTIVATE, &json!({}), timeout)
            .await
            .map_err(|e| Failure::of(method::ACTIVATE, e))?;
        Ok::<_, Failure>(api_version)
    };
    tokio::pin!(handshake);
    // The process may end, or break the protocol, during the handshake.
    let outcome = loop {
        tokio::select! {
            biased;
            exit = process.exit() => {
                let why = crashed(&exit.description);
                connection.end(&why);
                return Started::Failed(why);
            }
            end = async { reader.as_mut().expect("guarded").await }, if reader.is_some() => {
                reader = None;
                if let Ok(ReadEnd::Violation(v)) = end {
                    let why = format!("The extension {v}, so srelens stopped it");
                    connection.end(&why);
                    kill();
                    let _ = process.exit().await;
                    return Started::Failed(why);
                }
                // stdout closed: the process is ending, and its exit arm
                // says how. The handshake's deadline bounds a process that
                // closed stdout and lives on.
            }
            outcome = &mut handshake => break outcome,
        }
    };
    let failure = match outcome {
        Ok(api_version) => {
            return Started::Running(Running {
                connection,
                process,
                reader,
                api_version,
            })
        }
        Err(failure) => failure,
    };
    let (why, refused) = match failure {
        Failure::Incompatible(why) => (why, true),
        Failure::Other(why) => (why, false),
        // The session ended mid-call; the process's exit says how.
        Failure::Ended => {
            kill();
            let exit = process.exit().await;
            return Started::Failed(crashed(&exit.description));
        }
    };
    connection.end(&why);
    kill();
    let _ = process.exit().await;
    if refused {
        Started::Refused(why)
    } else {
        Started::Failed(why)
    }
}

enum Failure {
    /// No API version in common: not retried.
    Incompatible(String),
    /// The process ended during the handshake.
    Ended,
    Other(String),
}

impl Failure {
    fn of(name: &str, error: RequestError) -> Failure {
        match error {
            RequestError::Failed(rpc) if name == method::INITIALIZE => {
                match protocol::incompatible(SIDECAR_API_VERSIONS, &rpc) {
                    Some(why) => Failure::Incompatible(why),
                    None => {
                        Failure::Other(format!("The extension refused to {name}: {}", rpc.message))
                    }
                }
            }
            RequestError::Failed(rpc) => {
                Failure::Other(format!("The extension refused to {name}: {}", rpc.message))
            }
            RequestError::TimedOut { after, .. } => Failure::Other(format!(
                "The extension did not answer {name} within {}",
                human(after)
            )),
            RequestError::Ended(_) => Failure::Ended,
            other => Failure::Other(other.to_string()),
        }
    }
}

/// "The extension process exited unexpectedly: it …".
fn crashed(description: &str) -> String {
    format!("The extension process exited unexpectedly: it {description}")
}

/// Keep what the sidecar writes to stderr in its log.
async fn read_log(shared: Arc<Shared>, stderr: Box<dyn tokio::io::AsyncRead + Send + Unpin>) {
    let mut lines = BoundedLines::new(tokio::io::BufReader::new(stderr), LOG_LINE_BYTES);
    loop {
        match lines.next_line().await {
            Ok(Some(Line::Bytes(line))) => shared.log.sidecar(&line),
            Ok(Some(Line::TooLong)) => shared.log(
                LogLevel::Warn,
                &format!("The extension wrote a log line longer than {LOG_LINE_BYTES} bytes, which was dropped"),
            ),
            Ok(None) | Err(_) => return,
        }
    }
}

type HealthCheck =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, RequestError>> + Send>>;

type DataCheck = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>;

/// [`data::check`] of the sidecar's data directory, off the runtime's threads:
/// a directory near its entry limit takes a moment to walk.
fn check_data(config: &SidecarConfig) -> DataCheck {
    let path = config.command.data_dir.clone();
    let limits = config.limits.clone();
    Box::pin(async move {
        tokio::task::spawn_blocking(move || data::check(&path, &limits))
            .await
            .unwrap_or_else(|e| {
                Err(format!(
                    "srelens could not measure the extension's data directory: {e}"
                ))
            })
    })
}

/// Serve a running sidecar until it ends or the host stops it.
async fn run(
    shared: &Shared,
    config: &SidecarConfig,
    running: Running,
    controls: &mut mpsc::UnboundedReceiver<Control>,
) -> Ended {
    let Running {
        connection,
        mut process,
        mut reader,
        ..
    } = running;
    let policy = &config.policy;
    let kill = process.killer();
    let mut health = tokio::time::interval_at(
        Instant::now() + policy.health_interval,
        policy.health_interval,
    );
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The health check in flight, polled beside everything else so that a
    // stop or an exit is not held up by it.
    let mut check: Option<HealthCheck> = None;
    // The data directory is measured on its own interval, the same way.
    let mut measure = tokio::time::interval_at(
        Instant::now() + policy.data_check_interval,
        policy.data_check_interval,
    );
    measure.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut measuring: Option<DataCheck> = None;
    // Why the host stopped the process, when it did.
    let mut stopped_because: Option<String> = None;
    // Set with `stopped_because` when starting it again would not help.
    let mut refused = false;
    let ended = loop {
        tokio::select! {
            biased;
            exit = process.exit() => {
                let why = stopped_because
                    .take()
                    .unwrap_or_else(|| crashed(&exit.description));
                break if refused { Ended::Refused(why) } else { Ended::Crashed(why) };
            }
            end = async { reader.as_mut().expect("guarded").await }, if reader.is_some() => {
                reader = None;
                let why = match end {
                    // Already stopped for another reason: that one stands, and
                    // the exit arm follows.
                    _ if stopped_because.is_some() => continue,
                    Ok(ReadEnd::Violation(v)) => format!("The extension {v}, so srelens stopped it"),
                    // stdout closed. Almost always the process ending; give it
                    // a moment to be reaped, so its exit says how.
                    _ => match tokio::time::timeout(Duration::from_secs(1), process.exit()).await {
                        Ok(exit) => break Ended::Crashed(crashed(&exit.description)),
                        Err(_) => "The extension closed its standard output while still running, so srelens stopped it".to_owned(),
                    },
                };
                connection.end(&why);
                stopped_because = Some(why);
                kill();
            }
            control = controls.recv() => match control {
                Some(Control::Stop(done)) => {
                    stop(shared, config, &connection, &mut process).await;
                    break Ended::Stopped(done);
                }
                Some(Control::Restart) => {
                    stop(shared, config, &connection, &mut process).await;
                    break Ended::Restart;
                }
                None => {
                    kill();
                    break Ended::Dropped;
                }
            },
            _ = health.tick(), if check.is_none() => {
                let connection = connection.clone();
                let timeout = policy.health_timeout;
                check = Some(Box::pin(async move {
                    connection.call(method::HEALTH, &json!({}), timeout).await
                }));
            }
            _ = measure.tick(), if measuring.is_none() && stopped_because.is_none() => {
                measuring = Some(check_data(config));
            }
            measured = async { measuring.as_mut().expect("guarded").await }, if measuring.is_some() => {
                measuring = None;
                let Err(why) = measured else { continue };
                if stopped_because.is_some() {
                    continue;
                }
                let why = format!("{why}, so srelens stopped it");
                connection.end(&why);
                stopped_because = Some(why);
                refused = true;
                kill();
            }
            answer = async { check.as_mut().expect("guarded").await }, if check.is_some() => {
                check = None;
                let why = match answer {
                    Ok(_) => continue,
                    _ if stopped_because.is_some() => continue,
                    Err(RequestError::TimedOut { after, .. }) => format!(
                        "The extension did not answer its health check within {}, so srelens stopped it",
                        human(after)
                    ),
                    Err(RequestError::Failed(rpc)) => format!(
                        "The extension reported itself unhealthy ({}), so srelens stopped it",
                        rpc.message
                    ),
                    // It ended; the exit arm says how.
                    Err(_) => continue,
                };
                connection.end(&why);
                stopped_because = Some(why);
                kill();
            }
        }
    };
    shared.set_connection(None);
    match &ended {
        Ended::Crashed(why) | Ended::Refused(why) => connection.end(why),
        Ended::Dropped => connection.end("The extension was stopped"),
        // `stop` ended it.
        Ended::Stopped(_) | Ended::Restart => {}
    }
    ended
}

/// The graceful stop: no new requests, `deactivate`, `shutdown`, stdin
/// closed, and a grace period to exit before a kill.
async fn stop(
    shared: &Shared,
    config: &SidecarConfig,
    connection: &Connection,
    process: &mut Process,
) {
    shared.set(SidecarStatus::Stopping);
    shared.set_connection(None);
    let grace = config.policy.shutdown_grace;
    let _ = connection.call(method::DEACTIVATE, &json!({}), grace).await;
    let _ = connection.call(method::SHUTDOWN, &json!({}), grace).await;
    connection.close_stdin();
    if tokio::time::timeout(grace, process.exit()).await.is_err() {
        shared.log(
            LogLevel::Warn,
            &format!(
                "The extension did not exit within {} of shutdown, so srelens killed it",
                human(grace)
            ),
        );
        (process.killer())();
        let _ = process.exit().await;
    }
    connection.end("The extension was stopped");
    shared.log(LogLevel::Info, "The extension was stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_disabled_sidecar_offers_actions_and_the_headline() {
        let disabled = SidecarStatus::Disabled {
            reason: "it exited with status 1".into(),
        };
        assert_eq!(disabled.message(), Some(UNEXPECTED_EXIT));
        assert_eq!(disabled.actions(), DISABLED_ACTIONS);
        for other in [
            SidecarStatus::Starting,
            SidecarStatus::Stopped,
            SidecarStatus::Refused {
                reason: "no sandbox".into(),
            },
        ] {
            assert_eq!(other.message(), None, "{other:?}");
            assert!(other.actions().is_empty(), "{other:?}");
        }
    }

    #[test]
    fn a_restarting_sidecar_says_which_attempt_and_when() {
        let status = SidecarStatus::Restarting {
            attempt: 2,
            of: 3,
            delay: Duration::from_secs(5),
            reason: String::new(),
        };
        assert_eq!(
            status.unavailable(),
            "The extension stopped unexpectedly and is restarting (attempt 2 of 3, in 5 s)"
        );
    }
}
