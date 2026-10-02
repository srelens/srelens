mod common;

use common::FakeHost;
use serde::{Deserialize, Serialize};
use serde_json::json;
use srelens_sidecar::{Context, Error, Sidecar};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Deserialize)]
struct Name {
    name: String,
}

#[derive(Serialize)]
struct Greeting {
    greeting: String,
}

async fn greet(_ctx: Context, input: Name) -> Result<Greeting, Error> {
    Ok(Greeting {
        greeting: format!("Hello, {}", input.name),
    })
}

#[tokio::test]
async fn an_operation_gets_its_typed_input_and_answers_its_output() {
    let mut host = FakeHost::start(Sidecar::new("t", "1").operation("greet", greet));
    host.initialize().await;
    let id = host.request("greet", json!({"name": "srelens"})).await;
    assert_eq!(
        host.answer(id).await["result"],
        json!({"greeting": "Hello, srelens"})
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn input_that_does_not_fit_is_invalid_params_and_an_unknown_method_is_not_found() {
    let mut host = FakeHost::start(Sidecar::new("t", "1").operation("greet", greet));
    host.initialize().await;
    let id = host.request("greet", json!({"nom": "x"})).await;
    let answer = host.answer(id).await;
    assert_eq!(answer["error"]["code"], -32602);
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("name"),
        "{answer}"
    );
    let id = host.request("wave", json!({})).await;
    let answer = host.answer(id).await;
    assert_eq!(answer["error"]["code"], -32601);
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("wave"),
        "{answer}"
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_handlers_error_is_answered_as_it_said() {
    let sidecar = Sidecar::new("t", "1").operation(
        "fail",
        |_ctx: Context, _input: serde_json::Value| async {
            Err::<(), _>(Error::new(-32050, "no database yet").with_data(json!({"retry": true})))
        },
    );
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("fail", json!({})).await;
    assert_eq!(
        host.answer(id).await["error"],
        json!({"code": -32050, "message": "no database yet", "data": {"retry": true}})
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn health_is_answered_while_every_handler_is_busy() {
    let sidecar = Sidecar::new("t", "1").operation(
        "block",
        |ctx: Context, _: serde_json::Value| async move {
            ctx.cancelled().await;
            Ok::<_, Error>(())
        },
    );
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    for _ in 0..8 {
        host.request("block", json!({})).await;
    }
    let id = host.request("health", json!({})).await;
    assert_eq!(host.answer(id).await["result"], json!({}));
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_cancelled_request_is_answered_request_cancelled_once_and_its_handler_is_told() {
    let told = Arc::new(AtomicBool::new(false));
    let seen = told.clone();
    let sidecar =
        Sidecar::new("t", "1").operation("slow", move |ctx: Context, _: serde_json::Value| {
            let seen = seen.clone();
            async move {
                ctx.cancelled().await;
                seen.store(true, Ordering::SeqCst);
                Ok::<_, Error>("late")
            }
        });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("slow", json!({})).await;
    host.notify("$/cancelRequest", json!({"id": id})).await;
    assert_eq!(host.answer(id).await["error"]["code"], -32800);
    // The handler's own late answer is dropped: the next line answers this.
    let health = host.request("health", json!({})).await;
    assert_eq!(host.answer(health).await["result"], json!({}));
    // The handler runs on another thread: give it a moment to see the cancel.
    for _ in 0..100 {
        if told.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        told.load(Ordering::SeqCst),
        "the handler saw its cancellation"
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_handler_that_panics_is_answered_internal_error_and_the_sidecar_goes_on() {
    let sidecar = Sidecar::new("t", "1")
        .operation("boom", |_ctx: Context, _: serde_json::Value| async {
            if true {
                panic!("kaboom");
            }
            Ok::<(), Error>(())
        })
        .operation("greet", greet);
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("boom", json!({})).await;
    let answer = host.answer(id).await;
    assert_eq!(answer["error"]["code"], -32603);
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("boom"),
        "{answer}"
    );
    let id = host.request("greet", json!({"name": "x"})).await;
    assert_eq!(host.answer(id).await["result"]["greeting"], "Hello, x");
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_result_over_the_message_limit_is_answered_with_why_instead() {
    let sidecar = Sidecar::new("t", "1")
        .operation("huge", |_ctx: Context, _: serde_json::Value| async {
            Ok::<_, Error>("x".repeat(5 * 1024 * 1024))
        });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("huge", json!({})).await;
    let answer = host.answer(id).await;
    assert_eq!(answer["error"]["code"], -32603);
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("4 MiB"),
        "{answer}"
    );
    host.finish().await.unwrap();
}

#[test]
#[should_panic(expected = "cannot name an operation")]
fn a_reserved_name_cannot_be_registered() {
    let _ = Sidecar::new("t", "1").operation("shutdown", greet);
}

#[test]
#[should_panic(expected = "registered twice")]
fn a_name_cannot_be_registered_twice() {
    let _ = Sidecar::new("t", "1")
        .operation("greet", greet)
        .operation("greet", greet);
}
