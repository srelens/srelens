//! In-process autonomous SRE agent for SRElens TUI.
//!
//! Drives srelens's MCP tools directly in-process via `srelens_mcp::stdio::handle_request`
//! and runs the multi-turn agentic loop (`srelens_llm::agent_loop::run`), emitting streaming
//! events back into Ratatui's event loop.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use serde_json::{json, Value};
use srelens_agent::event::AgentEvent;
use srelens_kube::client_cache::ClientCache;
use srelens_llm::types::{ToolDef, Turn};
use srelens_llm::{LlmError, ToolCallResult, ToolInvoker};
use srelens_mcp::McpServer;

use crate::event::AppEvent;

/// A `ToolInvoker` backed by an in-process `McpServer`.
pub struct McpToolInvoker {
    server: Arc<McpServer>,
    aliases: Mutex<HashMap<String, String>>,
}

impl McpToolInvoker {
    pub fn new(server: Arc<McpServer>) -> Self {
        Self {
            server,
            aliases: Default::default(),
        }
    }
}

fn provider_safe_name(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { c } else { '_' })
        .collect()
}

fn assign_alias(aliases: &mut HashMap<String, String>, id: &str) -> String {
    let mut alias = provider_safe_name(id);
    while aliases.get(&alias).is_some_and(|existing| existing != id) {
        alias.push('_');
    }
    aliases.insert(alias.clone(), id.to_string());
    alias
}

