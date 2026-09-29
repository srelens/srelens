mod common;

use common::FakeHost;
use serde_json::{json, Value};
use srelens_sidecar::{CallContext, Context, Error, HostError, Sidecar};
use std::sync::{Arc, Mutex};

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
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], Err(HostError::ConsentDenied("declined".into())));
    assert_eq!(seen[1], Err(HostError::CapabilityFailed("no grant".into())));
    assert_eq!(seen[2], Err(HostError::InvalidParams("bad".into())));
    assert_eq!(seen[3], Err(HostError::Cancelled));
    assert!(matches!(&seen[4], Err(HostError::Rpc(e)) if e.code == -32601));
    drop(seen);
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
    let mut lines = vec![host.recv().await, host.recv().await];
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
    let seen = Arc::new(Mutex::new(None));
    let log = seen.clone();
    let sidecar = Sidecar::new("t", "1").operation("read", move |ctx: Context, _: Value| {
        let log = log.clone();
        async move {
            *log.lock().unwrap() = Some(ctx.host().read(&prod(), "apps").await);
            Ok::<_, Error>(())
        }
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    host.request("read", json!({})).await;
    host.call().await;
    host.finish().await.unwrap();
    // The handler was aborted with the session, or saw Disconnected: never a hang.
    assert!(matches!(
        *seen.lock().unwrap(),
        None | Some(Err(HostError::Disconnected))
    ));
}
