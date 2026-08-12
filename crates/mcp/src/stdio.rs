//! Minimal MCP server over a newline-delimited JSON-RPC 2.0 stream (the MCP
//! stdio transport). Implements `initialize`, `tools/list`, and `tools/call`
//! against the capability registry — so an external MCP client (Claude
//! Desktop, agents, IDEs) can list and call every capability.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::{McpServer, Transport};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Stable prefix on the text of a consent-denied tool result. CLI transports
/// strip `_meta`, so this text is the only denial signal that survives the
/// round trip through an agent CLI's transcript. `srelens_agent` defines the
/// same constant for its parsers (neither crate depends on the other); the
/// desktop crate has a test pinning the two equal.
pub const DENIED_PREFIX: &str = "consent denied: ";

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A `notifications/resources/updated` message. Carries only the URI — the
/// client re-reads to get content, which is what MCP specifies and what lets a
/// summary-level watch back a manifest subscription.
pub fn subscription_notification(uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/resources/updated",
        "params": { "uri": uri }
    })
}

/// Handle a single JSON-RPC request. Returns `None` for notifications (no id),
/// which must not produce a response.
pub async fn handle_request(
    server: &McpServer,
    req: &Value,
    transport: Transport,
) -> Option<Value> {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").cloned();

    match method {
        "initialize" => Some(ok(
            id?,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                // `subscribe` is transport-dependent: stdio can push
                // notifications on its own stdout, the POST-only HTTP transport
                // has no server-to-client channel at all (see issue #193).
                // `listChanged` is false because the resource list is two fixed
                // entries plus templates and never changes at runtime.
                "capabilities": {
                    "tools": {},
                    "prompts": {},
                    "resources": {
                        "subscribe": transport == Transport::Stdio,
                        "listChanged": false
                    }
                },
                "serverInfo": { "name": "srelens", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "ping" => Some(ok(id?, json!({}))),
        "notifications/initialized" | "initialized" => None,
        "tools/list" => {
            let tools: Vec<Value> = server
                .list_tools()
                .into_iter()
                .map(|t| {
                    let schema = if t.input_schema.is_null() {
                        json!({ "type": "object" })
                    } else {
                        t.input_schema
                    };
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": schema,
                        // MCP tool annotations. `readOnlyHint` lets a client
                        // (Cursor) auto-approve read-only calls in headless
                        // mode instead of rejecting them; destructive/consent-
                        // gated tools are `false` and stay gated by srelens's
                        // own confirm dialog.
                        "annotations": {
                            "readOnlyHint": t.read_only,
                            "destructiveHint": t.destructive,
                        },
                    })
                })
                .collect();
            Some(ok(id?, json!({ "tools": tools })))
        }
        "tools/call" => {
            let params = req.get("params");
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));

            // `_confirm` is a caller-supplied hint, never authorization: strip it
            // before the tool sees it, and let the injected policy decide.
            if let Some(obj) = args.as_object_mut() {
                obj.remove("_confirm");
            }
            // Deliberately re-read from `params` rather than reusing `args`: the
            // policy (e.g. `FlagGated`) needs to see `_confirm` to make its
            // decision, but the tool must never receive it. Two views of the
            // same call, scoped to who is allowed to see what.
            let raw_args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));

            let sensitive = server.is_sensitive(name);
            let mut decision = "auto";

            if let Some(kind) = server.consent_kind(name) {
                if let crate::policy::Decision::Denied(reason) =
                    server.confirm_policy().confirm(name, &raw_args, kind).await
                {
                    server.audit().record(crate::audit::AuditRecord {
                        transport,
                        tool: name.to_string(),
                        args: crate::audit::redact(&args, sensitive),
                        decision: "denied",
                        outcome: "error",
                        error: Some(reason.clone()),
                    });
                    // A result, not a transport error, so the agent can adapt.
                    // `_meta` (reserved by MCP for exactly this) marks the
                    // refusal so the in-process native agent can report "the
                    // user declined" rather than a failed execution; CLI
                    // clients strip it, which is why the text also carries
                    // `DENIED_PREFIX` — the only signal that survives a CLI's
                    // transcript for the srelens_agent parsers to match on.
                    return Some(ok(
                        id?,
                        json!({
                            "content": [{ "type": "text", "text": format!("{DENIED_PREFIX}{reason}") }],
                            "isError": true,
                            "_meta": { "srelens/denied": true }
                        }),
                    ));
                }
                decision = "approved";
            }

            let called = server.call_tool(name, args.clone()).await;
            server.audit().record(crate::audit::AuditRecord {
                transport,
                tool: name.to_string(),
                args: crate::audit::redact(&args, sensitive),
                decision,
                outcome: if called.is_ok() { "ok" } else { "error" },
                error: called.as_ref().err().map(|e| e.to_string()),
            });
            let result = match called {
                Ok(v) => json!({
                    "content": [{ "type": "text", "text": v.to_string() }],
                    "isError": false
                }),
                Err(e) => json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true
                }),
            };
            Some(ok(id?, result))
        }
        "prompts/list" => {
            let prompts: Vec<Value> = server
                .prompts()
                .list()
                .into_iter()
                .map(|p| {
                    let arguments: Vec<Value> = p
                        .arguments
                        .into_iter()
                        .map(|a| {
                            json!({
                                "name": a.name,
                                "description": a.description.unwrap_or_default(),
                                "required": a.required
                            })
                        })
                        .collect();
                    json!({
                        "name": p.name,
                        "description": p.description,
                        "arguments": arguments
                    })
                })
                .collect();
            Some(ok(id?, json!({ "prompts": prompts })))
        }
        "prompts/get" => {
            let params = req.get("params");
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            // MCP prompt arguments are strings; anything else is stringified
            // rather than rejected, so a client sending a number still works.
            // A JSON `null`, though, is dropped instead of becoming the string
            // "null": null means the argument is absent, and inserting it as
            // a value would let it pass the required/target presence checks
            // it should fail.
            let supplied: std::collections::BTreeMap<String, String> = params
                .and_then(|p| p.get("arguments"))
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            let s = match v {
                                Value::Null => return None,
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            Some((k.clone(), s))
                        })
                        .collect()
                })
                .unwrap_or_default();

            match server.prompts().get(name, &supplied) {
                Ok(rendered) => Some(ok(
                    id?,
                    json!({
                        "description": rendered.description,
                        "messages": [{
                            "role": "user",
                            "content": { "type": "text", "text": rendered.text }
                        }]
                    }),
                )),
                // Invalid params, not a tool error: the client sent a name or an
                // argument set this server cannot serve.
                Err(message) => Some(err(id?, -32602, &message)),
            }
        }
        "resources/list" => Some(ok(
            id?,
            json!({ "resources": crate::resources::fixed_resources() }),
        )),
        "resources/templates/list" => Some(ok(
            id?,
            json!({ "resourceTemplates": crate::resources::templates() }),
        )),
        "resources/read" => {
            let uri_str = req
                .get("params")
                .and_then(|p| p.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or("");

            let planned = crate::resources::ResourceUri::parse(uri_str)
                .and_then(|uri| crate::resources::plan_read(&uri, server.resources().as_ref()));

            let read = match planned {
                Ok(r) => r,
                Err(message) => return Some(err(id?, -32602, &message)),
            };

            // The catalog is assembled here rather than by invoking a
            // capability: it describes this server, not the cluster.
            let text = if read.capability == crate::resources::CATALOG_IN_PROCESS {
                json!({
                    "tools": server.list_tools().into_iter().map(|t| t.name).collect::<Vec<_>>(),
                    "prompts": server.prompts().list().into_iter().map(|p| p.name).collect::<Vec<_>>(),
                    "resourceTemplates": crate::resources::templates(),
                })
                .to_string()
            } else {
                // `McpServer::call_tool` is a bare registry invocation with no
                // gating or auditing of its own — that lives in the
                // `tools/call` arm, wrapped around the same call. This arm
                // reproduces both here so a resource read leaves the same
                // audit trail as the identical read via `tools/call`.
                if server.consent_kind(read.capability).is_some() {
                    // Unreachable today: `plan_read` only ever names the four
                    // hardcoded, unconditionally-read-only capabilities in
                    // `MAPPED_CAPABILITIES`, none of which are consent-gated.
                    // Guarded explicitly anyway, fail-closed, rather than
                    // assuming that stays true — clients auto-fetch
                    // resources, so raising a confirm dialog here (as
                    // `tools/call` does) would be exactly the consent-fatigue
                    // vector the design avoids.
                    let message = format!(
                        "{} is consent-gated and must be called as a tool, not read as a resource",
                        read.capability
                    );
                    server.audit().record(crate::audit::AuditRecord {
                        transport,
                        tool: read.capability.to_string(),
                        args: crate::audit::redact(
                            &read.args,
                            server.is_sensitive(read.capability),
                        ),
                        decision: "denied",
                        outcome: "error",
                        error: Some(message.clone()),
                    });
                    return Some(err(id?, -32602, &message));
                }

                let sensitive = server.is_sensitive(read.capability);
                let redacted_args = crate::audit::redact(&read.args, sensitive);
                let called = server.call_tool(read.capability, read.args.clone()).await;
                server.audit().record(crate::audit::AuditRecord {
                    transport,
                    tool: read.capability.to_string(),
                    args: redacted_args,
                    decision: "auto",
                    outcome: if called.is_ok() { "ok" } else { "error" },
                    error: called.as_ref().err().map(|e| e.to_string()),
                });

                match called {
                    Ok(v) => match &v {
                        // `getManifest` returns `{ yaml }` and `podLogs`
                        // `{ logs }`; unwrap those so the client gets the
                        // document, not a wrapper object. Keyed on the
                        // planned mime rather than "one string-valued key":
                        // a future capability that returns a single-key JSON
                        // object (e.g. `{ "currentContext": "prod" }`) must
                        // pass through as JSON, not be silently stripped to
                        // a bare string.
                        Value::Object(m) if read.mime != "application/json" && m.len() == 1 => m
                            .values()
                            .next()
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string()),
                        _ => v.to_string(),
                    },
                    Err(e) => return Some(err(id?, -32603, &e.to_string())),
                }
            };

            Some(ok(
                id?,
                json!({ "contents": [{ "uri": uri_str, "mimeType": read.mime, "text": text }] }),
            ))
        }
        _ => id.map(|id| err(id, -32601, "method not found")),
    }
}

