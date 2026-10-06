//! Executable apps' sidecars in this process (#574): one supervisor (#572) per app, at
//! the revision it was started for.
//!
//! A sidecar starts on its app's first operation call here, not at install: a process
//! that never calls one never runs it. Before every start, the restarts after a crash
//! included, its binary is checked against the digest list its package was unpacked
//! with ([`Verifying`]), so a file changed on disk since the install is refused, not
//! run. It gets its app's data directory (#573) and none of this process's environment,
//! and it calls back into the host only through the broker (#573) the MCP host set up
//! ([`SidecarHost`]). Its log and its process are the Inspector's to read (#575).
//!
//! An announced inventory write ([`AppSidecars::reconcile`]) stops every sidecar whose
//! app is gone, off, blocked, quarantined or at another revision; its callers still
//! waiting are answered at once. The next call to an updated app starts its new version.
use super::inspector::AppRuntime;
use super::{package, Installed, Inventory};
use serde_json::{Map, Value};
use srelens_capability::audit::AuditSink;
use srelens_capability::{CapabilityError, Registry};
use srelens_plugin_host::sidecar::data::DataDir;
use srelens_plugin_host::sidecar::{
    AppIdentity, Broker, CapabilityBroker, CgroupRoot, Consent, Enforcement, LaunchError, Launched,
    Launcher, Limits, NoBroker, OsSandbox, Policy, SandboxConfig, SidecarCommand, SidecarConfig,
    SidecarStatus, Supervisor,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Why a host with nowhere to keep an app's files runs no sidecar: the web host (#515).
pub(super) const NO_FILES: &str =
    "This host keeps no files for its apps, so it runs no executable apps";

/// The longest a call waits for a sidecar that is starting. Its handshake has timeouts
/// of its own (`initialize` and `activate`, each the request timeout); this bounds the
/// wait if the supervisor is somehow slower.
const START_WAIT: Duration = Duration::from_secs(90);

/// Names the trusted launcher, `srelens-sandbox-launch`, when it is not beside the
/// srelens binary (Linux and macOS).
const LAUNCHER_ENV: &str = "SRELENS_SANDBOX_LAUNCHER";
/// Names the cgroup v2 directory delegated to srelens for its sidecars (Linux), set up
/// by hand where there is no systemd user session. Without it srelens asks the systemd
/// user manager for a delegated scope (`CgroupRoot::SystemdScope`).
const CGROUP_ENV: &str = "SRELENS_SANDBOX_CGROUP_ROOT";

/// Where the sandbox finds what it needs on this machine.
fn sandbox_config() -> SandboxConfig {
    sandbox_config_from(|name| std::env::var_os(name), std::env::current_exe().ok())
}

/// [`sandbox_config`], from a variable lookup and srelens's own binary.
fn sandbox_config_from(
    var: impl Fn(&str) -> Option<OsString>,
    exe: Option<PathBuf>,
) -> SandboxConfig {
    let named = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let launcher = named(LAUNCHER_ENV).or_else(|| {
        let beside = exe?.with_file_name("srelens-sandbox-launch");
        beside.is_file().then_some(beside)
    });
    let cgroup = named(CGROUP_ENV).map_or(CgroupRoot::SystemdScope, CgroupRoot::Delegated);
    SandboxConfig { launcher, cgroup }
}

/// Starts a sidecar through `inner` only after checking its binary against the digest
/// list its package was unpacked with, at every launch: the supervisor relaunches the
/// same path after each crash, and a file replaced in between must not run unchecked.
/// A binary that no longer matches is refused, as a missing sandbox is, and the
/// supervisor does not retry it.
///
/// The check reads the whole binary on the supervisor's task. What it cannot close is
/// the moment between the hash and the exec: binding the exec to the checked bytes
/// needs an exec by descriptor, which the three launch backends do not share. The
/// directory is the owner's alone (`0700`), so whatever could swap the file in that
/// moment already runs as the person, with everything that grants.
struct Verifying {
    inner: Arc<dyn Launcher>,
    binary: InstalledBinary,
}

/// Which file of which installed version a sidecar runs.
struct InstalledBinary {
    packages: PathBuf,
    id: String,
    digest: String,
    /// Its package path, `bin/<platform>/<file>`.
    path: String,
}

impl Launcher for Verifying {
    fn enforcement(&self) -> Enforcement {
        self.inner.enforcement()
    }

    fn launch(&self, command: &SidecarCommand, limits: &Limits) -> Result<Launched, LaunchError> {
        let binary = &self.binary;
        let program =
            package::installed_binary(&binary.packages, &binary.id, &binary.digest, &binary.path)
                .map_err(LaunchError::Unavailable)?;
        if program != command.program {
            return Err(LaunchError::Unavailable(
                "The sidecar to start is not the binary its package names".into(),
            ));
        }
        self.inner.launch(command, limits)
    }
}

/// What the host answers a sidecar's calls with (#573): its registry, with the
/// `extensions.*` facade the broker calls; who confirms a call that needs a person;
/// and the audit trail every write is recorded in. An MCP host sets it once it holds
/// its registry (`AppTools::serve_sidecars`); until then a sidecar's calls are refused.
#[derive(Clone)]
pub struct SidecarHost {
    pub registry: Arc<Registry>,
    /// The desktop's host confirmation (#552), or `NoConsent` where nobody can be
    /// asked: a headless process, whose sidecars' writes are then all refused.
    pub consent: Arc<dyn Consent>,
    pub audit: Arc<dyn AuditSink>,
}

/// The sidecar of one app, at the revision it was started for.
struct Running {
    revision: u64,
    supervisor: Arc<Supervisor>,
}

/// The sidecars of one inventory's executable apps.
pub(super) struct AppSidecars {
    /// Where installed packages are unpacked; `None` on a host that keeps no app files.
    packages: Option<PathBuf>,
    /// The root of the apps' data directories (`Apps::data_root`, #573).
    data: Option<PathBuf>,
    /// Each app's log and the Inspector's view of its process (#575).
    runtime: Arc<AppRuntime>,
    /// What each sidecar's broker answers with; `None` until a host says.
    host: Mutex<Option<SidecarHost>>,
    /// The sandbox every sidecar is started in; replaced by tests.
    launcher: Mutex<Arc<dyn Launcher>>,
    running: Mutex<HashMap<String, Running>>,
}

impl AppSidecars {
    pub(super) fn new(
        packages: Option<PathBuf>,
        data: Option<PathBuf>,
        runtime: Arc<AppRuntime>,
    ) -> Self {
        Self {
            packages,
            data,
            runtime,
            host: Mutex::default(),
            launcher: Mutex::new(Arc::new(OsSandbox::new(sandbox_config()))),
            running: Mutex::default(),
        }
    }

    /// Answer the calls of every sidecar started from now on through `host`. A sidecar
    /// already running keeps the broker it was started with.
    pub(super) fn serve(&self, host: SidecarHost) {
        *self.host.lock().unwrap() = Some(host);
    }

    /// Start sidecars with `launcher` from now on. Test support.
    #[cfg(test)]
    pub(super) fn script(&self, launcher: Arc<dyn Launcher>) {
        *self.launcher.lock().unwrap() = launcher;
    }

    /// What `app`'s sidecar answers to `operation` with `input`, which the caller has
    /// checked against the operation's declared inputs. The sidecar is started first if
    /// it is not running at the app's revision.
    pub(super) async fn request(
        &self,
        app: &Installed,
        operation: &str,
        input: Map<String, Value>,
    ) -> Result<Value, CapabilityError> {
        let supervisor = self.supervisor(app).await?;
        // A call to a sidecar that is still coming up waits for it, rather than being
        // told to try again; one that is restarting, disabled or refused is told why.
        let mut status = supervisor.watch();
        let _ = tokio::time::timeout(
            START_WAIT,
            status.wait_for(|status| !matches!(status, SidecarStatus::Starting)),
        )
        .await;
        supervisor
            .request(operation, Value::Object(input))
            .await
            .map_err(|error| CapabilityError::Handler(error.to_string()))
    }

    /// The supervisor of `app` at its revision, started now if there is none.
    pub(super) async fn open_stream(&self, app: &Installed, method: &str, params: Map<String, Value>) -> Result<srelens_plugin_host::sidecar::SidecarStream, CapabilityError> {
        let supervisor = self.supervisor(app).await?;
        let mut status = supervisor.watch();
        let _ = tokio::time::timeout(START_WAIT, status.wait_for(|status| !matches!(status, SidecarStatus::Starting))).await;
        supervisor.open_stream(method, Value::Object(params)).await.map_err(|error| CapabilityError::Handler(error.to_string()))
    }

    async fn supervisor(&self, app: &Installed) -> Result<Arc<Supervisor>, CapabilityError> {
        if let Some(supervisor) = self.current(app) {
            return Ok(supervisor);
        }
        // Checking the binary reads it whole; not on an executor thread, and not under
        // the lock.
        let (packages, data, checked) = (self.packages.clone(), self.data.clone(), app.clone());
        let (config, binary) = tokio::task::spawn_blocking(move || {
            config_for(packages.as_deref(), data.as_deref(), &checked)
        })
        .await
        .map_err(|e| CapabilityError::Handler(e.to_string()))?
        .map_err(CapabilityError::Handler)?;
        let launcher: Arc<dyn Launcher> = Arc::new(Verifying {
            inner: self.launcher.lock().unwrap().clone(),
            binary,
        });
        let broker: Arc<dyn Broker> = match self.host.lock().unwrap().clone() {
            Some(host) => Arc::new(CapabilityBroker::new(
                host.registry,
                identity(app),
                host.audit,
                host.consent,
            )),
            // No host has said what answers a sidecar: every call it makes is refused.
            None => Arc::new(NoBroker),
        };
        let id = &app.manifest.id;
        let mut running = self.running.lock().unwrap();
        // Another call may have started it meanwhile.
        if let Some(current) = running
            .get(id)
            .filter(|current| current.revision == app.revision)
        {
            return Ok(current.supervisor.clone());
        }
        let supervisor = Arc::new(Supervisor::start_with_log(
            config,
            launcher,
            broker,
            self.runtime.log(id),
        ));
        self.runtime.attach(id, supervisor.clone());
        let replaced = running.insert(
            id.clone(),
            Running {
                revision: app.revision,
                supervisor: supervisor.clone(),
            },
        );
        if let Some(replaced) = replaced {
            stop(replaced.supervisor);
        }
        Ok(supervisor)
    }

    fn current(&self, app: &Installed) -> Option<Arc<Supervisor>> {
        self.running
            .lock()
            .unwrap()
            .get(&app.manifest.id)
            .filter(|current| current.revision == app.revision)
            .map(|current| current.supervisor.clone())
    }

    /// Stop every sidecar `state` no longer runs: its app is gone, off (by a person, by
    /// policy or by quarantine), or at another revision.
    pub(super) fn reconcile(&self, state: &Inventory) {
        let mut running = self.running.lock().unwrap();
        let ended: Vec<String> = running
            .iter()
            .filter(|(id, current)| {
                !state.plugins.iter().any(|app| {
                    &app.manifest.id == *id && app.revision == current.revision && app.runs()
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ended {
            if let Some(ended) = running.remove(&id) {
                self.runtime.detach(&id);
                stop(ended.supervisor);
            }
        }
    }

    /// End at once the sidecar of every app `state` no longer holds, before its data
    /// directory and, on Windows, its AppContainer profile are removed: the last handle
    /// dropped kills the process. A call still in flight holds its own handle, and the
    /// process ends when that call does.
    pub(super) fn end_uninstalled(&self, state: &Inventory) {
        let installed = |id: &String| state.plugins.iter().any(|app| &app.manifest.id == id);
        let mut running = self.running.lock().unwrap();
        let gone: Vec<String> = running
            .keys()
            .filter(|id| !installed(id))
            .cloned()
            .collect();
        for id in gone {
            running.remove(&id);
            self.runtime.detach(&id);
        }
    }

    /// The status of `id`'s sidecar in this process, if one was started. Test support.
    #[cfg(test)]
    pub(super) fn status(&self, id: &str) -> Option<SidecarStatus> {
        self.running
            .lock()
            .unwrap()
            .get(id)
            .map(|current| current.supervisor.status())
    }
}

/// Who a sidecar is, for its broker: the app and revision it was started for, and the
/// name and publisher a confirmation names it by — the publisher the host verified on
/// this read (#559), or `None` for an unsigned app.
fn identity(app: &Installed) -> AppIdentity {
    AppIdentity {
        id: app.manifest.id.clone(),
        revision: app.revision,
        name: app.manifest.name.clone(),
        publisher: app
            .signed_by
            .as_ref()
            .filter(|_| app.quarantined.is_none())
            .map(|signer| signer.name.clone()),
    }
}

/// Stops `supervisor`'s sidecar: gracefully when there is a runtime to wait on, else at
/// once, which dropping the last handle does. Callers still waiting are answered with why.
fn stop(supervisor: Arc<Supervisor>) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move { supervisor.stop().await });
    }
}

/// What starting `app`'s sidecar runs, and the installed file each launch checks it
/// against, or why it cannot run here.
fn config_for(
    packages: Option<&Path>,
    data: Option<&Path>,
    app: &Installed,
) -> Result<(SidecarConfig, InstalledBinary), String> {
    let (Some(packages), Some(data)) = (packages, data) else {
        return Err(NO_FILES.into());
    };
    let manifest = &app.manifest;
    let platform = srelens_plugin_host::host_platform().ok_or_else(|| {
        format!(
            "srelens runs no executable apps on {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let binary = manifest
        .sidecar_binary(platform)
        .ok_or_else(|| format!("{} ships no sidecar for {platform}", manifest.name))?;
    let digest = app
        .package
        .as_deref()
        .ok_or("This app has no package to run its sidecar from")?;
    // Checked here too, so a changed binary is the call's refusal rather than a
    // supervisor that starts only to refuse.
    let program = package::installed_binary(packages, &manifest.id, digest, binary)?;
    // The app's own directory under the data root (#573): private, and removed with it.
    let data_dir = DataDir::for_app(data, &manifest.id)
        .map_err(|e| format!("Could not open the app's data directory: {e}"))?
        .path()
        .to_path_buf();
    let config = SidecarConfig {
        command: SidecarCommand {
            app_id: manifest.id.clone(),
            program,
            args: Vec::new(),
            // Nothing of this process's: it may hold KUBECONFIG, cloud credentials or
            // tokens. The backends add only what the OS itself needs.
            env: Vec::new(),
            data_dir,
        },
        limits: Limits::default(),
        policy: Policy::default(),
    };
    let binary = InstalledBinary {
        packages: packages.to_path_buf(),
        id: manifest.id.clone(),
        digest: digest.to_owned(),
        path: binary.to_owned(),
    };
    Ok((config, binary))
}

/// A sidecar that runs as a task in the test's runtime, behind the public `Launcher`
/// trait: it answers the lifecycle as a well-behaved one does, and every operation with
/// what it was asked.
#[cfg(test)]
pub(super) mod fake {
    use serde_json::{json, Value};
    use srelens_plugin_host::sidecar::{
        Enforcement, Exit, LaunchError, Launched, Launcher, Limits, Process, SidecarCommand,
    };
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[derive(Default)]
    struct Record {
        commands: Vec<SidecarCommand>,
        methods: Vec<(usize, String)>,
        exited: Vec<usize>,
    }

    /// Answers `scan` with its params and the launch it came to; `fail` with an error.
    #[derive(Clone, Default)]
    pub struct FakeSidecar {
        record: Arc<Mutex<Record>>,
        /// Refuse to launch, as a machine with no sandbox does.
        pub unavailable: Option<String>,
    }

    impl FakeSidecar {
        /// One that refuses to launch, as a machine with no sandbox does.
        pub fn unavailable(why: &str) -> Self {
            Self {
                unavailable: Some(why.into()),
                ..Self::default()
            }
        }
        pub fn launches(&self) -> usize {
            self.record.lock().unwrap().commands.len()
        }
        pub fn commands(&self) -> Vec<SidecarCommand> {
            self.record.lock().unwrap().commands.clone()
        }
        /// The methods each launch was sent, in order.
        pub fn methods(&self) -> Vec<(usize, String)> {
            self.record.lock().unwrap().methods.clone()
        }
        /// The launches that have ended.
        pub fn exited(&self) -> Vec<usize> {
            self.record.lock().unwrap().exited.clone()
        }
    }

    impl Launcher for FakeSidecar {
        fn enforcement(&self) -> Enforcement {
            Enforcement::Kernel
        }

        fn launch(
            &self,
            command: &SidecarCommand,
            _limits: &Limits,
        ) -> Result<Launched, LaunchError> {
            if let Some(why) = &self.unavailable {
                return Err(LaunchError::Unavailable(why.clone()));
            }
            let launch = {
                let mut record = self.record.lock().unwrap();
                record.commands.push(command.clone());
                record.commands.len()
            };
            let (stdin, host_stdin) = tokio::io::duplex(1 << 16);
            let (host_stdout, mut stdout) = tokio::io::duplex(1 << 16);
            let (host_stderr, _stderr) = tokio::io::duplex(1 << 16);
            let record = self.record.clone();
            let task = tokio::spawn(async move {
                let _stderr = _stderr;
                let mut lines = BufReader::new(host_stdin).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let message: Value = serde_json::from_str(&line).unwrap();
                    let (Some(method), Some(id)) =
                        (message["method"].as_str(), message.get("id").cloned())
                    else {
                        continue;
                    };
                    record
                        .lock()
                        .unwrap()
                        .methods
                        .push((launch, method.to_owned()));
                    let answer = match method {
                        "stream/open" => {
                            let stream = message["params"]["stream"].clone();
                            let data = json!({"jsonrpc":"2.0","method":"stream/data","params":{"stream":stream,"data":{"state":"scanning"}}});
                            if stdout.write_all(format!("{data}\n").as_bytes()).await.is_err() { break; }
                            json!({"result":{}})
                        }
                        "initialize" => json!({"result": {"apiVersion": "0.1.0"}}),
                        "activate" | "deactivate" | "health" | "shutdown" => {
                            json!({"result": {}})
                        }
                        "fail" => json!({"error": {"code": 1, "message": "the scan failed"}}),
                        // Dies without answering, as a crash would.
                        "crash" => break,
                        // Makes one call back into the host (#573): `method`, with
                        // `params` as JSON text, and answers with what came back.
                        "relay" => {
                            let params = &message["params"];
                            let call = json!({"jsonrpc": "2.0", "id": "host-1",
                                "method": params["method"],
                                "params": serde_json::from_str::<Value>(
                                    params["params"].as_str().unwrap_or("{}")).unwrap()});
                            if stdout
                                .write_all(format!("{call}\n").as_bytes())
                                .await
                                .is_err()
                            {
                                break;
                            }
                            let mut came_back = Value::Null;
                            while let Ok(Some(line)) = lines.next_line().await {
                                let reply: Value = serde_json::from_str(&line).unwrap();
                                if reply["id"] == "host-1" && reply.get("method").is_none() {
                                    came_back = reply;
                                    break;
                                }
                            }
                            match came_back.get("error") {
                                Some(error) => json!({"result": {"refused": error}}),
                                None => json!({"result": {"answer": came_back["result"]}}),
                            }
                        }
                        _ => json!({"result": {"operation": method,
                            "params": message["params"], "launch": launch}}),
                    };
                    let mut answer = answer;
                    answer["jsonrpc"] = json!("2.0");
                    answer["id"] = id;
                    if stdout
                        .write_all(format!("{answer}\n").as_bytes())
                        .await
                        .is_err()
                        || method == "shutdown"
                    {
                        break;
                    }
                }
                record.lock().unwrap().exited.push(launch);
            });
            let abort = task.abort_handle();
            let exit = async move {
                let _ = task.await;
                Exit {
                    description: "exited with status 0".into(),
                    code: Some(0),
                    signal: None,
                    memory_limit: false,
                }
            };
            Ok(Launched {
                stdin: Box::new(stdin),
                stdout: Box::new(host_stdout),
                stderr: Box::new(host_stderr),
                process: Process::new(Some(launch as u32), exit, move || abort.abort()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::executable_tests::{install_scanner, SCANNER};
    use super::super::read;
    use super::super::tests::configure;
    use super::fake::FakeSidecar;
    use super::*;
    use serde_json::json;

    #[test]
    fn without_a_named_cgroup_srelens_asks_systemd_for_one() {
        let unset = |_: &str| None;
        assert_eq!(
            sandbox_config_from(unset, None).cgroup,
            CgroupRoot::SystemdScope
        );
        let empty = |name: &str| (name == CGROUP_ENV).then(OsString::new);
        assert_eq!(
            sandbox_config_from(empty, None).cgroup,
            CgroupRoot::SystemdScope
        );
        let named = |name: &str| (name == CGROUP_ENV).then(|| OsString::from("/sys/fs/cgroup/x"));
        assert_eq!(
            sandbox_config_from(named, None).cgroup,
            CgroupRoot::Delegated("/sys/fs/cgroup/x".into())
        );
    }

    #[test]
    fn the_launcher_is_the_named_one_or_the_one_beside_srelens() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("srelens");
        let unset = |_: &str| None;
        assert_eq!(sandbox_config_from(unset, Some(exe.clone())).launcher, None);
        let beside = dir.path().join("srelens-sandbox-launch");
        std::fs::write(&beside, "").unwrap();
        assert_eq!(
            sandbox_config_from(unset, Some(exe.clone())).launcher,
            Some(beside)
        );
        let named = |name: &str| (name == LAUNCHER_ENV).then(|| OsString::from("/opt/launcher"));
        assert_eq!(
            sandbox_config_from(named, Some(exe)).launcher,
            Some(PathBuf::from("/opt/launcher"))
        );
    }

    fn sidecars(path: &Path, fake: &FakeSidecar) -> AppSidecars {
        let sidecars = AppSidecars::new(
            Some(path.with_extension("packages")),
            Some(path.with_extension("data")),
            Arc::default(),
        );
        sidecars.script(Arc::new(fake.clone()));
        sidecars
    }

    fn scanner(path: &Path) -> Installed {
        read(path)
            .unwrap()
            .plugins
            .into_iter()
            .find(|app| app.manifest.id == SCANNER)
            .unwrap()
    }

    fn scan(image: &str) -> Map<String, Value> {
        json!({ "image": image }).as_object().unwrap().clone()
    }

    async fn eventually(what: &str, check: impl Fn() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{what}");
    }

    #[tokio::test]
    async fn a_sidecar_starts_on_its_first_call_with_its_binary_its_directory_and_no_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install_scanner(&path);
        let fake = FakeSidecar::default();
        let sidecars = sidecars(&path, &fake);
        // Installing started nothing.
        assert_eq!(fake.launches(), 0);
        let app = scanner(&path);
        let answer = sidecars.request(&app, "scan", scan("nginx")).await.unwrap();
        assert_eq!(
            answer,
            json!({"operation": "scan", "params": {"image": "nginx"}, "launch": 1})
        );
        // The same sidecar answers the next call.
        sidecars.request(&app, "scan", scan("redis")).await.unwrap();
        assert_eq!(fake.launches(), 1);
        let command = &fake.commands()[0];
        assert_eq!(command.app_id, SCANNER);
        assert!(command.env.is_empty(), "{:?}", command.env);
        assert!(command.args.is_empty());
        let platform = srelens_plugin_host::host_platform().unwrap();
        assert!(
            command
                .program
                .ends_with(Path::new("bin").join(platform).join("scanner")),
            "{}",
            command.program.display()
        );
        // The app's own data directory (#573), named for its ID.
        assert_eq!(
            command.data_dir,
            path.with_extension("data")
                .join(srelens_plugin_host::sidecar::data::directory_name(SCANNER))
        );
        assert!(command.data_dir.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&command.data_dir)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let methods: Vec<String> = fake.methods().into_iter().map(|(_, m)| m).collect();
        assert_eq!(methods, ["initialize", "activate", "scan", "scan"]);
    }

    #[tokio::test]
    async fn a_binary_changed_since_the_install_is_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install_scanner(&path);
        let fake = FakeSidecar::default();
        let sidecars = sidecars(&path, &fake);
        let app = scanner(&path);
        let platform = srelens_plugin_host::host_platform().unwrap();
        let binary = path
            .with_extension("packages")
            .join(SCANNER)
            .join(app.package.as_deref().unwrap())
            .join("bin")
            .join(platform)
            .join("scanner");
        std::fs::write(&binary, b"#!/bin/sh\necho swapped\n").unwrap();
        let refused = sidecars
            .request(&app, "scan", scan("nginx"))
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("no longer matches its digest"),
            "{refused}"
        );
        assert_eq!(fake.launches(), 0);
    }

    /// The supervisor relaunches after a crash; a binary replaced in between is refused
    /// at that relaunch, not run.
    #[tokio::test]
    async fn a_binary_changed_after_a_crash_is_refused_at_the_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install_scanner(&path);
        let fake = FakeSidecar::default();
        let sidecars = sidecars(&path, &fake);
        let app = scanner(&path);
        sidecars.request(&app, "scan", scan("nginx")).await.unwrap();
        let platform = srelens_plugin_host::host_platform().unwrap();
        let binary = path
            .with_extension("packages")
            .join(SCANNER)
            .join(app.package.as_deref().unwrap())
            .join("bin")
            .join(platform)
            .join("scanner");
        std::fs::write(&binary, b"#!/bin/sh\necho swapped\n").unwrap();
        // It dies; the supervisor waits out its first backoff and launches again.
        assert!(sidecars.request(&app, "crash", Map::new()).await.is_err());
        for _ in 0..500 {
            if matches!(
                sidecars.status(SCANNER),
                Some(SidecarStatus::Refused { .. })
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        match sidecars.status(SCANNER) {
            Some(SidecarStatus::Refused { reason }) => {
                assert!(reason.contains("no longer matches its digest"), "{reason}")
            }
            other => panic!("the restart was not refused: {other:?}"),
        }
        assert_eq!(fake.launches(), 1, "the swapped binary was launched");
    }

    #[tokio::test]
    async fn a_sidecar_the_sandbox_cannot_start_is_refused_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install_scanner(&path);
        let fake = FakeSidecar::unavailable("srelens has no sandbox for executable apps on plan9");
        let sidecars = sidecars(&path, &fake);
        let refused = sidecars
            .request(&scanner(&path), "scan", scan("nginx"))
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("on plan9"), "{refused}");
        assert!(matches!(
            sidecars.status(SCANNER),
            Some(SidecarStatus::Refused { .. })
        ));
    }

    #[tokio::test]
    async fn an_error_from_the_sidecar_is_the_calls_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install_scanner(&path);
        let fake = FakeSidecar::default();
        let sidecars = sidecars(&path, &fake);
        let failed = sidecars
            .request(&scanner(&path), "fail", Map::new())
            .await
            .unwrap_err();
        assert!(
            matches!(&failed, CapabilityError::Handler(why)
                if why == "The extension answered with an error: the scan failed"),
            "{failed}"
        );
    }

    #[tokio::test]
    async fn disabling_updating_or_removing_the_app_stops_its_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install_scanner(&path);
        let fake = FakeSidecar::default();
        let sidecars = sidecars(&path, &fake);
        sidecars
            .request(&scanner(&path), "scan", scan("a"))
            .await
            .unwrap();

        // Disabled: stopped, with the lifecycle's own goodbye.
        let state = configure(
            &path,
            json!({"action":"enable","id":SCANNER,"enabled":false}),
        )
        .unwrap();
        sidecars.reconcile(&state);
        eventually("the disabled app's sidecar exits", || fake.exited() == [1]).await;
        let methods: Vec<String> = fake.methods().into_iter().map(|(_, m)| m).collect();
        assert!(
            methods.ends_with(&["deactivate".into(), "shutdown".into()]),
            "{methods:?}"
        );
        assert!(sidecars.status(SCANNER).is_none());

        // Enabled again: a new sidecar on the next call, not before.
        let state = configure(
            &path,
            json!({"action":"enable","id":SCANNER,"enabled":true}),
        )
        .unwrap();
        sidecars.reconcile(&state);
        assert_eq!(fake.launches(), 1);
        sidecars
            .request(&scanner(&path), "scan", scan("b"))
            .await
            .unwrap();
        assert_eq!(fake.launches(), 2);

        // Updated: the old revision's sidecar goes with the announcement, and the next
        // call starts the new revision's.
        let before = scanner(&path).revision;
        let updated = install_scanner(&path);
        assert_ne!(before, updated);
        sidecars.reconcile(&read(&path).unwrap());
        eventually("the old revision's sidecar exits", || {
            fake.exited().contains(&2)
        })
        .await;
        let answer = sidecars
            .request(&scanner(&path), "scan", scan("c"))
            .await
            .unwrap();
        assert_eq!(answer["launch"], 3);

        // Removed: stopped, and its data directory goes with it.
        let data = path
            .with_extension("data")
            .join(srelens_plugin_host::sidecar::data::directory_name(SCANNER));
        assert!(data.is_dir());
        let state = configure(&path, json!({"action":"remove","id":SCANNER})).unwrap();
        sidecars.reconcile(&state);
        eventually("the removed app's sidecar exits", || {
            fake.exited().contains(&3)
        })
        .await;
        assert!(!data.exists());
    }
}
