mod common;

use common::FakeHost;
use serde_json::{json, Value};
use srelens_sidecar::{CallContext, Context, Error, Host, HostError, Sidecar};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn prod() -> CallContext {
    CallContext::new("kind-dev", Some("team")).unwrap()
}

// The two contexts below are struct literals: the fields are `pub`, so
// `CallContext::new`, which would refuse both, is bypassed.

fn blank_cluster() -> CallContext {
    CallContext {
        cluster_id: "  ".to_owned(),
        namespace: None,
    }
}

fn bad_namespace() -> CallContext {
    CallContext {
        cluster_id: "kind-dev".to_owned(),
        namespace: Some("Team".to_owned()),
    }
}

/// Run `calls` inside an operation. Each call has a field srelens would
/// refuse, so the SDK must refuse it itself: the operation's own answer is
/// the next line the host reads (no `host/*` call reached the wire, and
/// `finish` fails on any line left unread), and each error is `InvalidCall`
/// naming the field in `fields`, in order.
async fn refused_before_sending<F, Fut>(fields: &[&str], calls: F)
where
    F: Fn(Host) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Vec<Result<Value, HostError>>> + Send + 'static,
{
    let seen = Arc::new(Mutex::new(None));
    let log = seen.clone();
    let calls = Arc::new(calls);
    let sidecar = Sidecar::new("t", "1").operation("bad", move |ctx: Context, _: Value| {
        let (log, calls) = (log.clone(), calls.clone());
        async move {
            let outcomes = calls(ctx.host().clone()).await;
            *log.lock().unwrap() = Some(outcomes);
            Ok::<_, Error>(())
        }
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("bad", json!({})).await;
    assert_eq!(host.answer(id).await["result"], Value::Null);
    let outcomes = seen.lock().unwrap().take().expect("the handler ran");
    assert_eq!(outcomes.len(), fields.len());
    for (outcome, field) in outcomes.into_iter().zip(fields) {
        let error = outcome.expect_err("srelens would refuse this call, so the SDK does");
        let HostError::InvalidCall(why) = &error else {
            panic!("expected InvalidCall for `{field}`, got {error:?}");
        };
        assert!(why.contains(&format!("`{field}`")), "{field}: {why}");
        assert_eq!(error.to_string(), format!("the call was not sent: {why}"));
        // A handler that passes it on with `?` fails with `-32603`: the
        // sidecar's own bug, not the app's request.
        assert_eq!(Error::from(error).code(), -32603);
    }
    // The session with srelens goes on.
    let health = host.request("health", json!({})).await;
    assert_eq!(host.answer(health).await["result"], json!({}));
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_blank_cluster_id_is_refused_before_it_is_sent() {
    refused_before_sending(&["clusterId"], |host| async move {
        vec![host.read(&blank_cluster(), "apps").await]
    })
    .await;
}

#[tokio::test]
async fn a_namespace_that_is_not_a_name_is_refused_before_it_is_sent() {
    refused_before_sending(&["namespace"], |host| async move {
        vec![host.read(&bad_namespace(), "apps").await]
    })
    .await;
}

#[tokio::test]
async fn a_capability_that_is_not_an_identifier_is_refused_before_it_is_sent() {
    refused_before_sending(&["capability"], |host| async move {
        vec![host.read(&prod(), "bad capability").await]
    })
    .await;
}

#[tokio::test]
async fn an_object_name_that_is_not_one_is_refused_before_it_is_sent() {
    refused_before_sending(&["name"], |host| async move {
        vec![host.resource(&prod(), "apps", "web/1").await]
    })
    .await;
}

#[tokio::test]
async fn an_action_that_is_not_an_identifier_is_refused_before_it_is_sent() {
    refused_before_sending(&["action"], |host| async move {
        vec![
            host.action(&prod(), "apps", "web", "sync now", "u-1", "42")
                .await,
        ]
    })
    .await;
}

#[tokio::test]
async fn a_uid_that_is_not_a_token_is_refused_before_it_is_sent() {
    refused_before_sending(&["uid"], |host| async move {
        vec![
            host.action(&prod(), "apps", "web", "sync", "u 1", "42")
                .await,
        ]
    })
    .await;
}

#[tokio::test]
async fn a_resource_version_that_is_not_a_token_is_refused_before_it_is_sent() {
    refused_before_sending(&["resourceVersion"], |host| async move {
        vec![host.action(&prod(), "apps", "web", "sync", "u-1", "").await]
    })
    .await;
}

#[tokio::test]
async fn resource_and_action_check_the_fields_read_does_not_take_too() {
    refused_before_sending(
        &[
            "clusterId",
            "namespace",
            "capability",
            "name",
            "clusterId",
            "namespace",
            "capability",
            "name",
        ],
        |host| async move {
            vec![
                host.resource(&blank_cluster(), "apps", "web").await,
                host.resource(&bad_namespace(), "apps", "web").await,
                host.resource(&prod(), "bad capability", "web").await,
                host.resource(&prod(), "apps", "..").await,
                host.action(&blank_cluster(), "apps", "web", "sync", "u-1", "42")
                    .await,
                host.action(&bad_namespace(), "apps", "web", "sync", "u-1", "42")
                    .await,
                host.action(&prod(), "bad capability", "web", "sync", "u-1", "42")
                    .await,
                host.action(&prod(), "apps", "", "sync", "u-1", "42").await,
            ]
        },
    )
    .await;
}

#[tokio::test]
async fn a_bad_call_is_refused_at_once_even_when_every_slot_is_taken() {
    let seen = Arc::new(Mutex::new(None));
    let log = seen.clone();
    let sidecar = Sidecar::new("t", "1")
        .operation("read", |ctx: Context, _: Value| async move {
            Ok::<_, Error>(ctx.host().read(&prod(), "apps").await?)
        })
        .operation("bad", move |ctx: Context, _: Value| {
            let log = log.clone();
            async move {
                *log.lock().unwrap() = Some(ctx.host().read(&prod(), "bad capability").await);
                Ok::<_, Error>(())
            }
        });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let mut reads = Vec::new();
    for _ in 0..8 {
        reads.push(host.request("read", json!({})).await);
    }
    let mut calls = Vec::new();
    for _ in 0..8 {
        calls.push(host.call().await);
    }
    // All eight slots are taken. A call srelens would refuse needs none: it
    // is refused here, without waiting for one.
    let bad = host.request("bad", json!({})).await;
    assert_eq!(host.answer(bad).await["result"], Value::Null);
    let outcome = seen.lock().unwrap().take().expect("the handler ran");
    assert!(
        matches!(outcome, Err(HostError::InvalidCall(_))),
        "{outcome:?}"
    );
    for call in &calls {
        host.reply(&call["id"], Ok(json!(1))).await;
    }
    for _ in &reads {
        host.recv().await;
    }
    host.finish().await.unwrap();
}

/// Run `call` in an operation and check that the host is sent `method` with
/// exactly `params`. The values below pass the shape of the field they are
/// given for and fail the others: a field checked against the wrong shape
/// would refuse what srelens accepts, and nothing would reach the wire.
async fn is_sent<F, Fut>(method: &str, params: Value, call: F)
where
    F: Fn(Host) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, HostError>> + Send + 'static,
{
    let call = Arc::new(call);
    let sidecar = Sidecar::new("t", "1").operation("go", move |ctx: Context, _: Value| {
        let call = call.clone();
        async move { Ok::<_, Error>(call(ctx.host().clone()).await?) }
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize().await;
    let id = host.request("go", json!({})).await;
    let sent = host.call().await;
    assert_eq!(sent["method"], method);
    assert_eq!(sent["params"], params);
    host.reply(&sent["id"], Ok(json!({"answered": method})))
        .await;
    assert_eq!(host.answer(id).await["result"], json!({"answered": method}));
    host.finish().await.unwrap();
}

fn context_json() -> Value {
    json!({"clusterId": "kind-dev", "namespace": "team"})
}

#[tokio::test]
async fn a_dotted_object_name_is_sent() {
    // A dot is an object name's, not an identifier's.
    is_sent(
        "host/resource",
        json!({"context": context_json(), "capability": "apps", "name": "web.v1-2"}),
        |host| async move { host.resource(&prod(), "apps", "web.v1-2").await },
    )
    .await;
}

#[tokio::test]
async fn an_object_name_of_253_characters_is_sent() {
    // An identifier stops at 64; an object name at 253.
    let name = "a".repeat(253);
    let expected = name.clone();
    is_sent(
        "host/resource",
        json!({"context": context_json(), "capability": "apps", "name": expected}),
        move |host| {
            let name = name.clone();
            async move { host.resource(&prod(), "apps", &name).await }
        },
    )
    .await;
}

#[tokio::test]
async fn an_action_on_a_dotted_object_name_is_sent() {
    is_sent(
        "host/action",
        json!({"context": context_json(), "capability": "apps", "name": "web.v1-2",
               "action": "sync", "uid": "u-1", "resourceVersion": "42"}),
        |host| async move {
            host.action(&prod(), "apps", "web.v1-2", "sync", "u-1", "42")
                .await
        },
    )
    .await;
}

#[tokio::test]
async fn a_uid_with_visible_characters_an_identifier_refuses_is_sent() {
    // A token is any printable ASCII, `:` and `/` included.
    is_sent(
        "host/action",
        json!({"context": context_json(), "capability": "apps", "name": "web",
               "action": "sync", "uid": "a:b/c", "resourceVersion": "42"}),
        |host| async move {
            host.action(&prod(), "apps", "web", "sync", "a:b/c", "42")
                .await
        },
    )
    .await;
}

#[tokio::test]
async fn a_resource_version_of_128_visible_characters_is_sent() {
    // A token goes to 128 characters, and `~` is not an identifier's.
    let version = "~".repeat(128);
    let expected = version.clone();
    is_sent(
        "host/action",
        json!({"context": context_json(), "capability": "apps", "name": "web",
               "action": "sync", "uid": "u-1", "resourceVersion": expected}),
        move |host| {
            let version = version.clone();
            async move {
                host.action(&prod(), "apps", "web", "sync", "u-1", &version)
                    .await
            }
        },
    )
    .await;
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
    // The ninth is not sent until one is answered. Not fenced with `health`,
    // whose answer goes ahead of a queued call.
    assert!(
        host.next_within(Duration::from_millis(300)).await.is_none(),
        "the ninth call must wait for a slot"
    );
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

/// A host whose `maxConcurrentRequests` is `limit` is sent `in_flight` calls
/// at once, and the call after them waits until one is answered.
async fn calls_in_flight_under(limit: u64, in_flight: usize) {
    let sidecar = Sidecar::new("t", "1").operation("read", |ctx: Context, _: Value| async move {
        Ok::<_, Error>(ctx.host().read(&prod(), "apps").await?)
    });
    let mut host = FakeHost::start(sidecar);
    host.initialize_with_concurrency(limit).await;
    for _ in 0..=in_flight {
        host.request("read", json!({})).await;
    }
    let mut calls = Vec::new();
    for _ in 0..in_flight {
        calls.push(host.call().await);
    }
    assert!(
        host.next_within(Duration::from_millis(300)).await.is_none(),
        "the call after {in_flight} must wait for a slot"
    );
    // Answering one frees its slot: that operation's own answer, and the
    // waiting call, in either order.
    host.reply(&calls[0]["id"], Ok(json!(1))).await;
    let mut lines = [host.recv().await, host.recv().await];
    lines.sort_by_key(|m| m.get("method").is_some());
    assert_eq!(lines[0]["result"], json!(1), "{lines:?}");
    assert_eq!(lines[1]["method"], "host/read", "{lines:?}");
    // Clean up: nothing is left unread.
    for call in calls[1..].iter().chain([&lines[1]]) {
        host.reply(&call["id"], Ok(json!(1))).await;
    }
    for _ in 0..in_flight {
        host.recv().await;
    }
    host.finish().await.unwrap();
}

#[tokio::test]
async fn a_host_that_allows_two_calls_at_once_gets_two_and_the_third_waits() {
    calls_in_flight_under(2, 2).await;
}

#[tokio::test]
async fn a_host_that_allows_more_than_eight_still_gets_eight() {
    calls_in_flight_under(20, 8).await;
}

#[tokio::test]
async fn a_host_that_says_zero_is_read_as_eight_and_no_call_hangs() {
    calls_in_flight_under(0, 8).await;
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
async fn a_name_over_the_message_limit_is_refused_as_invalid_without_echoing_it_and_the_session_goes_on(
) {
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
        .expect_err("a name over the limit cannot be sent");
    // The name is over its own cap (253 bytes) long before the message limit,
    // so it is refused for that, and the refusal does not repeat the 5 MiB.
    assert!(
        matches!(&error, HostError::InvalidCall(why) if why.contains("`name`")),
        "the session with srelens is still live; the call alone was refused: {error:?}"
    );
    // The refusal must not echo the 5 MiB value. It is the rule's sentence
    // and the field's name, about 110 bytes, so a few hundred leaves room to
    // reword it and is nowhere near the size of the value.
    assert!(
        error.to_string().len() < 300,
        "{} bytes",
        error.to_string().len()
    );
    let health = host.request("health", json!({})).await;
    assert_eq!(host.answer(health).await["result"], json!({}));
    host.finish().await.unwrap();
}
