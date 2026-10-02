mod common;

use common::FakeHost;
use serde_json::{json, Value};
use srelens_sidecar::{CallContext, Context, Error, Host, HostError, Sidecar};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn prod() -> CallContext {
    CallContext::new("kind-dev", Some("team")).unwrap()
}

#[tokio::test]
async fn each_host_call_names_its_context_and_gets_the_hosts_answer() {
    let sidecar = Sidecar::new("t", "1")
        .operation("read", |ctx: Context, _: Value| async move {
            Ok::<_, Error>(ctx.host().read(&prod(), "apps").await?)
        })
        .operation("resource", |ctx: Context, _: Value| async move {
            Ok::<_, Error>(ctx.host().resource(&prod(), "apps", "web").await?)
        })
        .operation("action", |ctx: Context, _: Value| async move {
            Ok::<_, Error>(
                ctx.host()
                    .action(&prod(), "apps", "web", "sync", "u-1", "42")
                    .await?,
            )
        });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let context = json!({"clusterId": "kind-dev", "namespace": "team"});
    for (op, method, params) in [
        (
            "read",
            "host/read",
            json!({"context": context, "capability": "apps"}),
        ),
        (
            "resource",
            "host/resource",
            json!({"context": context, "capability": "apps", "name": "web"}),
        ),
        (
            "action",
            "host/action",
            json!({"context": context, "capability": "apps", "name": "web",
                                         "action": "sync", "uid": "u-1", "resourceVersion": "42"}),
        ),
    ] {
        let id = host.request(op, json!({})).await;
        let call = host.call().await;
        assert_eq!(call["method"], method);
        assert_eq!(call["params"], params);
        assert!(
            call["id"].as_str().is_some_and(|id| id.starts_with("c-")),
            "{call}"
        );
        host.reply(&call["id"], Ok(json!({"answered": method})))
            .await;
        assert_eq!(host.answer(id).await["result"], json!({"answered": method}));
    }
    host.finish().await.unwrap();
}