fn tool_def_from_json(v: &Value) -> ToolDef {
    ToolDef {
        name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        description: v.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
        input_schema: v.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
        read_only: v
            .get("annotations")
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[async_trait]
impl ToolInvoker for McpToolInvoker {
    async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let resp = srelens_mcp::stdio::handle_request(&self.server, &req, srelens_mcp::Transport::Http)
            .await
            .ok_or_else(|| LlmError::Api("tools/list returned no response".into()))?;
        let tools = resp
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut aliases = self.aliases.lock().unwrap();
        Ok(tools
            .iter()
            .map(|v| {
                let mut def = tool_def_from_json(v);
                def.name = assign_alias(&mut aliases, &def.name);
                def
            })
            .collect())
    }

    async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError> {
        let real_name = self.aliases.lock().unwrap().get(name).cloned().unwrap_or_else(|| name.to_string());
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": real_name, "arguments": args },
        });
        let resp = srelens_mcp::stdio::handle_request(&self.server, &req, srelens_mcp::Transport::Http)
            .await
            .ok_or_else(|| LlmError::Api("tools/call returned no response".into()))?;
        if let Some(err) = resp.get("error") {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("tool call failed");
            return Ok(ToolCallResult { content: msg.to_string(), is_error: true, denied: false });
        }
        let result = resp.get("result");
        let is_error = result.and_then(|r| r.get("isError")).and_then(Value::as_bool).unwrap_or(false);
        let denied = result
            .and_then(|r| r.get("_meta"))
            .and_then(|m| m.get("srelens/denied"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let content = result
            .and_then(|r| r.get("content"))
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok(ToolCallResult { content, is_error, denied })
    }
}

pub fn build_mcp_server(
    cache: Arc<ClientCache>,
    kubeconfig_paths: Vec<PathBuf>,
) -> Arc<McpServer> {
    let registry = srelens_registry::build_registry_with_paths(cache, kubeconfig_paths);
    let policy = Arc::new(srelens_mcp::policy::FlagGated::new(false, true));
    let server = McpServer::new(Arc::new(registry))
        .with_policy(policy)
        .with_kind_resolver(srelens_registry::kind_resolver());
    Arc::new(server)
}

pub async fn run_native_agent_turn(
    config: srelens_llm::ProviderConfig,
    invoker: Arc<McpToolInvoker>,
    history: Arc<tokio::sync::Mutex<Vec<Turn>>>,
    prompt: String,
    active_context: String,
    active_namespace: String,
    event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    let provider = srelens_llm::HttpProvider::new(config);
    let start_time = std::time::Instant::now();
    let out_chars = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let out_chars_clone = out_chars.clone();
    let event_tx_clone = event_tx.clone();

    let enriched_prompt = format!(
        "[Active Kubernetes Context: \"{}\", Namespace: \"{}\"]\n\n{}",
        active_context, active_namespace, prompt
    );

    let prior_turns = {
        let lock = history.lock().await;
        lock.clone()
    };

    let mut on_event = move |ev: AgentEvent| {
        match ev {
            AgentEvent::TextDelta { text } => {
                out_chars_clone.fetch_add(text.len(), std::sync::atomic::Ordering::Relaxed);
                let _ = event_tx_clone.send(AppEvent::ActionResult {
                    title: "ai_chunk".to_string(),
                    result: Ok(text),
                });
            }
            AgentEvent::Thinking { text } => {
                let _ = event_tx_clone.send(AppEvent::ActionResult {
                    title: "ai_status".to_string(),
                    result: Ok(text),
                });
            }
            AgentEvent::ToolCallStart { id, tool, args } => {
                let args_preview = if args.is_null() {
                    String::new()
                } else if let Some(s) = args.as_str() {
                    s.to_string()
                } else {
                    args.to_string()
                };
                let _ = event_tx_clone.send(AppEvent::ActionResult {
                    title: "ai_status".to_string(),
                    result: Ok(format!("Executing {}...", tool)),
                });
                let _ = event_tx_clone.send(AppEvent::ActionResult {
                    title: "ai_tool_start".to_string(),
                    result: Ok(format!("{}|{}|{}", id, tool, args_preview)),
                });
            }
            AgentEvent::ToolResult { id, status } => {
                let status_str = match status {
                    srelens_agent::event::ToolStatus::Ok => "ok",
                    srelens_agent::event::ToolStatus::Error => "error",
                    srelens_agent::event::ToolStatus::Denied => "denied",
                };
                let _ = event_tx_clone.send(AppEvent::ActionResult {
                    title: "ai_tool_done".to_string(),
                    result: Ok(format!("{}|{}", id, status_str)),
                });
            }
            AgentEvent::TurnDone => {}
            AgentEvent::Error { message } => {
                let _ = event_tx_clone.send(AppEvent::ActionResult {
                    title: "ai_chunk".to_string(),
                    result: Ok(format!("\n[Error: {}]", message)),
                });
            }
        }
    };

    let result = srelens_llm::agent_loop::run(
        &provider,
        invoker.as_ref(),
        prior_turns,
        enriched_prompt,
        &mut on_event,
    )
    .await;

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let prompt_est = (prompt.len() + 200) / 4;
    let comp_est = out_chars.load(std::sync::atomic::Ordering::Relaxed).max(1) / 4;
    let total_est = prompt_est + comp_est;
    let payload = format!("{}|{}|{}|{}|{}", prompt_est, comp_est, 0, total_est, duration_ms);
    let _ = event_tx.send(AppEvent::ActionResult {
        title: "ai_usage".to_string(),
        result: Ok(payload),
    });

    match result {
        Ok(updated_turns) => {
            {
                let mut lock = history.lock().await;
                *lock = updated_turns;
                if lock.len() > 40 {
                    let excess = lock.len() - 40;
                    lock.drain(0..excess);
                }
            }
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
        }
        Err(err) => {
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_chunk".to_string(),
                result: Err(format!("AI Agent Error: {}", err)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
        }
    }
}

pub async fn run_boxed_cursor_turn(
    cursor_bin: String,
    model: String,
    api_key: Option<String>,
    query: String,
    active_ctx: String,
    active_ns: String,
    cache: Arc<ClientCache>,
    kubeconfig_paths: Vec<PathBuf>,
    event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    let start_time = std::time::Instant::now();

    // 1. Bind ephemeral loopback port for MCP HTTP server
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => {
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_chunk".to_string(),
                result: Err(format!("Failed to bind local MCP port: {}", e)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
            return;
        }
    };
    let local_addr = match listener.local_addr() {
        Ok(a) => a,
        Err(e) => {
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_chunk".to_string(),
                result: Err(format!("Failed to get local MCP address: {}", e)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
            return;
        }
    };
    let url = format!("http://127.0.0.1:{}/mcp", local_addr.port());
    let token = srelens_mcp::auth::Token::generate();
    let token_str = token.as_str().to_string();

    // 2. Build MCP server
    let registry = srelens_registry::build_registry_with_paths(cache, kubeconfig_paths);
    let policy = Arc::new(srelens_mcp::policy::FlagGated::new(false, true));
    let server = McpServer::new(Arc::new(registry))
        .with_policy(policy)
        .with_kind_resolver(srelens_registry::kind_resolver());

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = srelens_mcp::http::serve_http_with_shutdown(
            server,
            listener,
            async {
                let _ = shutdown_rx.await;
            },
            token,
        )
        .await;
    });

    // 3. Create isolated temp directories for cursor-agent
    let temp_cfg = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            let _ = shutdown_tx.send(());
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_chunk".to_string(),
                result: Err(format!("Failed to create config dir: {}", e)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
            return;
        }
    };
    let temp_workspace = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            let _ = shutdown_tx.send(());
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_chunk".to_string(),
                result: Err(format!("Failed to create workspace dir: {}", e)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
            return;
        }
    };

    // Write cli-config.json to block shell/kubectl
    let cfg_path = temp_cfg.path().join("cli-config.json");
    if let Err(e) = std::fs::write(&cfg_path, srelens_agent::adapter::cursor_cli_config_json()) {
        let _ = shutdown_tx.send(());
        let _ = event_tx.send(AppEvent::ActionResult {
            title: "ai_chunk".to_string(),
            result: Err(format!("Failed to write cli-config.json: {}", e)),
        });
        let _ = event_tx.send(AppEvent::ActionResult {
            title: "ai_done".to_string(),
            result: Ok(String::new()),
        });
        return;
    }

    // Write .cursor/mcp.json to configure srelens MCP server
    let cursor_dir = temp_workspace.path().join(".cursor");
    if let Err(e) = std::fs::create_dir_all(&cursor_dir) {
        let _ = shutdown_tx.send(());
        let _ = event_tx.send(AppEvent::ActionResult {
            title: "ai_chunk".to_string(),
            result: Err(format!("Failed to create .cursor dir: {}", e)),
        });
        let _ = event_tx.send(AppEvent::ActionResult {
            title: "ai_done".to_string(),
            result: Ok(String::new()),
        });
        return;
    }
    if let Err(e) = std::fs::write(
        cursor_dir.join("mcp.json"),
        srelens_agent::adapter::cursor_mcp_json(&url, &token_str),
    ) {
        let _ = shutdown_tx.send(());
        let _ = event_tx.send(AppEvent::ActionResult {
            title: "ai_chunk".to_string(),
            result: Err(format!("Failed to write mcp.json: {}", e)),
        });
        let _ = event_tx.send(AppEvent::ActionResult {
            title: "ai_done".to_string(),
            result: Ok(String::new()),
        });
        return;
    }

    // 4. Construct cursor command with boxing flags
    let prompt_with_context = format!(
        "[Active Kubernetes Context: \"{}\", Namespace: \"{}\"]\n\n{}",
        active_ctx, active_ns, query
    );
    let cmd_spec = srelens_agent::adapter::cursor_command(
        &cursor_bin,
        &prompt_with_context,
        &temp_cfg.path().to_string_lossy(),
        &temp_workspace.path().to_string_lossy(),
        None,
    );

    let mut cmd = tokio::process::Command::new(&cmd_spec.program);
    cmd.args(&cmd_spec.args);
    for (k, v) in &cmd_spec.env {
        cmd.env(k, v);
    }
    cmd.current_dir(temp_workspace.path());
    if !model.is_empty() && model != "default" {
        cmd.arg("--model").arg(&model);
    }
    if let Some(key) = api_key {
        cmd.env("CURSOR_API_KEY", key);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // 5. Spawn and stream results
    match cmd.spawn() {
        Ok(mut child) => {
            if let Some(stdout) = child.stdout.take() {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let trimmed = line.trim();
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        if let Some(t) = v.get("type").and_then(|s| s.as_str()) {
                            if t == "thinking" {
                                let _ = event_tx.send(AppEvent::ActionResult {
                                    title: "ai_status".to_string(),
                                    result: Ok("Thinking & analyzing cluster query...".to_string()),
                                });
                            } else if t == "tool_call" {
                                let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
                                if subtype == "started" {
                                    if let Some((id, tool, args)) = crate::app::extract_tool_call_start_info(&v) {
                                        let _ = event_tx.send(AppEvent::ActionResult {
                                            title: "ai_status".to_string(),
                                            result: Ok(format!("Executing {} query on cluster...", tool)),
                                        });
                                        let _ = event_tx.send(AppEvent::ActionResult {
                                            title: "ai_tool_start".to_string(),
                                            result: Ok(format!("{}|{}|{}", id, tool, args)),
                                        });
                                    }
                                } else if subtype == "completed" {
                                    if let Some((id, is_err)) = crate::app::extract_tool_call_completed_info(&v) {
                                        let status_str = if is_err { "error" } else { "ok" };
                                        let _ = event_tx.send(AppEvent::ActionResult {
                                            title: "ai_tool_done".to_string(),
                                            result: Ok(format!("{}|{}", id, status_str)),
                                        });
                                    }
                                }
                            } else if t == "result" {
                                if let Some((prompt, comp, cached, total, dur)) = crate::app::extract_usage_metrics(&v) {
                                    let dur_val = dur.unwrap_or_else(|| start_time.elapsed().as_millis() as u64);
                                    let payload = format!("{}|{}|{}|{}|{}", prompt, comp, cached, total, dur_val);
                                    let _ = event_tx.send(AppEvent::ActionResult {
                                        title: "ai_usage".to_string(),
                                        result: Ok(payload),
                                    });
                                }
                            }
                        }
                    }

                    let events = srelens_agent::cursor::parse_line(&line);
                    for ev in events {
                        match ev {
                            srelens_agent::event::AgentEvent::TextDelta { text } => {
                                let _ = event_tx.send(AppEvent::ActionResult {
                                    title: "ai_chunk".to_string(),
                                    result: Ok(text),
                                });
                            }
                            srelens_agent::event::AgentEvent::Error { message } => {
                                let _ = event_tx.send(AppEvent::ActionResult {
                                    title: "ai_chunk".to_string(),
                                    result: Ok(format!("\n[Error: {}]", message)),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            let _ = child.wait().await;
            let _ = shutdown_tx.send(());
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
        }
        Err(err) => {
            let _ = shutdown_tx.send(());
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_chunk".to_string(),
                result: Err(format!("Failed to launch cursor-agent: {}", err)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: "ai_done".to_string(),
                result: Ok(String::new()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_tool_invoker_lists_and_maps_aliases() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let server = build_mcp_server(cache, vec![]);
        let invoker = McpToolInvoker::new(server);

        let tools = invoker.list_tools().await.expect("list_tools succeeds");
        assert!(!tools.is_empty(), "tools list should contain capabilities");

        // Check provider-safe naming
        for t in &tools {
            assert!(
                t.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "tool name '{}' must be provider safe",
                t.name
            );
        }

        // Must include basic core k8s capabilities
        let has_list_pods = tools.iter().any(|t| t.name.contains("listPods"));
        let has_list_ns = tools.iter().any(|t| t.name.contains("listNamespaces"));
        assert!(has_list_pods, "should include listPods capability");
        assert!(has_list_ns, "should include listNamespaces capability");
    }
}
