//! A fake sidecar that runs as a task inside the test's runtime, behind the
//! public `Launcher` trait. Its pipes are in-memory, so a paused clock only
//! moves when every task is waiting on a timer: a timeout fires at exactly its
//! deadline, and a backoff is exactly its length.

use serde_json::{json, Value};
use srelens_plugin_host::sidecar::{
    Enforcement, Exit, LaunchError, Launched, Launcher, Limits, Process, SidecarCommand,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::Instant;

/// What the fake does with one request.
pub enum Reply {
    Result(Value),
    Error(i64, &'static str),
    /// Never answers.
    Silent,
    /// Never answers, and stops reading: not even a closed stdin ends it.
    Hang,
    /// Writes this line instead of an answer.
    Raw(&'static str),
    /// Answers, then exits with this status.
    ResultThenExit(Value, i32),
    /// Answers after this long, still reading meanwhile.
    Late(std::time::Duration, Value),
    /// Dies as an abort would.
    Crash,
}

/// A request the fake received, and a way to write to its stdout.
pub struct Call<'a> {
    pub method: &'a str,
    pub params: &'a Value,
    /// Which launch this is, from 1.
    pub launch: usize,
    out: &'a mpsc::UnboundedSender<String>,
}

impl Call<'_> {
    pub fn notify(&self, method: &str, params: Value) {
        let _ = self
            .out
            .send(json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string());
    }
}

type Handler = Arc<dyn Fn(&Call) -> Option<Reply> + Send + Sync>;

/// The answers every well-behaved sidecar gives.
fn standard(call: &Call) -> Reply {
    match call.method {
        "initialize" => Reply::Result(json!({"apiVersion": "0.1.0", "sidecar": {"name": "fake"}})),
        "activate" | "deactivate" | "health" => Reply::Result(json!({})),
        "shutdown" => Reply::ResultThenExit(json!({}), 0),
        _ => Reply::Result(call.params.clone()),
    }
}

#[derive(Default)]
struct Record {
    /// Every message the host wrote, across launches, with the launch number.
    received: Vec<(usize, Value)>,
    /// When each launch happened.
    launched_at: Vec<Instant>,
    /// Launches whose process was killed rather than exiting.
    killed: Vec<usize>,
}

#[derive(Clone)]
pub struct FakeLauncher {
    handler: Handler,
    enforcement: Enforcement,
    unavailable: Option<String>,
    stderr: Vec<&'static str>,
    memory: Option<u64>,
    launches: Arc<AtomicUsize>,
    record: Arc<Mutex<Record>>,
}

impl FakeLauncher {
    /// A sidecar that answers as `answer` says, and otherwise as a
    /// well-behaved one does.
    pub fn new(answer: impl Fn(&Call) -> Option<Reply> + Send + Sync + 'static) -> FakeLauncher {
        FakeLauncher {
            handler: Arc::new(answer),
            enforcement: Enforcement::Kernel,
            unavailable: None,
            stderr: Vec::new(),
            memory: None,
            launches: Arc::new(AtomicUsize::new(0)),
            record: Arc::default(),
        }
    }

    pub fn well_behaved() -> FakeLauncher {
        FakeLauncher::new(|_| None)
    }

    pub fn enforcing(mut self, enforcement: Enforcement) -> FakeLauncher {
        self.enforcement = enforcement;
        self
    }

    pub fn unavailable(mut self, why: &str) -> FakeLauncher {
        self.unavailable = Some(why.to_owned());
        self
    }

    /// Writes `line` to stderr when it starts, after any given before.
    pub fn logging(mut self, line: &'static str) -> FakeLauncher {
        self.stderr.push(line);
        self
    }

    /// Reports `bytes` as its memory use, as the Linux backend reads a cgroup's.
    pub fn using_memory(mut self, bytes: u64) -> FakeLauncher {
        self.memory = Some(bytes);
        self
    }

    pub fn launches(&self) -> usize {
        self.launches.load(Ordering::SeqCst)
    }

    pub fn launched_at(&self) -> Vec<Instant> {
        self.record.lock().unwrap().launched_at.clone()
    }

    pub fn killed(&self) -> Vec<usize> {
        self.record.lock().unwrap().killed.clone()
    }

    /// The methods the host sent, in order, across launches.
    pub fn methods(&self) -> Vec<String> {
        self.received()
            .iter()
            .filter_map(|(_, m)| m["method"].as_str().map(str::to_owned))
            .collect()
    }

    pub fn received(&self) -> Vec<(usize, Value)> {
        self.record.lock().unwrap().received.clone()
    }
}

impl Launcher for FakeLauncher {
    fn enforcement(&self) -> Enforcement {
        self.enforcement.clone()
    }

    fn launch(&self, _command: &SidecarCommand, _limits: &Limits) -> Result<Launched, LaunchError> {
        if let Some(why) = &self.unavailable {
            return Err(LaunchError::Unavailable(why.clone()));
        }
        let launch = self.launches.fetch_add(1, Ordering::SeqCst) + 1;
        self.record.lock().unwrap().launched_at.push(Instant::now());
        let (stdin, host_stdin) = tokio::io::duplex(1 << 16);
        let (host_stdout, stdout) = tokio::io::duplex(1 << 16);
        let (host_stderr, mut stderr) = tokio::io::duplex(1 << 16);
        let handler = self.handler.clone();
        let record = self.record.clone();
        let log = self.stderr.clone();
        let task = tokio::spawn(async move {
            for line in log {
                let _ = stderr.write_all(format!("{line}\n").as_bytes()).await;
            }
            let (out, mut lines) = mpsc::unbounded_channel::<String>();
            let mut stdout = stdout;
            let writer = tokio::spawn(async move {
                while let Some(line) = lines.recv().await {
                    if stdout
                        .write_all(format!("{line}\n").as_bytes())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let mut input = BufReader::new(host_stdin).lines();
            let status = loop {
                let Ok(Some(line)) = input.next_line().await else {
                    // The host closed stdin: exit as a sidecar should.
                    break 0;
                };
                let message: Value = serde_json::from_str(&line).expect("the host writes JSON");
                record
                    .lock()
                    .unwrap()
                    .received
                    .push((launch, message.clone()));
                let Some(method) = message["method"].as_str() else {
                    continue;
                };
                let Some(id) = message.get("id").cloned() else {
                    continue;
                };
                let call = Call {
                    method,
                    params: &message["params"],
                    launch,
                    out: &out,
                };
                let reply = handler(&call).unwrap_or_else(|| standard(&call));
                let answer = |result: Value| {
                    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
                };
                match reply {
                    Reply::Result(result) => {
                        let _ = out.send(answer(result));
                    }
                    Reply::Error(code, text) => {
                        let error = json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}});
                        let _ = out.send(error.to_string());
                    }
                    Reply::Silent => {}
                    Reply::Raw(line) => {
                        let _ = out.send(line.to_owned());
                    }
                    Reply::ResultThenExit(result, status) => {
                        let _ = out.send(answer(result));
                        break status;
                    }
                    Reply::Late(after, result) => {
                        let out = out.clone();
                        let line = answer(result);
                        tokio::spawn(async move {
                            tokio::time::sleep(after).await;
                            let _ = out.send(line);
                        });
                    }
                    Reply::Hang => std::future::pending::<()>().await,
                    Reply::Crash => break -6,
                }
            };
            drop(out);
            let _ = writer.await;
            status
        });
        let killer = Arc::new(Killer {
            task: task.abort_handle(),
            record: self.record.clone(),
            launch,
        });
        let exit = async move {
            match task.await {
                Ok(-6) => exit_by_signal(6, "SIGABRT"),
                Ok(status) => Exit {
                    description: format!("exited with status {status}"),
                    code: Some(status),
                    signal: None,
                    memory_limit: false,
                },
                Err(_) => exit_by_signal(9, "SIGKILL"),
            }
        };
        let mut process = Process::new(Some(launch as u32), exit, move || killer.kill());
        if let Some(bytes) = self.memory {
            process = process.with_memory(move || Some(bytes));
        }
        Ok(Launched {
            stdin: Box::new(stdin),
            stdout: Box::new(host_stdout),
            stderr: Box::new(host_stderr),
            process,
        })
    }
}

fn exit_by_signal(signal: i32, name: &str) -> Exit {
    Exit {
        description: format!("was killed by signal {signal} ({name})"),
        code: None,
        signal: Some(signal),
        memory_limit: false,
    }
}

/// Kills the fake when told to, and when the last handle to its process
/// goes, as a real backend's child is killed; records each kill of a fake
/// that was still running.
struct Killer {
    task: tokio::task::AbortHandle,
    record: Arc<Mutex<Record>>,
    launch: usize,
}

impl Killer {
    fn kill(&self) {
        if self.task.is_finished() {
            return;
        }
        let mut record = self.record.lock().unwrap();
        if !record.killed.contains(&self.launch) {
            record.killed.push(self.launch);
        }
        self.task.abort();
    }
}

impl Drop for Killer {
    fn drop(&mut self) {
        self.kill();
    }
}
