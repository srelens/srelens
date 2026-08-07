//! Minimal MCP server over a newline-delimited JSON-RPC 2.0 stream (the MCP
//! stdio transport). Implements `initialize`, `tools/list`, and `tools/call`
//! against the capability registry — so an external MCP client (Claude
//! Desktop, agents, IDEs) can list and call every capability.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::{McpServer, Transport};

const PROTOCOL_VERSION: &str = "2024-11-05";

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
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
                "capabilities": { "tools": {}, "prompts": {} },
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
                    json!({ "name": t.name, "description": t.description, "inputSchema": schema })
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
                    return Some(ok(
                        id?,
                        json!({
                            "content": [{ "type": "text", "text": reason }],
                            "isError": true
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
        _ => id.map(|id| err(id, -32601, "method not found")),
    }
}

/// Run the MCP stdio loop: read newline-delimited JSON-RPC requests from
/// `reader`, write responses to `writer`, until EOF.
pub async fn serve<R, W>(server: McpServer, reader: R, mut writer: W) -> std::io::Result<()>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(resp) = handle_request(&server, &req, crate::Transport::Stdio).await {
            let s = serde_json::to_string(&resp)?;
            writer.write_all(s.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    Ok(())
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
}
