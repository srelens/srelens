//! `run_stdio` installs the level set with `Sidecar::log_level`. It exits the
//! process and installs a process-global logger, so the test runs itself again
//! as a child process: the child is a real sidecar on its own stdin and stdout,
//! the parent plays srelens, and the child's stderr says what it logged.

mod common;

use serde_json::{json, Value};
use srelens_sidecar::{Context, Error, Sidecar};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;

const CHILD: &str = "SRELENS_SDK_RUN_STDIO_CHILD";
const TEST: &str = "run_stdio_logs_at_the_level_set_with_log_level_and_at_info_without_one";

/// The child: a sidecar whose operation logs at debug and at info, run on
/// stdio, with the level set only if `mode` is `debug`.
fn run_as_child(mode: &str) -> ! {
    let mut sidecar =
        Sidecar::new("t", "1").operation("probe", |_ctx: Context, _: Value| async move {
            log::debug!("DEBUG-LINE");
            log::info!("INFO-LINE");
            Ok::<_, Error>(json!({}))
        });
    if mode == "debug" {
        sidecar = sidecar.log_level(log::LevelFilter::Debug);
    }
    let _ = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(sidecar.run_stdio());
    unreachable!("run_stdio exits the process");
}

/// Run the child in `mode`, play srelens to it until its operation has been
/// answered, end its input, and return what it wrote to stderr.
fn stderr_of_child(mode: &str) -> String {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The child's stdout holds libtest's own lines as well as the sidecar's:
    // keep the ones that are JSON, read on a thread so a silent child times out.
    let stdout = child.stdout.take().unwrap();
    let (lines, answers) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<Value>(&line) {
                if lines.send(message).is_err() {
                    break;
                }
            }
        }
    });
    let mut stdin = child.stdin.take().unwrap();
    let data_dir = std::env::temp_dir();
    for (id, method, params) in [
        (
            1,
            "initialize",
            common::initialize_params(&["0.1.0"], &data_dir.to_string_lossy()),
        ),
        (2, "activate", json!({})),
        (3, "probe", json!({})),
    ] {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(stdin, "{request}").unwrap();
    }
    // The handler logs before it returns, so its answer means it has logged.
    // Ending the session first would abort it.
    loop {
        match answers.recv_timeout(common::WAIT) {
            Ok(message) if message["id"] == 3 => break,
            Ok(_) => {}
            Err(why) => {
                let _ = child.kill();
                panic!("the sidecar did not answer `probe`: {why}");
            }
        }
    }
    drop(stdin);
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "the child exited {status}: {stderr}");
    stderr
}

#[test]
fn run_stdio_logs_at_the_level_set_with_log_level_and_at_info_without_one() {
    if let Ok(mode) = std::env::var(CHILD) {
        run_as_child(&mode);
    }
    let set = stderr_of_child("debug");
    // `INFO-LINE` in both shows the child ran this test and its handler.
    assert!(set.contains("INFO-LINE"), "{set}");
    assert!(
        set.contains("DEBUG-LINE"),
        "`log_level(Debug)` was not applied: {set}"
    );
    let default = stderr_of_child("default");
    assert!(default.contains("INFO-LINE"), "{default}");
    assert!(
        !default.contains("DEBUG-LINE"),
        "the default is not Info: {default}"
    );
}