/// Subscription methods, handled in the serve loop rather than in
/// `handle_request` — that function is shared with the POST-only HTTP transport,
/// which must keep answering -32601 because it cannot push.
///
/// Synchronous: nothing here awaits. The watch it spawns signals changes by
/// sending the canonical URI on `tx`, which the loop selects on.
fn handle_subscription(
    server: &std::sync::Arc<McpServer>,
    subs: &std::sync::Arc<crate::subscriptions::SubscriptionRegistry>,
    dirty: &std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
    wake: &tokio::sync::mpsc::Sender<()>,
    req: &Value,
    method: &str,
) -> Option<Value> {
    let id = req.get("id").cloned()?;
    let uri_str = req
        .get("params")
        .and_then(|p| p.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let uri = match crate::resources::ResourceUri::parse(uri_str) {
        Ok(u) => u,
        Err(message) => return Some(err(id, -32602, &message)),
    };
    // Canonical form, so two spellings of one object cannot make two watches.
    let canonical = uri.to_string();

    if method == "resources/unsubscribe" {
        subs.remove(&canonical);
        return Some(ok(id, json!({})));
    }

    if let Err(message) = crate::resources::is_subscribable(&uri) {
        return Some(err(id, -32602, &message));
    }
    // `plan_read` is entirely offline: it validates the URI's shape, scope,
    // and kind (and refuses Secrets) without checking that the capability it
    // would invoke is actually registered on this server. So this guarantees
    // the URI is well-formed and addressable — not that a later read will
    // succeed; a subscription can still outlive a URI whose capability is
    // missing or fails at call time. `server.resources()` matches the
    // resolver `plan_read` and the real read path both use, so this and the
    // eventual read agree on what's addressable.
    if let Err(message) = crate::resources::plan_read(&uri, server.resources().as_ref()) {
        return Some(err(id, -32602, &message));
    }

    // The callback is sync and owns only the dirty set and a wakeup sender,
    // so it needs no writer and no spawn. It never awaits, so the dirty
    // set's lock is only ever held for the insert itself — matching the
    // discipline in `subscriptions.rs`.
    let dirty = dirty.clone();
    let wake = wake.clone();
    let notify_uri = canonical.clone();
    let on_change = Box::new(move || {
        dirty
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(notify_uri.clone());
        // `try_send` never blocks, keeping this callback synchronous. If the
        // capacity-1 channel is already full, a wakeup is already pending —
        // dropping this one is correct, not a lost notification, because
        // that pending wakeup already guarantees `serve_loop` will come back
        // and drain the whole dirty set, which by then includes this URI too.
        let _ = wake.try_send(());
    });

    let handle = match server.watcher().watch(&uri, on_change) {
        Ok(h) => h,
        Err(message) => return Some(err(id, -32602, &message)),
    };
    if let Err(message) = subs.insert(canonical, handle) {
        return Some(err(id, -32602, &message));
    }
    Some(ok(id, json!({})))
}

/// Run the MCP stdio loop. Owns its writer and the subscription registry, so
/// no watch outlives this session — stdio's client spawned the process, so loop
/// exit *is* disconnect.
///
/// Watch callbacks are sync and cannot write asynchronously, so they cannot
/// hand the loop content directly. Instead they insert the changed URI into a
/// shared dirty set and signal a capacity-1 wakeup channel; this loop selects
/// on the wakeup alongside incoming requests and, once woken, drains the
/// whole dirty set. Because a notification carries only the URI (the client
/// re-reads for content), N queued changes to the same URI are indistinguishable
/// from one, so coalescing them is a strict improvement, not a tradeoff — it
/// also bounds pending memory at the number of distinct subscribed URIs
/// (at most `MAX_SUBSCRIPTIONS`) rather than growing without limit if the
/// client stops reading. Responses and notifications are still written by one
/// task and serialised by construction — no mutex around the writer, no
/// spawn, and the writer bound stays `AsyncWrite + Unpin`.
pub async fn serve<R, W>(server: McpServer, reader: R, mut writer: W) -> std::io::Result<()>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let server = std::sync::Arc::new(server);
    let subs = std::sync::Arc::new(crate::subscriptions::SubscriptionRegistry::new());
    let dirty: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::channel::<()>(1);

    let result =
        serve_loop(&server, &subs, &dirty, &wake_tx, &mut wake_rx, reader, &mut writer).await;
    // Release every watch however the loop ended — EOF or a write error. Doing
    // this here rather than inside the loop means the `?` error path cannot skip
    // it.
    subs.abort_all();
    result
}

