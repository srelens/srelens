//! The hello-world sidecar through its whole life under srelens's own
//! supervisor, with whatever launcher the test gives it.

#![allow(dead_code)]

use serde_json::{json, Value};
use srelens_plugin_host::sidecar::data::DataDir;
use srelens_plugin_host::sidecar::protocol::{code, RpcError};
use srelens_plugin_host::sidecar::{
    Broker, Launcher, Limits, LogLevel, LogSource, Policy, RequestError, SidecarCommand,
    SidecarConfig, SidecarStatus, SidecarStream, StreamEvent, Supervisor,
};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Which example a run starts.
#[derive(Clone, Copy, Debug)]
pub enum Language {
    Rust,
    Go,
}

/// The example's binary. The Rust one is this package's own; the Go one is
/// built by `go build -C sdk/examples/hello-world/go -o <path> .` and named
/// by `SRELENS_HELLO_WORLD_GO`, an absolute path. Its cases are `#[ignore]`d,
/// and one run without the variable fails saying so rather than passing.
pub fn program(language: Language) -> PathBuf {
    match language {
        Language::Rust => PathBuf::from(env!("CARGO_BIN_EXE_hello-world")),
        Language::Go => std::env::var_os("SRELENS_HELLO_WORLD_GO")
            .map(PathBuf::from)
            .expect(
                "set SRELENS_HELLO_WORLD_GO to the Go hello-world binary, built with \
                 `go build -C sdk/examples/hello-world/go -o <path> .`",
            ),
    }
}

const APP_ID: &str = "org.example.hello-world";
const WAIT: Duration = Duration::from_secs(30);

/// Answers `host/read` with rows and what it was asked, as the facade
/// would; refuses anything else.
pub struct Pods;

impl Broker for Pods {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
        Box::pin(async move {
            match method {
                "host/read" => Ok(json!({"rows": [{"name": "web"}], "asked": params})),
                other => Err(RpcError::new(
                    code::METHOD_NOT_FOUND,
                    format!("no `{other}` here"),
                )),
            }
        })
    }
}

pub fn data_dir() -> PathBuf {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("a temporary directory"));
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    DataDir::for_app(&root.path().join(n.to_string()), APP_ID)
        .expect("a data directory")
        .path()
        .to_owned()
}

pub fn config(program: PathBuf, limits: Limits) -> SidecarConfig {
    SidecarConfig {
        command: SidecarCommand {
            app_id: APP_ID.into(),
            program,
            args: Vec::new(),
            env: Vec::new(),
            data_dir: data_dir(),
        },
        limits,
        policy: Policy::default(),
    }
}

async fn until(supervisor: &Supervisor, done: impl Fn(&SidecarStatus) -> bool) -> SidecarStatus {
    let mut status = supervisor.watch();
    let found = match tokio::time::timeout(WAIT, status.wait_for(|s| done(s))).await {
        Ok(Ok(found)) => found.clone(),
        Ok(Err(_)) => panic!("the supervisor is gone"),
        Err(_) => panic!("the status never came; it is {:?}", supervisor.status()),
    };
    found
}

async fn next(stream: &mut SidecarStream) -> Option<StreamEvent> {
    tokio::time::timeout(WAIT, stream.next())
        .await
        .expect("a stream event in time")
}

async fn logged(supervisor: &Supervisor, needle: &str) -> bool {
    for _ in 0..100 {
        if supervisor.logs().iter().any(|line| {
            line.source == LogSource::Sidecar
                && line.level == LogLevel::Info
                && line.text.contains(needle)
        }) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Start, serve every kind of call, stop: what srelens does with a sidecar.
pub async fn whole_life(program: PathBuf, launcher: Arc<dyn Launcher>, limits: Limits) {
    let supervisor = Supervisor::start(config(program, limits), launcher, Arc::new(Pods));
    let running = until(&supervisor, |s| {
        matches!(
            s,
            SidecarStatus::Running { .. } | SidecarStatus::Refused { .. }
        )
    })
    .await;
    assert!(
        matches!(running, SidecarStatus::Running { .. }),
        "{running:?}; logs: {:?}",
        supervisor.logs()
    );

    assert_eq!(
        supervisor
            .request("greet", json!({"name": "srelens"}))
            .await,
        Ok(json!({"greeting": "Hello, srelens"}))
    );
    assert!(
        logged(&supervisor, "greeting srelens").await,
        "the sidecar's log line, at info: {:?}",
        supervisor.logs()
    );

    let pods = supervisor
        .request("pods", json!({"cluster": "kind-dev", "namespace": "team"}))
        .await
        .unwrap();
    assert_eq!(pods["rows"], json!([{"name": "web"}]));
    assert_eq!(
        pods["asked"],
        json!({"context": {"clusterId": "kind-dev", "namespace": "team"}, "capability": "pods"})
    );

    let mut count = supervisor
        .open_stream("count", json!({"to": 3}))
        .await
        .unwrap();
    let mut frames = Vec::new();
    loop {
        match next(&mut count).await {
            Some(StreamEvent::Data(n)) => frames.push(n),
            Some(StreamEvent::Closed) => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(frames, [json!(0), json!(1), json!(2)]);

    let mut long = supervisor
        .open_stream("count", json!({"to": 1_000_000}))
        .await
        .unwrap();
    assert_eq!(next(&mut long).await, Some(StreamEvent::Data(json!(0))));
    drop(long);

    let refused = supervisor
        .request("greet", json!({"nom": "x"}))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, RequestError::Failed(e) if e.code == code::INVALID_PARAMS),
        "{refused:?}"
    );
    assert_eq!(supervisor.health().await, Ok(()));

    supervisor.stop().await;
    assert_eq!(supervisor.status(), SidecarStatus::Stopped);
    assert!(
        supervisor
            .logs()
            .iter()
            .all(|line| !line.text.contains("did not exit")),
        "it exited on its own after shutdown: {:?}",
        supervisor.logs()
    );
}
