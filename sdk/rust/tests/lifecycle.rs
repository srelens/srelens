mod common;

use common::{initialize_params, is_sidecar_message, FakeHost, WAIT};
use serde_json::json;
use srelens_sidecar::{Sidecar, SidecarError};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

fn sidecar() -> Sidecar {
    Sidecar::new("test-sidecar", "1.2.3")
}

/// A writer whose every write fails, as a broken pipe would.
struct FailingWriter;

impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::other("broken pipe")))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn initialize_names_the_version_and_the_sidecar() {
    let mut host = FakeHost::start(sidecar());
    let result = host.initialize().await;
    assert_eq!(
        result,
        json!({"apiVersion": "0.1.0", "sidecar": {"name": "test-sidecar", "version": "1.2.3"}})
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn with_no_version_in_common_initialize_says_which_it_speaks() {
    let mut host = FakeHost::start(sidecar());
    let id = host
        .request("initialize", initialize_params(&["9.0.0"], "/d"))
        .await;
    let answer = host.answer(id).await;
    assert_eq!(answer["error"]["code"], -32001);
    assert_eq!(answer["error"]["data"], json!({"supported": ["0.1.0"]}));
    host.finish().await.unwrap();
}

#[tokio::test]
async fn nothing_is_served_before_initialize() {
    let mut host = FakeHost::start(sidecar());
    let id = host.request("health", json!({})).await;
    assert_eq!(host.answer(id).await["error"]["code"], -32600);
    host.finish().await.unwrap();
}

#[tokio::test]
async fn health_and_deactivate_are_answered_with_nothing() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    for method in ["health", "deactivate"] {
        let id = host.request(method, json!({})).await;
        assert_eq!(host.answer(id).await["result"], json!({}), "{method}");
    }
    host.finish().await.unwrap();
}

#[tokio::test]
async fn shutdown_is_answered_and_then_the_session_ends() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    let id = host.request("shutdown", json!({})).await;
    assert_eq!(host.answer(id).await["result"], json!({}));
    host.ended().await.unwrap();
}

#[tokio::test]
async fn the_session_ends_when_its_input_does() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_line_from_srelens_that_is_not_json_rpc_ends_the_session_with_an_error() {
    let mut host = FakeHost::start(sidecar());
    host.send_raw("this is not json").await;
    assert!(matches!(host.ended().await, Err(SidecarError::Protocol(_))));
}

#[tokio::test]
async fn a_sidecar_whose_output_fails_ends_with_an_io_error() {
    let line = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": initialize_params(&["0.1.0"], "/d")})
    );
    let reader = io::Cursor::new(line.into_bytes());
    let result = tokio::time::timeout(WAIT, sidecar().run(reader, FailingWriter))
        .await
        .expect("the session ended in time");
    assert!(matches!(result, Err(SidecarError::Io(_))), "{result:?}");
}

/// Stands in for tokio's stdin: `lines` first, then a read that waits on a
/// blocking thread (`spawn_blocking`, on the runtime reading it), which
/// nothing can cancel, until the test drops the sender `HeldInput::new`
/// returns. A runtime that waits for its blocking tasks waits for the test.
/// With `then_panic`, the next read panics instead, as a bug in the session
/// would.
struct HeldInput {
    lines: io::Cursor<Vec<u8>>,
    release: Option<std::sync::mpsc::Receiver<()>>,
    /// Set once the held read has started.
    held: Arc<AtomicBool>,
    then_panic: bool,
}

impl HeldInput {
    fn new(lines: String, then_panic: bool) -> (HeldInput, std::sync::mpsc::Sender<()>) {
        let (release_tx, release) = std::sync::mpsc::channel();
        let input = HeldInput {
            lines: io::Cursor::new(lines.into_bytes()),
            release: Some(release),
            held: Arc::default(),
            then_panic,
        };
        (input, release_tx)
    }
}

impl AsyncRead for HeldInput {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.lines.position() < this.lines.get_ref().len() as u64 {
            return Pin::new(&mut this.lines).poll_read(cx, buf);
        }
        let Some(release) = this.release.take() else {
            if this.then_panic {
                panic!("the session broke while a read was held");
            }
            return Poll::Pending;
        };
        tokio::task::spawn_blocking(move || {
            let _ = release.recv();
        });
        this.held.store(true, Ordering::SeqCst);
        if this.then_panic {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }
}

#[tokio::test]
async fn a_sidecar_whose_output_fails_ends_even_while_a_read_of_its_input_is_held() {
    let line = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": initialize_params(&["0.1.0"], "/d")})
    );
    let (input, release) = HeldInput::new(line, false);
    let held = input.held.clone();
    let result = tokio::time::timeout(WAIT, sidecar().run(input, FailingWriter))
        .await
        .expect("the session ended in time, without waiting for the held read");
    assert!(matches!(result, Err(SidecarError::Io(_))), "{result:?}");
    assert!(
        held.load(Ordering::SeqCst),
        "the session ended while a read was held"
    );
    drop(release);
}

#[tokio::test]
async fn a_session_that_panics_ends_with_an_io_error_even_while_a_read_is_held() {
    let (input, release) = HeldInput::new(String::new(), true);
    let run = tokio::spawn(sidecar().run(input, tokio::io::sink()));
    let result = tokio::time::timeout(WAIT, run)
        .await
        .expect("the session ended in time, without waiting for the held read")
        .expect("`run` returned instead of panicking");
    assert!(
        matches!(&result, Err(SidecarError::Io(e)) if e.to_string() == "the session thread stopped"),
        "{result:?}"
    );
    drop(release);
}

#[tokio::test]
async fn dropping_the_run_future_ends_the_session() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    host.abandon().await;
}

#[test]
fn the_schema_check_refuses_what_a_sidecar_may_not_write() {
    // Guards the guard: the validator every test reads through is live.
    assert!(is_sidecar_message(
        &json!({"jsonrpc": "2.0", "id": 1, "result": {}})
    ));
    assert!(!is_sidecar_message(
        &json!({"jsonrpc": "2.0", "id": "1", "result": {}})
    ));
}
