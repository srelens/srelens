mod common;

use common::FakeHost;
use serde::Deserialize;
use serde_json::{json, Value};
use srelens_sidecar::{Context, Error, Frames, Sidecar, StreamClosed};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
struct Count {
    to: u64,
}

async fn count(ctx: Context, input: Count, frames: Frames) -> Result<(), Error> {
    for n in 0..input.to {
        if ctx.is_cancelled() {
            break;
        }
        frames.send(&n).await?;
    }
    Ok(())
}

fn sidecar() -> Sidecar {
    Sidecar::new("t", "1")
        .stream("count", count)
        .stream(
            "fail",
            |_ctx: Context, _: Value, frames: Frames| async move {
                frames.send(&"first").await?;
                Err(Error::internal("the registry did not answer"))
            },
        )
        .stream(
            "forever",
            |ctx: Context, _: Value, frames: Frames| async move {
                let mut n = 0u64;
                while frames.send(&n).await.is_ok() {
                    n += 1;
                    tokio::task::yield_now().await;
                    if ctx.is_cancelled() {
                        break;
                    }
                }
                Ok(())
            },
        )
}

#[tokio::test]
async fn a_stream_is_acknowledged_then_sends_its_frames_then_closes() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 1, "method": "count", "params": {"to": 3}}),
        )
        .await;
    assert_eq!(host.answer(id).await["result"], json!({}));
    for n in 0..3 {
        assert_eq!(
            host.recv().await,
            json!({"jsonrpc": "2.0", "method": "stream/data", "params": {"stream": 1, "data": n}})
        );
    }
    assert_eq!(
        host.recv().await,
        json!({"jsonrpc": "2.0", "method": "stream/close", "params": {"stream": 1}})
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_failing_stream_ends_with_its_error() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 2, "method": "fail", "params": {}}),
        )
        .await;
    host.answer(id).await;
    assert_eq!(host.recv().await["params"]["data"], "first");
    assert_eq!(
        host.recv().await,
        json!({"jsonrpc": "2.0", "method": "stream/error", "params": {"stream": 2, "message": "the registry did not answer"}})
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn an_unknown_stream_or_bad_input_is_refused_before_it_opens() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 3, "method": "nope", "params": {}}),
        )
        .await;
    assert_eq!(host.answer(id).await["error"]["code"], -32601);
    let id = host
        .request(
            "stream/open",
            json!({"stream": 4, "method": "count", "params": {"to": "three"}}),
        )
        .await;
    assert_eq!(host.answer(id).await["error"]["code"], -32602);
    let health = host.request("health", json!({})).await;
    assert_eq!(
        host.answer(health).await["result"],
        json!({}),
        "nothing else was sent"
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_cancelled_stream_stops_and_sends_no_terminal_frame() {
    let mut host = FakeHost::start(sidecar());
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 5, "method": "forever", "params": {}}),
        )
        .await;
    host.answer(id).await;
    host.recv().await;
    host.notify("stream/cancel", json!({"stream": 5})).await;
    // Frames already queued may still arrive before the answer to this.
    let health = host.request("health", json!({})).await;
    loop {
        let line = host.recv().await;
        if line.get("id") == Some(&json!(health)) {
            break;
        }
        assert_eq!(
            line["method"], "stream/data",
            "only frames queued before the cancel: {line}"
        );
    }
    // Then at most the one frame that was mid-send when the cancel landed,
    // and never a close or an error: the stream stopped.
    let mut stragglers = 0;
    while let Some(line) = host
        .next_within(std::time::Duration::from_millis(300))
        .await
    {
        assert_eq!(
            line["method"], "stream/data",
            "no terminal frame after a cancel: {line}"
        );
        stragglers += 1;
        assert!(stragglers <= 1, "the stream kept sending after its cancel");
    }
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_stream_whose_error_is_too_large_still_ends_with_an_error_frame() {
    let sidecar = Sidecar::new("t", "1").stream(
        "too-large-error",
        |_ctx: Context, _: Value, _frames: Frames| async move {
            Err(Error::internal("x".repeat(5 * 1024 * 1024)))
        },
    );
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 7, "method": "too-large-error", "params": {}}),
        )
        .await;
    host.answer(id).await;
    let end = host.recv().await;
    assert_eq!(end["method"], "stream/error");
    assert_eq!(end["params"]["stream"], 7);
    assert!(
        end["params"]["message"].as_str().unwrap().contains("4 MiB"),
        "{end}"
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_frame_that_cannot_be_serialized_says_so() {
    let captured: Arc<Mutex<Option<StreamClosed>>> = Arc::new(Mutex::new(None));
    let captured_in_handler = captured.clone();
    let sidecar = Sidecar::new("t", "1").stream(
        "unserializable",
        move |_ctx: Context, _: Value, frames: Frames| {
            let captured = captured_in_handler.clone();
            async move {
                let mut m: HashMap<(u8, u8), u8> = HashMap::new();
                m.insert((1, 2), 3);
                let error = frames.send(&m).await.unwrap_err();
                *captured.lock().unwrap() = Some(error.clone());
                Err(error.into())
            }
        },
    );
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 8, "method": "unserializable", "params": {}}),
        )
        .await;
    host.answer(id).await;
    let end = host.recv().await;
    assert_eq!(end["method"], "stream/error");
    assert!(
        end["params"]["message"]
            .as_str()
            .unwrap()
            .contains("could not be serialized"),
        "{end}"
    );
    assert!(
        matches!(
            captured.lock().unwrap().take(),
            Some(StreamClosed::Invalid(_))
        ),
        "expected StreamClosed::Invalid"
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_frame_over_the_message_limit_is_refused_to_the_handler() {
    let sidecar = Sidecar::new("t", "1").stream(
        "big",
        |_ctx: Context, _: Value, frames: Frames| async move {
            let refused = frames.send(&"x".repeat(5 * 1024 * 1024)).await.unwrap_err();
            Err(Error::internal(refused.to_string()))
        },
    );
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host
        .request(
            "stream/open",
            json!({"stream": 6, "method": "big", "params": {}}),
        )
        .await;
    host.answer(id).await;
    let end = host.recv().await;
    assert_eq!(end["method"], "stream/error");
    assert!(
        end["params"]["message"].as_str().unwrap().contains("4 MiB"),
        "{end}"
    );
    host.finish().await.unwrap();
}
