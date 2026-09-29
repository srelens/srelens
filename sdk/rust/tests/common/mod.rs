//! A fake srelens: drives `Sidecar::run` over in-memory pipes, as the
//! supervisor would, and holds every line the sidecar writes to the committed
//! schema's `SidecarMessage` -- including anything written after the test's
//! last explicit read, which `finish` and `ended` drain and check before
//! they return.

#![allow(dead_code)]

use serde_json::{json, Value};
use srelens_sidecar::{Sidecar, SidecarError};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::DuplexStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};
use tokio::task::JoinHandle;

/// How long a test waits for the sidecar to write. Far above anything the
/// SDK should need; a wait this long is a failure.
pub const WAIT: Duration = Duration::from_secs(5);

fn sidecar_message() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let root = srelens_sidecar_protocol::schema();
        jsonschema::draft7::new(&json!({
            "definitions": root["definitions"],
            "allOf": [{"$ref": "#/definitions/SidecarMessage"}],
        }))
        .expect("the protocol schema compiles")
    })
}

/// Whether `message` is a line a sidecar may write, per the committed schema.
pub fn is_sidecar_message(message: &Value) -> bool {
    sidecar_message().is_valid(message)
}

pub struct FakeHost {
    to_sidecar: WriteHalf<DuplexStream>,
    from_sidecar: Lines<BufReader<ReadHalf<DuplexStream>>>,
    session: JoinHandle<Result<(), SidecarError>>,
    next_id: u64,
    pub data_dir: std::path::PathBuf,
}

impl FakeHost {
    pub fn start(sidecar: Sidecar) -> FakeHost {
        let (host_end, sidecar_end) = tokio::io::duplex(1 << 20);
        let (sidecar_in, sidecar_out) = tokio::io::split(sidecar_end);
        let session = tokio::spawn(sidecar.run(sidecar_in, sidecar_out));
        let (from, to) = tokio::io::split(host_end);
        FakeHost {
            to_sidecar: to,
            from_sidecar: BufReader::new(from).lines(),
            session,
            next_id: 0,
            data_dir: std::env::temp_dir(),
        }
    }

    pub async fn send_raw(&mut self, line: &str) {
        self.to_sidecar
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    /// Send a request; its id.
    pub async fn request(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send_raw(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        )
        .await;
        id
    }

    pub async fn notify(&mut self, method: &str, params: Value) {
        self.send_raw(&json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string())
            .await;
    }

    /// The next line the sidecar wrote, which must be a valid `SidecarMessage`.
    pub async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(WAIT, self.from_sidecar.next_line())
            .await
            .expect("the sidecar wrote nothing in time")
            .expect("its stdout reads")
            .expect("its stdout is still open");
        let message: Value = serde_json::from_str(&line).expect("one JSON object per line");
        let errors: Vec<String> = sidecar_message()
            .iter_errors(&message)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "the sidecar wrote {message}, which the schema refuses: {errors:?}"
        );
        message
    }

    /// The next line if the sidecar writes one within `wait`, validated as
    /// [`FakeHost::recv`] does; `None` if it writes nothing.
    pub async fn next_within(&mut self, wait: Duration) -> Option<Value> {
        let line = tokio::time::timeout(wait, self.from_sidecar.next_line())
            .await
            .ok()?;
        let line = line.expect("its stdout reads")?;
        let message: Value = serde_json::from_str(&line).expect("one JSON object per line");
        assert!(is_sidecar_message(&message), "the schema refuses {message}");
        Some(message)
    }

    /// The next line, which must be the answer to `id`.
    pub async fn answer(&mut self, id: u64) -> Value {
        let message = self.recv().await;
        assert_eq!(
            message["id"],
            json!(id),
            "expected the answer to {id}, got {message}"
        );
        message
    }

    /// Initialize and activate, as the supervisor does; `initialize`'s result.
    pub async fn initialize(&mut self) -> Value {
        let id = self
            .request(
                "initialize",
                initialize_params(&["0.1.0"], &self.data_dir.to_string_lossy()),
            )
            .await;
        let initialized = self.answer(id).await;
        let id = self.request("activate", json!({})).await;
        assert_eq!(self.answer(id).await["result"], json!({}));
        initialized["result"].clone()
    }

    /// Close the sidecar's stdin and wait for the session to end. Then check
    /// that nothing was left on its stdout unread (see [`drain`]).
    pub async fn finish(mut self) -> Result<(), SidecarError> {
        self.to_sidecar.shutdown().await.unwrap();
        drop(self.to_sidecar);
        let ended = tokio::time::timeout(WAIT, self.session)
            .await
            .expect("the session ended in time")
            .expect("the session did not panic");
        drain(&mut self.from_sidecar).await;
        ended
    }

    /// Wait for the session to end without closing stdin (after `shutdown`).
    /// Then check that nothing was left on its stdout unread (see [`drain`]).
    pub async fn ended(mut self) -> Result<(), SidecarError> {
        let ended = tokio::time::timeout(WAIT, self.session)
            .await
            .expect("the session ended in time")
            .expect("the session did not panic");
        drain(&mut self.from_sidecar).await;
        ended
    }
}

/// Read `from_sidecar` to EOF (bounded by [`WAIT`]), validating every
/// remaining line exactly as [`FakeHost::recv`] does. Called once the
/// session has ended, when EOF should follow quickly: the writer task
/// returns and shuts its write half down. Any line still here is one no
/// test assertion consumed -- invalid JSON, a schema-refused line, a
/// duplicate answer, anything -- and that is itself the failure this
/// harness exists to catch.
async fn drain(from_sidecar: &mut Lines<BufReader<ReadHalf<DuplexStream>>>) {
    loop {
        let line = tokio::time::timeout(WAIT, from_sidecar.next_line())
            .await
            .expect("the sidecar's stdout did not close in time")
            .expect("its stdout reads");
        let Some(line) = line else {
            return; // EOF: nothing left unread.
        };
        let message: Value = serde_json::from_str(&line).expect("one JSON object per line");
        let errors: Vec<String> = sidecar_message()
            .iter_errors(&message)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "the sidecar wrote {message}, which the schema refuses: {errors:?}"
        );
        panic!("the sidecar wrote {message}, which the test never read");
    }
}

pub fn initialize_params(offered: &[&str], data_dir: &str) -> Value {
    json!({
        "apiVersions": offered,
        "host": {"name": "srelens", "version": "0.15.0"},
        "limits": {"requestTimeoutMs": 30000, "maxConcurrentRequests": 8, "maxStreams": 5,
                   "memoryBytes": 268435456u64, "cpus": 1.0, "dataBytes": 1073741824u64, "dataEntries": 100000},
        "dataDirectory": data_dir,
    })
}
