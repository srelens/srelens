mod common;

use common::{initialize_params, is_sidecar_message, FakeHost, WAIT};
use serde_json::json;
use srelens_sidecar::{Sidecar, SidecarError};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

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