#[tokio::test]
async fn the_hosts_refusals_become_host_errors_by_code() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let sidecar = Sidecar::new("t", "1").operation("read", move |ctx: Context, _: Value| {
        let log = log.clone();
        async move {
            let outcome = ctx.host().read(&prod(), "apps").await;
            log.lock().unwrap().push(outcome);
            Ok::<_, Error>(())
        }
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    for (code, message) in [
        (-32002, "declined"),
        (-32003, "no grant"),
        (-32602, "bad"),
        (-32800, "cancelled"),
        (-32601, "no such"),
    ] {
        let id = host.request("read", json!({})).await;
        let call = host.call().await;
        host.reply(&call["id"], Err(json!({"code": code, "message": message})))
            .await;
        host.answer(id).await;
    }
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0], Err(HostError::ConsentDenied("declined".into())));
    assert_eq!(seen[1], Err(HostError::CapabilityFailed("no grant".into())));
    assert_eq!(seen[2], Err(HostError::InvalidParams("bad".into())));
    assert_eq!(seen[3], Err(HostError::Cancelled));
    assert!(matches!(&seen[4], Err(HostError::Rpc(e)) if e.code == -32601));
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_call_the_sidecar_stops_waiting_for_is_cancelled_at_the_host() {
    let sidecar = Sidecar::new("t", "1").operation("read", |ctx: Context, _: Value| async move {
        let context = prod();
        tokio::select! {
            _ = ctx.host().read(&context, "apps") => {}
            _ = ctx.cancelled() => {}
        }
        Ok::<_, Error>(())
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("read", json!({})).await;
    let call = host.call().await;
    host.notify("$/cancelRequest", json!({"id": id})).await;
    // srelens's cancel answer and the sidecar's cancel of its own call, in either order.
    let mut lines = [host.recv().await, host.recv().await];
    lines.sort_by_key(|m| m.get("method").is_some());
    assert_eq!(lines[0]["error"]["code"], -32800);
    assert_eq!(
        lines[1],
        json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": call["id"]}})
    );
    host.finish().await.unwrap();
}

#[tokio::test]
async fn at_most_eight_calls_are_in_flight_and_the_ninth_waits() {
    let sidecar = Sidecar::new("t", "1").operation("read", |ctx: Context, _: Value| async move {
        Ok::<_, Error>(ctx.host().read(&prod(), "apps").await?)
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    for _ in 0..9 {
        host.request("read", json!({})).await;
    }
    let mut calls = Vec::new();
    for _ in 0..8 {
        calls.push(host.call().await);
    }
    // The ninth is not sent until one is answered.
    let id = host.request("health", json!({})).await;
    assert_eq!(host.answer(id).await["result"], json!({}));
    host.reply(&calls[0]["id"], Ok(json!(1))).await;
    let mut ninth_seen = false;
    for _ in 0..2 {
        let line = host.recv().await;
        if line.get("method").is_some() {
            ninth_seen = true;
        }
    }
    assert!(ninth_seen, "the ninth call was sent once a slot freed");
    host.finish().await.unwrap();
}

#[tokio::test]
async fn calls_waiting_when_the_session_ends_are_disconnected() {
    // The handler hands its `Host` clone, and the outcome of a call made on
    // a detached task, out to the test: that task is not tracked by the
    // session and so is never aborted, unlike the handler's own task -- it
    // must see `Disconnected` from `Host::disconnect` itself, or this test
    // would hang instead of failing.
    let (host_tx, host_rx) = tokio::sync::oneshot::channel::<Host>();
    let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<Result<Value, HostError>>();
    let host_tx = Arc::new(Mutex::new(Some(host_tx)));
    let outcome_tx = Arc::new(Mutex::new(Some(outcome_tx)));
    let sidecar = Sidecar::new("t", "1").operation("read", move |ctx: Context, _: Value| {
        let host_tx = host_tx.clone();
        let outcome_tx = outcome_tx.clone();
        async move {
            let host = ctx.host().clone();
            if let Some(tx) = host_tx.lock().unwrap().take() {
                let _ = tx.send(host.clone());
            }
            tokio::spawn(async move {
                let result = host.read(&prod(), "apps").await;
                if let Some(tx) = outcome_tx.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
            });
            Ok::<_, Error>(())
        }
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("read", json!({})).await;
    // The op's own answer and the detached task's host/read call, in either order.
    let mut lines = [host.recv().await, host.recv().await];
    lines.sort_by_key(|m| m.get("method").is_some());
    assert_eq!(lines[0]["id"], json!(id));
    assert_eq!(lines[1]["method"], "host/read");

    let host2 = host_rx.await.expect("the handler shared its Host clone");

    host.finish().await.unwrap();

    let outcome = tokio::time::timeout(Duration::from_millis(300), outcome_rx)
        .await
        .expect("the detached call resolved once the session ended")
        .expect("the outcome was sent");
    assert_eq!(outcome, Err(HostError::Disconnected));

    // A later call on the same (disconnected) Host fails at once, not by hanging.
    let fresh = tokio::time::timeout(Duration::from_millis(100), host2.read(&prod(), "apps"))
        .await
        .expect("a call on a disconnected host returns at once");
    assert_eq!(fresh, Err(HostError::Disconnected));
}

#[tokio::test]
async fn a_cancelled_call_keeps_its_slot_until_srelens_answers() {
    let sidecar = Sidecar::new("t", "1").operation("read", |ctx: Context, _: Value| async move {
        let context = prod();
        tokio::select! {
            _ = ctx.host().read(&context, "apps") => {}
            _ = ctx.cancelled() => {}
        }
        Ok::<_, Error>(())
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;

    let mut op_ids = Vec::new();
    for _ in 0..8 {
        op_ids.push(host.request("read", json!({})).await);
    }
    let mut calls = Vec::new();
    for _ in 0..8 {
        calls.push(host.call().await);
    }
    for &id in &op_ids {
        host.notify("$/cancelRequest", json!({"id": id})).await;
    }
    // The 8 op-level cancel answers and the 8 sidecar cancels of their own
    // host calls, in any order.
    let mut lines = Vec::new();
    for _ in 0..16 {
        lines.push(host.recv().await);
    }
    let cancel_answers: Vec<_> = lines.iter().filter(|m| m.get("method").is_none()).collect();
    let cancel_notes: Vec<_> = lines.iter().filter(|m| m.get("method").is_some()).collect();
    assert_eq!(cancel_answers.len(), 8, "{lines:?}");
    assert_eq!(cancel_notes.len(), 8, "{lines:?}");
    for answer in &cancel_answers {
        assert_eq!(answer["error"]["code"], -32800);
    }
    let mut cancelled_ids: Vec<String> = cancel_notes
        .iter()
        .map(|m| m["params"]["id"].as_str().unwrap().to_owned())
        .collect();
    cancelled_ids.sort();
    let mut call_ids: Vec<String> = calls
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect();
    call_ids.sort();
    assert_eq!(cancelled_ids, call_ids);

    // Every slot is still held: the ninth call does not reach srelens.
    let id9 = host.request("read", json!({})).await;
    assert!(
        host.next_within(Duration::from_millis(300)).await.is_none(),
        "the ninth call must wait for a freed slot"
    );

    // Answering one of the cancelled calls frees its slot.
    host.reply(
        &calls[0]["id"],
        Err(json!({"code": -32800, "message": "cancelled"})),
    )
    .await;
    let ninth_call = host.call().await;
    assert_eq!(ninth_call["method"], "host/read");

    // Clean up: nothing is left unread.
    host.reply(&ninth_call["id"], Ok(json!(1))).await;
    host.answer(id9).await;
    for call in &calls[1..] {
        host.reply(
            &call["id"],
            Err(json!({"code": -32800, "message": "cancelled"})),
        )
        .await;
    }
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_call_over_the_message_limit_is_refused_as_too_large_and_the_session_goes_on() {
    let seen = Arc::new(Mutex::new(None));
    let log = seen.clone();
    let sidecar = Sidecar::new("t", "1").operation("big", move |ctx: Context, _: Value| {
        let log = log.clone();
        async move {
            let name = "x".repeat(5 * 1024 * 1024);
            let outcome = ctx.host().resource(&prod(), "apps", &name).await;
            *log.lock().unwrap() = Some(outcome);
            Ok::<_, Error>(())
        }
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("big", json!({})).await;
    // The operation's own answer is the next line: no call reached the wire.
    assert_eq!(host.answer(id).await["result"], Value::Null);
    let error = seen
        .lock()
        .unwrap()
        .take()
        .expect("the handler ran")
        .expect_err("a call over the limit cannot be sent");
    assert!(
        matches!(error, HostError::TooLarge(bytes) if bytes > 5 * 1024 * 1024),
        "the session with srelens is still live; the call alone was too large: {error:?}"
    );
    assert!(error.to_string().contains("4 MiB"), "{error}");
    let health = host.request("health", json!({})).await;
    assert_eq!(host.answer(health).await["result"], json!({}));
    host.finish().await.unwrap();
}