async fn serve_loop<R, W>(
    server: &std::sync::Arc<McpServer>,
    subs: &std::sync::Arc<crate::subscriptions::SubscriptionRegistry>,
    dirty: &std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
    wake_tx: &tokio::sync::mpsc::Sender<()>,
    wake_rx: &mut tokio::sync::mpsc::Receiver<()>,
    reader: R,
    writer: &mut W,
) -> std::io::Result<()>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = reader.lines();

    loop {
        tokio::select! {
            // Biased so a pending request is always answered before
            // notifications, keeping request/response ordering deterministic.
            // The cost: `tokio::select!` with `biased` polls branches in
            // written order and returns on the first `Ready` one *without
            // polling the rest* — so whenever `lines.next_line()` resolves
            // immediately (a pipelined client, or several lines arriving in
            // one read), `rx.recv()` below is never polled that iteration,
            // and would starve every queued notification until EOF's drain.
            // The explicit `drain_notifications` call after each processed
            // line is what closes that gap; removing it silently turns
            // "pushed" notifications into "delivered whenever the client
            // happens to pause or disconnect".
            biased;

            line = lines.next_line() => {
                let Some(line) = line? else {
                    // EOF. Same starvation as above, one last time: drain
                    // whatever is already pending before leaving. No wakeup
                    // token is needed here — the dirty set is read directly.
                    drain_notifications(writer, dirty).await?;
                    return Ok(());
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(req) = serde_json::from_str::<Value>(trimmed) else { continue };

                let method = req.get("method").and_then(Value::as_str).unwrap_or("");
                // Subscription methods are handled here, not in
                // `handle_request`: that function is shared with the POST-only
                // HTTP transport, which has no channel to push notifications and
                // must keep answering -32601.
                let resp = if method == "resources/subscribe" || method == "resources/unsubscribe" {
                    handle_subscription(server, subs, dirty, wake_tx, &req, method)
                } else {
                    handle_request(server, &req, crate::Transport::Stdio).await
                };
                if let Some(resp) = resp {
                    write_line(writer, &resp).await?;
                }
                // `biased` means `wake_rx.recv()` below is never polled while
                // a line is ready — a pipelined client (several requests in
                // one read, or a watch racing ahead) would starve every
                // pending notification until EOF's drain otherwise. Draining
                // here after every line bounds delivery to at most one
                // request and keeps ordering deterministic: a request's own
                // response is written before any notification a concurrent
                // watch queued during its handling.
                drain_notifications(writer, dirty).await?;
            }

            Some(()) = wake_rx.recv() => {
                // The wakeup itself carries no payload — it only means "the
                // dirty set changed" — so draining reads the set directly
                // rather than the channel.
                drain_notifications(writer, dirty).await?;
            }
        }
    }
}

/// Write one notification per URI currently in the dirty set, then clear it,
/// without waiting for more to arrive. Shared by the per-line and EOF paths
/// so `biased` select's starvation gap (see `serve_loop`) is closed the same
/// way in both places.
///
/// Takes the whole set under the lock via `mem::take` and releases the lock
/// before writing anything — the lock is never held across an await, matching
/// the discipline in `subscriptions.rs`. `BTreeSet` yields URIs in sorted
/// order rather than arrival order; that's fine, and deliberate: each
/// notification is independent and carries no ordering meaning, it only
/// tells the client to re-read that one URI.
async fn drain_notifications<W>(
    writer: &mut W,
    dirty: &std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let uris = {
        let mut guard = dirty.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    for uri in uris {
        write_line(writer, &subscription_notification(&uri)).await?;
    }
    Ok(())
}

async fn write_line<W>(writer: &mut W, value: &Value) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let s = serde_json::to_string(value)?;
    writer.write_all(s.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::{Capability, Registry};
    use std::sync::Arc;
    use tokio::io::BufReader;

    fn server_with_ping() -> McpServer {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health check", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        McpServer::new(Arc::new(reg))
    }

    #[tokio::test]
    async fn initialize_returns_protocol_and_server_info() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "srelens");
    }

    #[tokio::test]
    async fn tools_list_includes_registry_capabilities() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "ping");
        assert_eq!(tools[0]["inputSchema"]["type"], "object");
        // A read-only capability advertises `readOnlyHint: true` so MCP clients
        // (Cursor) can auto-approve the call in non-interactive mode.
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        assert_eq!(tools[0]["annotations"]["destructiveHint"], false);
    }

    #[tokio::test]
    async fn tools_call_invokes_capability() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ping","arguments":"hi"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("echo"));
    }

    fn server_with_destructive() -> McpServer {
        use srelens_capability::{Annotations, Capability};
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("danger", "destructive", |_| async {
            Ok(json!({ "done": true }))
        });
        cap.annotations = Annotations::DESTRUCTIVE;
        reg.register(cap);
        McpServer::new(Arc::new(reg))
    }

    // `Capability`'s field set in this codebase includes `output_schema`,
    // which the brief's literal omits — so this mirrors `server_with_destructive`
    // (build via `Capability::read_only`, then override only what differs)
    // rather than constructing the struct literal directly.
    fn server_with_readonly() -> McpServer {
        use srelens_capability::{Annotations, Capability};
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("readit", "reads", |_| async {
            Ok(json!({ "ok": true }))
        });
        cap.annotations = Annotations::READ_ONLY;
        reg.register(cap);
        McpServer::new(Arc::new(reg))
    }

    #[tokio::test]
    async fn destructive_tool_is_gated_without_confirm() {
        let server = server_with_destructive();
        let resp = handle_request(
            &server,
            &json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"danger","arguments":{}}}),
            Transport::Http,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], true);
    }

    /// The bug this closes: the gate used to read `_confirm` from the caller's
    /// own arguments, so any client could authorize itself.
    #[tokio::test]
    async fn caller_supplied_confirm_does_not_authorize() {
        let server = server_with_destructive();
        let resp = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "danger", "arguments": { "_confirm": true } }
            }),
            Transport::Http,
        )
        .await
        .expect("response");
        assert_eq!(resp["result"]["isError"], json!(true), "expected denial, got {resp}");
    }

    #[tokio::test]
    async fn destructive_denied_when_no_policy_wired() {
        let server = server_with_destructive(); // default policy = AlwaysDeny
        let resp = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "danger", "arguments": {} }
            }),
            Transport::Http,
        )
        .await
        .expect("response");
        assert_eq!(resp["result"]["isError"], json!(true));
        // The refusal marker that lets the native agent report "user declined"
        // instead of a failed execution.
        assert_eq!(resp["result"]["_meta"]["srelens/denied"], json!(true));
        // And the text marker that survives CLI transports (which strip _meta).
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with(DENIED_PREFIX), "got: {text}");
    }

    /// The pair of assertions here is the point: the tool must never see
    /// `_confirm`, while the policy must. A regression that reordered the
    /// strip, or that started passing the stripped `args` to the policy
    /// instead of `raw_args`, would silently break `FlagGated` in production
    /// with nothing else in this suite catching it.
    #[tokio::test]
    async fn approving_policy_lets_the_tool_run_and_strips_confirm() {
        use srelens_capability::Annotations;
        use std::sync::Mutex;

        // Captures what each side actually received, rather than trusting
        // that "the tool ran" and "the policy approved" implies either saw
        // the right shape of arguments.
        let policy_saw: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let tool_saw: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));

        struct Yes(Arc<Mutex<Option<Value>>>);
        #[async_trait::async_trait]
        impl crate::policy::ConfirmPolicy for Yes {
            async fn confirm(
                &self,
                _t: &str,
                a: &serde_json::Value,
                _kind: crate::policy::ConsentKind,
            ) -> crate::policy::Decision {
                *self.0.lock().unwrap() = Some(a.clone());
                crate::policy::Decision::Approved
            }
        }

        let mut reg = Registry::new();
        let mut cap = {
            let tool_saw = tool_saw.clone();
            Capability::read_only("danger", "destructive", move |args| {
                let tool_saw = tool_saw.clone();
                async move {
                    *tool_saw.lock().unwrap() = Some(args.clone());
                    Ok(json!({ "done": true }))
                }
            })
        };
        cap.annotations = Annotations::DESTRUCTIVE;
        reg.register(cap);
        let server =
            McpServer::new(Arc::new(reg)).with_policy(Arc::new(Yes(policy_saw.clone())));

        let resp = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "danger", "arguments": { "_confirm": true } }
            }),
            Transport::Http,
        )
        .await
        .expect("response");
        assert_eq!(resp["result"]["isError"], json!(false), "got {resp}");

        let seen_by_tool = tool_saw.lock().unwrap().clone().expect("tool ran");
        assert!(
            seen_by_tool.get("_confirm").is_none(),
            "tool must not see _confirm, got {seen_by_tool}"
        );

        let seen_by_policy = policy_saw.lock().unwrap().clone().expect("policy consulted");
        assert_eq!(
            seen_by_policy.get("_confirm"),
            Some(&json!(true)),
            "policy must see _confirm, got {seen_by_policy}"
        );
    }

    #[tokio::test]
    async fn read_only_tool_never_consults_the_policy() {
        // AlwaysDeny is the default; a read-only tool must still work.
        let server = server_with_readonly();
        let resp = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "readit", "arguments": {} }
            }),
            Transport::Stdio,
        )
        .await
        .expect("response");
        assert_eq!(resp["result"]["isError"], json!(false), "got {resp}");
    }

    #[tokio::test]
    async fn notification_produces_no_response() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            Transport::Stdio,
        )
        .await;
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn every_tool_call_is_audited_with_its_decision() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Spy(Mutex<Vec<(String, &'static str, &'static str)>>);
        impl crate::audit::AuditSink for Spy {
            fn record(&self, rec: crate::audit::AuditRecord) {
                self.0.lock().unwrap().push((rec.tool, rec.decision, rec.outcome));
            }
        }
        let spy = Arc::new(Spy::default());
        let server = server_with_destructive().with_audit(spy.clone());

        let _ = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "danger", "arguments": {} }
            }),
            Transport::Http,
        )
        .await;

        let seen = spy.0.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "expected one audit record, got {seen:?}");
        assert_eq!(seen[0].0, "danger");
        assert_eq!(seen[0].1, "denied");
    }

    /// Sibling of `every_tool_call_is_audited_with_its_decision`, pinning the
    /// `"approved"` value: without this, `"approved"` and `"auto"` could be
    /// swapped in the implementation and no test would notice — `decision` is
    /// the whole point of the audit log.
    #[tokio::test]
    async fn a_destructive_call_approved_by_policy_is_audited_as_approved() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Spy(Mutex<Vec<(String, &'static str, &'static str)>>);
        impl crate::audit::AuditSink for Spy {
            fn record(&self, rec: crate::audit::AuditRecord) {
                self.0.lock().unwrap().push((rec.tool, rec.decision, rec.outcome));
            }
        }
        struct AlwaysApprove;
        #[async_trait::async_trait]
        impl crate::policy::ConfirmPolicy for AlwaysApprove {
            async fn confirm(
                &self,
                _tool: &str,
                _args: &Value,
                _kind: crate::policy::ConsentKind,
            ) -> crate::policy::Decision {
                crate::policy::Decision::Approved
            }
        }

        let spy = Arc::new(Spy::default());
        let server = server_with_destructive()
            .with_policy(Arc::new(AlwaysApprove))
            .with_audit(spy.clone());

        let _ = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "danger", "arguments": {} }
            }),
            Transport::Http,
        )
        .await;

        let seen = spy.0.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "expected one audit record, got {seen:?}");
        assert_eq!(seen[0].0, "danger");
        assert_eq!(seen[0].1, "approved");
    }

    /// Sibling pinning the `"auto"` value: a read-only tool never consults
    /// the policy, so it must be recorded as `"auto"`, not `"approved"`.
    #[tokio::test]
    async fn a_read_only_call_is_audited_as_auto() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Spy(Mutex<Vec<(String, &'static str, &'static str)>>);
        impl crate::audit::AuditSink for Spy {
            fn record(&self, rec: crate::audit::AuditRecord) {
                self.0.lock().unwrap().push((rec.tool, rec.decision, rec.outcome));
            }
        }

        let spy = Arc::new(Spy::default());
        let server = server_with_readonly().with_audit(spy.clone());

        let _ = handle_request(
            &server,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "readit", "arguments": {} }
            }),
            Transport::Stdio,
        )
        .await;

        let seen = spy.0.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "expected one audit record, got {seen:?}");
        assert_eq!(seen[0].0, "readit");
        assert_eq!(seen[0].1, "auto");
    }

    #[tokio::test]
    async fn serve_processes_a_session_over_the_stream() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ping","arguments":"yo"}}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        serve(server_with_ping(), BufReader::new(input.as_bytes()), &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        // Two responses (initialize + tools/call); the notification yields none.
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("protocolVersion"));
        assert!(lines[1].contains("echo"));
    }

    #[tokio::test]
    async fn initialize_advertises_prompts() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert!(
            resp["result"]["capabilities"]["prompts"].is_object(),
            "a client that is not told about prompts will never ask for them: {resp}"
        );
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn prompts_list_returns_the_builtin_flows() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":2,"method":"prompts/list"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let prompts = resp["result"]["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 4);
        let names: Vec<&str> = prompts.iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"pod-crashloop"), "got {names:?}");
        let first = &prompts[0];
        assert!(first["description"].is_string());
        let args = first["arguments"].as_array().unwrap();
        assert!(args.iter().any(|a| a["name"] == "context" && a["required"] == true));
    }

    #[tokio::test]
    async fn prompts_get_returns_a_rendered_user_message() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{
                "name":"pod-crashloop",
                "arguments":{"context":"kind","namespace":"prod","pod":"web-0"}
            }}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let messages = resp["result"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"]["type"], "text");
        let text = messages[0]["content"]["text"].as_str().unwrap();
        assert!(text.contains("web-0"));
        assert!(text.contains("prod"));
        assert!(!text.contains("{{"), "no placeholder may reach the agent");
    }

    #[tokio::test]
    async fn prompts_get_rejects_an_unknown_name_as_invalid_params() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":4,"method":"prompts/get","params":{"name":"nope"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602, "got {resp}");
        assert!(resp["error"]["message"].as_str().unwrap().contains("nope"));
    }

    #[tokio::test]
    async fn prompts_get_rejects_a_missing_required_argument() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":5,"method":"prompts/get","params":{
                "name":"pod-crashloop","arguments":{}
            }}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("context"));
    }

    /// Prompts touch no cluster, so there is nothing to account for. The reads
    /// the agent then makes are audited as ordinary tool calls.
    #[tokio::test]
    async fn prompts_are_not_audited() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Spy(Mutex<usize>);
        impl crate::audit::AuditSink for Spy {
            fn record(&self, _rec: crate::audit::AuditRecord) {
                *self.0.lock().unwrap() += 1;
            }
        }
        let spy = Arc::new(Spy::default());
        let server = server_with_ping().with_audit(spy.clone());

        for method in ["prompts/list", "prompts/get"] {
            let _ = handle_request(
                &server,
                &json!({"jsonrpc":"2.0","id":1,"method":method,"params":{
                    "name":"pod-crashloop","arguments":{"context":"kind"}
                }}),
                Transport::Stdio,
            )
            .await;
        }
        assert_eq!(*spy.0.lock().unwrap(), 0, "prompts must not be audited");
    }

    /// `context` is the one argument documented as never safe to guess, so a
    /// present-but-blank value must be rejected the same as an absent one —
    /// otherwise the agent renders instructions against an empty context and
    /// either fails every downstream call or invents one.
    #[tokio::test]
    async fn prompts_get_rejects_a_blank_required_argument() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":9,"method":"prompts/get","params":{
                "name":"pod-crashloop","arguments":{"context": "", "pod": "web-0"}
            }}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602, "got {resp}");
        assert!(resp["error"]["message"].as_str().unwrap().contains("context"));
    }

    /// A JSON `null` is the absence of a value, not the string "null". A
    /// client whose templating omits an unset variable by emitting `null`
    /// (common) must trip the same required-argument error an omitted key
    /// would.
    #[tokio::test]
    async fn prompts_get_rejects_a_null_required_argument() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":6,"method":"prompts/get","params":{
                "name":"pod-crashloop","arguments":{"context": null}
            }}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602, "got {resp}");
        assert!(resp["error"]["message"].as_str().unwrap().contains("context"));
    }

    /// A `null` target argument must not count as "supplied": it must fall
    /// back to discover mode rather than rendering a targeted flow with a
    /// null-ish resource name baked in.
    #[tokio::test]
    async fn prompts_get_treats_a_null_target_as_absent_and_uses_discover_mode() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":7,"method":"prompts/get","params":{
                "name":"pod-crashloop",
                "arguments":{"context":"kind","pod":null}
            }}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let text = resp["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(
            text.contains("k8s.listPods"),
            "expected discover-mode body, got {text}"
        );
    }

    /// Sibling of the two null-handling tests above, pinning that a
    /// non-null coercion (a JSON number) still stringifies and is accepted
    /// as a supplied target — only `null` is special-cased.
    #[tokio::test]
    async fn prompts_get_still_coerces_a_non_null_target_value() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":8,"method":"prompts/get","params":{
                "name":"pod-crashloop",
                "arguments":{"context":"kind","pod":123}
            }}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let text = resp["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("123"), "got {text}");
    }

    fn server_with_kinds() -> McpServer {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                match kind {
                    "Pod" => Some(crate::resources::KindScope::Namespaced),
                    "Node" => Some(crate::resources::KindScope::ClusterScoped),
                    _ => None,
                }
            }
        }
        struct StubWatcher;
        impl crate::resources::ObjectWatcher for StubWatcher {
            fn watch(
                &self,
                _uri: &crate::resources::ResourceUri,
                _on_change: Box<dyn FnMut() + Send>,
            ) -> Result<tokio::task::AbortHandle, String> {
                Ok(tokio::spawn(async { std::future::pending::<()>().await }).abort_handle())
            }
        }
        server_with_ping().with_resources(Arc::new(Kinds)).with_watcher(Arc::new(StubWatcher))
    }

    #[tokio::test]
    async fn initialize_advertises_resources_with_subscribe_true_on_stdio() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let caps = &resp["result"]["capabilities"];
        assert_eq!(caps["resources"]["subscribe"], json!(true));
        assert_eq!(caps["resources"]["listChanged"], json!(false));
        assert!(caps["tools"].is_object(), "tools must survive");
        assert!(caps["prompts"].is_object(), "prompts must survive");
    }

    /// The HTTP transport is POST-only with no server-to-client channel, so it
    /// must not claim a capability it cannot honour.
    #[tokio::test]
    async fn initialize_advertises_subscribe_false_on_http() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            Transport::Http,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["capabilities"]["resources"]["subscribe"], json!(false));
    }

    #[tokio::test]
    async fn resources_list_returns_the_fixed_pair() {
        let resp = handle_request(
            &server_with_kinds(),
            &json!({"jsonrpc":"2.0","id":2,"method":"resources/list"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["resources"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn resources_templates_list_returns_four_templates() {
        let resp = handle_request(
            &server_with_kinds(),
            &json!({"jsonrpc":"2.0","id":3,"method":"resources/templates/list"}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["resourceTemplates"].as_array().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn resources_read_rejects_a_secret_uri_with_invalid_params() {
        let resp = handle_request(
            &server_with_kinds(),
            &json!({"jsonrpc":"2.0","id":4,"method":"resources/read",
                    "params":{"uri":"k8s://c/ns/Secret/db-creds"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("k8s.getSecret"));
    }

    #[tokio::test]
    async fn resources_read_rejects_a_malformed_uri() {
        let resp = handle_request(
            &server_with_kinds(),
            &json!({"jsonrpc":"2.0","id":5,"method":"resources/read",
                    "params":{"uri":"not-a-uri"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602);
    }

    /// `k8s://catalog` is assembled in-process, so it works with no cluster.
    #[tokio::test]
    async fn resources_read_serves_the_catalog_without_a_cluster() {
        let resp = handle_request(
            &server_with_kinds(),
            &json!({"jsonrpc":"2.0","id":6,"method":"resources/read",
                    "params":{"uri":"k8s://catalog"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        let contents = &resp["result"]["contents"][0];
        assert_eq!(contents["uri"], "k8s://catalog");
        assert_eq!(contents["mimeType"], "application/json");
        let text = contents["text"].as_str().unwrap();
        assert!(text.contains("ping"), "the catalog must list tools: {text}");
    }

    #[tokio::test]
    async fn resources_read_without_a_resolver_addresses_no_objects() {
        // `server_with_ping()` wires no resolver, so `NoKinds` applies.
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":7,"method":"resources/read",
                    "params":{"uri":"k8s://c/ns/Pod/web-0"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32602);
    }

    fn server_with_kinds_and_manifest(
        handler: impl Fn(Value) -> Result<Value, srelens_capability::CapabilityError>
            + Send
            + Sync
            + 'static,
    ) -> McpServer {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                match kind {
                    "Pod" => Some(crate::resources::KindScope::Namespaced),
                    _ => None,
                }
            }
        }
        let handler = Arc::new(handler);
        let mut reg = Registry::new();
        reg.register(Capability::read_only("k8s.getManifest", "reads a manifest", move |v| {
            let handler = handler.clone();
            async move { handler(v) }
        }));
        McpServer::new(Arc::new(reg)).with_resources(Arc::new(Kinds))
    }

    /// The vulnerability Finding 1 closes: `McpServer::call_tool` is a bare
    /// registry invocation with no gating or auditing of its own, so a
    /// resource read that skipped straight to it would leave zero trail
    /// while the identical read via `tools/call` is logged. The audit
    /// record must name the underlying capability, not `"resources/read"`,
    /// since that is what an operator needs to see what actually touched
    /// the cluster.
    #[tokio::test]
    async fn resources_read_records_one_audit_entry_naming_the_underlying_capability() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Spy(Mutex<Vec<(String, &'static str, &'static str)>>);
        impl crate::audit::AuditSink for Spy {
            fn record(&self, rec: crate::audit::AuditRecord) {
                self.0.lock().unwrap().push((rec.tool, rec.decision, rec.outcome));
            }
        }

        let spy = Arc::new(Spy::default());
        let server = server_with_kinds_and_manifest(|_| Ok(json!({ "yaml": "kind: Pod" })))
            .with_audit(spy.clone());

        let resp = handle_request(
            &server,
            &json!({"jsonrpc":"2.0","id":10,"method":"resources/read",
                    "params":{"uri":"k8s://c/ns/Pod/web-0"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["contents"][0]["text"], "kind: Pod", "got {resp}");

        let seen = spy.0.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "expected exactly one audit record, got {seen:?}");
        assert_eq!(seen[0].0, "k8s.getManifest", "must name the underlying capability");
        assert_eq!(seen[0].1, "auto");
        assert_eq!(seen[0].2, "ok");
    }

    /// Finding 2: no existing test reached a real `call_tool` failure through
    /// the read path — every other case is either the in-process catalog or
    /// rejected earlier by `plan_read` with `-32602`.
    #[tokio::test]
    async fn resources_read_surfaces_a_capability_failure_as_internal_error() {
        let server = server_with_kinds_and_manifest(|_| {
            Err(srelens_capability::CapabilityError::Handler("no such pod".to_string()))
        });

        let resp = handle_request(
            &server,
            &json!({"jsonrpc":"2.0","id":11,"method":"resources/read",
                    "params":{"uri":"k8s://c/ns/Pod/web-0"}}),
            Transport::Stdio,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32603, "got {resp}");
        assert!(
            resp["error"]["message"].as_str().unwrap().contains("no such pod"),
            "got {resp}"
        );
    }

    #[test]
    fn a_subscription_notification_is_a_jsonrpc_notification_with_only_the_uri() {
        let n = subscription_notification("k8s://c/ns/Pod/web-0");
        assert_eq!(n["jsonrpc"], "2.0");
        assert_eq!(n["method"], "notifications/resources/updated");
        assert_eq!(n["params"]["uri"], "k8s://c/ns/Pod/web-0");
        assert!(n.get("id").is_none(), "a notification must carry no id");
        // Content is never pushed; the client re-reads.
        assert!(n["params"].get("contents").is_none());
        assert!(n["params"].get("text").is_none());
    }

    /// The HTTP transport has no server-to-client channel, so subscribe must
    /// never be served there — `handle_request` (which HTTP uses) must not know
    /// the method at all.
    #[tokio::test]
    async fn handle_request_does_not_serve_subscribe() {
        let resp = handle_request(
            &server_with_ping(),
            &json!({"jsonrpc":"2.0","id":1,"method":"resources/subscribe",
                    "params":{"uri":"k8s://c/ns/Pod/p"}}),
            Transport::Http,
        )
        .await
        .unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    /// End-to-end over the real `serve` loop: subscribe, then unsubscribe, and
    /// confirm both are answered. The watch itself needs a cluster, so this
    /// asserts the protocol handling and registry bookkeeping.
    #[tokio::test]
    async fn serve_answers_subscribe_and_unsubscribe() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"resources/unsubscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        serve(server_with_kinds(), BufReader::new(input.as_bytes()), &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one response each: {text}");
        for line in lines {
            let v: Value = serde_json::from_str(line).unwrap();
            assert!(v.get("result").is_some(), "expected success, got {line}");
        }
    }

    /// A watcher that records whether it fired, via a real async wakeup
    /// rather than a flag a test would have to poll. Used by the two tests
    /// below to synchronize deterministically on "the watch has run and
    /// queued its notification" instead of racing a background task against
    /// the test's own progress.
    struct FiringWatcher(std::sync::Arc<tokio::sync::Notify>);
    impl crate::resources::ObjectWatcher for FiringWatcher {
        fn watch(
            &self,
            _uri: &crate::resources::ResourceUri,
            mut on_change: Box<dyn FnMut() + Send>,
        ) -> Result<tokio::task::AbortHandle, String> {
            let fired = self.0.clone();
            Ok(tokio::spawn(async move {
                on_change();
                fired.notify_one();
                std::future::pending::<()>().await
            })
            .abort_handle())
        }
    }

    /// Drives `serve` over a real async pipe pair (`tokio::io::duplex`)
    /// rather than the in-memory `&[u8]`/`Vec<u8>` every other test in this
    /// module uses.
    ///
    /// An in-memory reader/writer never returns `Pending`: `AsyncRead for
    /// &[u8]` and `AsyncWrite for Vec<u8>` both resolve every poll
    /// immediately. That gives a single-threaded executor no principled
    /// reason to ever revisit its run queue, so a freshly `tokio::spawn`ed
    /// watch task is never polled before the session ends — which is exactly
    /// why `a_watch_event_pushes_a_notification_onto_the_stream` and the
    /// pipelining test below need to observe a watch firing at all. An
    /// earlier version of this fix tried `#[tokio::test(flavor =
    /// "multi_thread")]` with the in-memory fixture instead, reasoning that
    /// production (`apps/desktop/src-tauri/src/main.rs`'s `run_mcp_stdio`)
    /// runs on a multi-thread runtime, so an idle worker would pick up the
    /// spawned task regardless of whether this loop's own task ever yields.
    /// That is true in isolation, but it is a race against OS thread
    /// scheduling, not a `Pending`-driven guarantee: run as part of this
    /// crate's full, highly parallel test binary (~200 concurrently
    /// scheduled tests contending for CPU), it failed reliably (5/5), not
    /// just occasionally. A `DuplexStream` genuinely returns `Pending` when
    /// its peer has not written anything yet — exactly like a real stdio
    /// pipe — so it gives the executor a real, load-independent reason to
    /// poll the spawned watch task, making these tests deterministic under
    /// any scheduling flavor or system load.
    fn spawn_serve_over_duplex(
        server: McpServer,
    ) -> (
        tokio::io::DuplexStream,
        tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
        tokio::task::JoinHandle<std::io::Result<()>>,
    ) {
        let (client_in, server_in) = tokio::io::duplex(8192);
        let (server_out, client_out) = tokio::io::duplex(8192);
        let serve_task = tokio::spawn(async move {
            serve(server, BufReader::new(server_in), server_out).await
        });
        (client_in, BufReader::new(client_out).lines(), serve_task)
    }

    /// The notification path end to end, without a cluster: a stub watcher
    /// fires `on_change` immediately, so this proves a watch event actually
    /// reaches the client's stream — not just that the message shape is right.
    #[tokio::test]
    async fn a_watch_event_pushes_a_notification_onto_the_stream() {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }

        let fired = std::sync::Arc::new(tokio::sync::Notify::new());
        let server = server_with_ping()
            .with_resources(Arc::new(Kinds))
            .with_watcher(Arc::new(FiringWatcher(fired.clone())));

        let (mut client_in, mut out_lines, serve_task) = spawn_serve_over_duplex(server);

        client_in
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}
"#,
            )
            .await
            .unwrap();

        // Deterministic: wait for the watch to actually have run and queued
        // its notification, rather than racing it against this test ending
        // the session.
        fired.notified().await;

        drop(client_in); // EOF: nothing more is coming.
        serve_task.await.unwrap().unwrap();

        let mut text = String::new();
        while let Some(line) = out_lines.next_line().await.unwrap() {
            text.push_str(&line);
            text.push('\n');
        }
        assert!(
            text.contains("notifications/resources/updated"),
            "a watch event must push a notification: {text}"
        );
        assert!(text.contains("k8s://c/ns/Pod/web-0"), "the notification must name the URI: {text}");
    }

    /// The bug the per-line drain in `serve_loop` closes: `biased` select
    /// polls `rx.recv()` only when no line is ready, so a pipelined client —
    /// several requests arriving in one read before the loop catches up —
    /// would starve every queued notification until EOF, turning "pushed"
    /// into "delivered whenever the client happens to pause or disconnect".
    /// This asserts the notification is written right after the subscribe
    /// response, well before the last of the pipelined responses — i.e. that
    /// it did not wait for EOF's drain to appear.
    ///
    /// Unlike `a_watch_event_pushes_a_notification_onto_the_stream`, this
    /// watcher fires `on_change` *synchronously inside `watch()` itself*,
    /// before `handle_subscription` even returns, rather than from a spawned
    /// task. That is deliberate, not a shortcut: it makes "the notification
    /// is already in the channel by the time line 1 finishes processing" a
    /// plain causal fact of call order, not a race against when some other
    /// task happens to be scheduled — so this test is deterministic under
    /// the default single-threaded `#[tokio::test]` runtime regardless of
    /// how many other tests are contending for CPU, and can use the same
    /// plain `&[u8]`/`Vec<u8>` fixture as every other non-watch test in this
    /// module. It is still a faithful reproduction of the bug: what matters
    /// for `biased` starvation is only that the notification is ready
    /// *before* the loop finishes draining an already-buffered run of lines,
    /// not how it got that way.
    #[tokio::test]
    async fn a_pipelined_client_still_gets_the_notification_before_the_session_ends() {
        struct ImmediatelyFiringWatcher;
        impl crate::resources::ObjectWatcher for ImmediatelyFiringWatcher {
            fn watch(
                &self,
                _uri: &crate::resources::ResourceUri,
                mut on_change: Box<dyn FnMut() + Send>,
            ) -> Result<tokio::task::AbortHandle, String> {
                on_change();
                Ok(tokio::spawn(async { std::future::pending::<()>().await }).abort_handle())
            }
        }

        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }

        let server = server_with_ping()
            .with_resources(Arc::new(Kinds))
            .with_watcher(Arc::new(ImmediatelyFiringWatcher));

        // Subscribe, then a burst of buffered requests arriving as one
        // pipelined batch — exactly the shape that starves `rx.recv()` under
        // `biased` select without a per-line drain, because `next_line()`
        // resolves synchronously for every one of these without ever giving
        // the executor cause to look elsewhere.
        let mut input = String::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}"#,
        );
        input.push('\n');
        for i in 2..22 {
            input.push_str(&format!(r#"{{"jsonrpc":"2.0","id":{i},"method":"ping"}}"#));
            input.push('\n');
        }
        let mut out: Vec<u8> = Vec::new();
        serve(server, BufReader::new(input.as_bytes()), &mut out).await.unwrap();

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let notification_pos = lines
            .iter()
            .position(|l| l.contains("notifications/resources/updated"))
            .expect("a watch event must push a notification");
        assert!(
            notification_pos < lines.len() - 1,
            "the notification must not be pushed only by the EOF drain, \
             i.e. it must appear before the final response: {text}"
        );
    }

    /// A watcher whose spawned task calls `on_change` several times in a
    /// tight loop with no `.await` between calls, so all the calls complete
    /// within a single poll of the task — there is no scheduling point at
    /// which the loop could interleave a drain between them. That makes the
    /// dirty-set coalescing this proves deterministic rather than a race:
    /// however the loop happens to be scheduled, all `times` inserts land in
    /// the dirty set before it can ever be drained, so exactly one entry for
    /// this URI survives regardless.
    struct RepeatFiringWatcher {
        times: usize,
        fired: std::sync::Arc<tokio::sync::Notify>,
    }
    impl crate::resources::ObjectWatcher for RepeatFiringWatcher {
        fn watch(
            &self,
            _uri: &crate::resources::ResourceUri,
            mut on_change: Box<dyn FnMut() + Send>,
        ) -> Result<tokio::task::AbortHandle, String> {
            let fired = self.fired.clone();
            let times = self.times;
            Ok(tokio::spawn(async move {
                for _ in 0..times {
                    on_change();
                }
                fired.notify_one();
                std::future::pending::<()>().await
            })
            .abort_handle())
        }
    }

    /// The coalescing this closes: without it, 5 `on_change` calls to one
    /// subscribed URI would queue 5 identical notifications, making the
    /// client re-read the same URI 5 times to learn the same thing once.
    /// Bounding pending memory at "distinct subscribed URIs" rather than
    /// "watch events ever fired" depends on this actually deduplicating.
    #[tokio::test]
    async fn repeated_changes_to_one_uri_coalesce_to_fewer_notifications() {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }

        let fired = std::sync::Arc::new(tokio::sync::Notify::new());
        let server = server_with_ping().with_resources(Arc::new(Kinds)).with_watcher(Arc::new(
            RepeatFiringWatcher { times: 5, fired: fired.clone() },
        ));

        let (mut client_in, mut out_lines, serve_task) = spawn_serve_over_duplex(server);

        client_in
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}
"#,
            )
            .await
            .unwrap();

        // Deterministic: wait for all 5 `on_change` calls to have actually
        // run and been inserted into the dirty set, rather than racing them
        // against this test ending the session.
        fired.notified().await;

        drop(client_in); // EOF: nothing more is coming.
        serve_task.await.unwrap().unwrap();

        let mut text = String::new();
        while let Some(line) = out_lines.next_line().await.unwrap() {
            text.push_str(&line);
            text.push('\n');
        }
        let count = text.matches("notifications/resources/updated").count();
        assert_eq!(
            count, 1,
            "5 on_change calls to the same URI must coalesce to exactly one notification: {text}"
        );
    }

    /// Sibling of the coalescing test above, pinning the other half: distinct
    /// URIs must never collapse into each other just because they share a
    /// drain pass. A dirty set keyed on the whole URI is what this depends
    /// on; a regression that, say, coalesced by kind or by drain pass alone
    /// would break this while leaving the single-URI test above green.
    #[tokio::test]
    async fn two_different_subscribed_uris_each_still_get_a_notification() {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }

        struct TwoUriWatcher {
            fired_a: std::sync::Arc<tokio::sync::Notify>,
            fired_b: std::sync::Arc<tokio::sync::Notify>,
        }
        impl crate::resources::ObjectWatcher for TwoUriWatcher {
            fn watch(
                &self,
                uri: &crate::resources::ResourceUri,
                mut on_change: Box<dyn FnMut() + Send>,
            ) -> Result<tokio::task::AbortHandle, String> {
                let fired = if uri.to_string().contains("web-0") {
                    self.fired_a.clone()
                } else {
                    self.fired_b.clone()
                };
                Ok(tokio::spawn(async move {
                    on_change();
                    fired.notify_one();
                    std::future::pending::<()>().await
                })
                .abort_handle())
            }
        }

        let fired_a = std::sync::Arc::new(tokio::sync::Notify::new());
        let fired_b = std::sync::Arc::new(tokio::sync::Notify::new());
        let server = server_with_ping().with_resources(Arc::new(Kinds)).with_watcher(Arc::new(
            TwoUriWatcher { fired_a: fired_a.clone(), fired_b: fired_b.clone() },
        ));

        let (mut client_in, mut out_lines, serve_task) = spawn_serve_over_duplex(server);

        client_in
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}
"#,
            )
            .await
            .unwrap();
        client_in
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-1"}}
"#,
            )
            .await
            .unwrap();

        fired_a.notified().await;
        fired_b.notified().await;

        drop(client_in); // EOF: nothing more is coming.
        serve_task.await.unwrap().unwrap();

        let mut text = String::new();
        while let Some(line) = out_lines.next_line().await.unwrap() {
            text.push_str(&line);
            text.push('\n');
        }
        let count = text.matches("notifications/resources/updated").count();
        assert_eq!(count, 2, "two distinct URIs must not coalesce into one: {text}");
        assert!(text.contains("web-0"), "got {text}");
        assert!(text.contains("web-1"), "got {text}");
    }

    /// stdio's client spawned the process, so loop exit IS disconnect — nothing
    /// may outlive it. This is the leak the registry exists to prevent.
    ///
    /// This does not assert `AbortHandle::is_finished()` synchronously after
    /// `serve` returns: `crates/mcp/src/subscriptions.rs`'s test module
    /// documents (see `spawn_forever_joined`) that under this crate's
    /// current-thread `#[tokio::test]` runtime, a freshly spawned task is
    /// never even polled before such a synchronous check runs — measured at
    /// 0/50 passes, independently reproduced at 0/20. Instead this retains the
    /// watch's `JoinHandle` too and awaits it under `assert_aborted`'s bound,
    /// which synchronizes on the cancellation actually completing rather than
    /// racing the executor, and fails fast with a named message rather than
    /// hanging if the abort never happens.
    #[tokio::test]
    async fn serve_aborts_every_subscription_when_the_loop_exits() {
        use std::sync::{Arc as StdArc, Mutex};

        type RecordedHandles = StdArc<Mutex<Option<(tokio::task::AbortHandle, tokio::task::JoinHandle<()>)>>>;

        // Records the handles the watcher handed out, so the test can check
        // the watch was aborted after `serve` returned.
        #[derive(Clone)]
        struct Recording(RecordedHandles);
        impl crate::resources::ObjectWatcher for Recording {
            fn watch(
                &self,
                _uri: &crate::resources::ResourceUri,
                _on_change: Box<dyn FnMut() + Send>,
            ) -> Result<tokio::task::AbortHandle, String> {
                let join = tokio::spawn(async { std::future::pending::<()>().await });
                let abort = join.abort_handle();
                *self.0.lock().unwrap() = Some((abort.clone(), join));
                Ok(abort)
            }
        }

        let recorded = StdArc::new(Mutex::new(None));
        let server = server_with_kinds().with_watcher(Arc::new(Recording(recorded.clone())));

        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        serve(server, BufReader::new(input.as_bytes()), &mut out).await.unwrap();

        let (_, join) = recorded.lock().unwrap().take().expect("a watch was started");
        match tokio::time::timeout(std::time::Duration::from_secs(2), join).await {
            Ok(Ok(())) => panic!("watch task ran to completion instead of being aborted"),
            Ok(Err(join_err)) => assert!(
                join_err.is_cancelled(),
                "the watch must be aborted when the loop exits, but it ended for another reason: {join_err}"
            ),
            Err(_) => panic!(
                "timed out after 2s waiting for the watch to be aborted — it was never cancelled"
            ),
        }
    }

    #[tokio::test]
    async fn serve_rejects_subscribing_to_a_non_subscribable_uri() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://catalog"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/p/logs"}}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        serve(server_with_kinds(), BufReader::new(input.as_bytes()), &mut out)
            .await
            .unwrap();
        for line in String::from_utf8(out).unwrap().lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["error"]["code"], -32602, "got {line}");
        }
    }

    /// `NoWatcher` is unit-tested directly in `resources.rs`, but that alone
    /// does not prove the fail-closed default is actually reachable through
    /// the real subscribe path — a wiring bug could leave it dead code while
    /// every real server accidentally got a working watcher some other way.
    /// This drives `resources/subscribe` through `serve` on a server built
    /// with plain `McpServer::new` — no `.with_watcher(...)` at all — so it
    /// falls back to `NoWatcher`, and checks the rejection actually happens
    /// end to end.
    #[tokio::test]
    async fn serve_rejects_a_subscription_when_no_watcher_is_wired() {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }
        // Deliberately no `.with_watcher(...)`: this is what a host that
        // wires resources but forgets (or has no) cluster watcher looks like.
        let server = server_with_ping().with_resources(Arc::new(Kinds));

        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"k8s://c/ns/Pod/web-0"}}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        serve(server, BufReader::new(input.as_bytes()), &mut out).await.unwrap();

        let text = String::from_utf8(out).unwrap();
        let v: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["error"]["code"], -32602, "got {text}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains("no cluster watcher"),
            "the message must explain no watcher is wired, got {text}"
        );
    }
}
