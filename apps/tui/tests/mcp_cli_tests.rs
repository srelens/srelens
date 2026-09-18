use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use clap::Parser;
use srelens_tui::mcp_server::{build_stdio_mcp_server, run_mcp_stdio};

fn init_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn mcp_stdio_initialize_and_tools_list() {
    init_crypto_provider();
    let server = build_stdio_mcp_server(vec![], false, true);

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        let _ = run_mcp_stdio(server, server_read, server_write).await;
    });

    let mut lines = tokio::io::BufReader::new(client_read);

    // 1. Initialize
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "test-agent", "version": "1.0.0" }
        }
    });
    client_write
        .write_all(format!("{}\n", init_req).as_bytes())
        .await
        .unwrap();

    use tokio::io::AsyncBufReadExt;
    let mut resp_line = String::new();
    lines.read_line(&mut resp_line).await.unwrap();
    let resp: Value = serde_json::from_str(&resp_line).expect("valid json response");
    assert_eq!(resp.get("id").and_then(Value::as_i64), Some(1));
    let result = resp.get("result").expect("result object");
    assert!(result.get("serverInfo").is_some());

    // 2. tools/list
    let list_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    client_write
        .write_all(format!("{}\n", list_req).as_bytes())
        .await
        .unwrap();

    resp_line.clear();
    lines.read_line(&mut resp_line).await.unwrap();
    let list_resp: Value = serde_json::from_str(&resp_line).expect("valid json response");
    assert_eq!(list_resp.get("id").and_then(Value::as_i64), Some(2));
    let tools = list_resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
        .expect("tools array");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .collect();

    assert!(names.contains(&"ping"), "expected ping tool, got {names:?}");
    assert!(
        names.contains(&"k8s.listPods"),
        "expected k8s.listPods tool, got {names:?}"
    );
    assert!(
        names.contains(&"k8s.listNodes"),
        "expected k8s.listNodes tool, got {names:?}"
    );
    assert!(
        names.contains(&"k8s.helmInstall"),
        "expected k8s.helmInstall tool, got {names:?}"
    );
    assert!(
        names.contains(&"k8s.helmRollback"),
        "expected k8s.helmRollback tool, got {names:?}"
    );
    assert!(
        names.contains(&"k8s.helmUninstall"),
        "expected k8s.helmUninstall tool, got {names:?}"
    );

    drop(client_write);
    let _ = server_task.await;
}

#[tokio::test]
async fn mcp_stdio_read_only_tool_ping() {
    init_crypto_provider();
    let server = build_stdio_mcp_server(vec![], false, true);

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        let _ = run_mcp_stdio(server, server_read, server_write).await;
    });

    let mut lines = tokio::io::BufReader::new(client_read);

    // Call ping tool
    let ping_req = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "ping",
            "arguments": { "test": "echo" }
        }
    });
    client_write
        .write_all(format!("{}\n", ping_req).as_bytes())
        .await
        .unwrap();

    use tokio::io::AsyncBufReadExt;
    let mut resp_line = String::new();
    lines.read_line(&mut resp_line).await.unwrap();
    let resp: Value = serde_json::from_str(&resp_line).expect("valid json response");
    assert_eq!(resp.get("id").and_then(Value::as_i64), Some(10));
    let text = resp
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .expect("text content in tool result");
    assert!(
        text.contains("echo"),
        "expected ping to echo back argument, got: {text}"
    );

    drop(client_write);
    let _ = server_task.await;
}

#[tokio::test]
async fn mcp_stdio_destructive_policy_denies_without_flag() {
    init_crypto_provider();
    // allow_destructive = false
    let server = build_stdio_mcp_server(vec![], false, false);

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        let _ = run_mcp_stdio(server, server_read, server_write).await;
    });

    let mut lines = tokio::io::BufReader::new(client_read);

    // Attempt to call k8s.helmUninstall without flag
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "tools/call",
        "params": {
            "name": "k8s.helmUninstall",
            "arguments": { "context": "ctx", "name": "my-release" }
        }
    });
    client_write
        .write_all(format!("{}\n", call_req).as_bytes())
        .await
        .unwrap();

    use tokio::io::AsyncBufReadExt;
    let mut resp_line = String::new();
    lines.read_line(&mut resp_line).await.unwrap();
    let resp: Value = serde_json::from_str(&resp_line).expect("valid json response");
    assert_eq!(resp.get("id").and_then(Value::as_i64), Some(20));
    let text = resp
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        text.contains("consent denied") || text.contains("mutates the cluster"),
        "expected consent denial for destructive tool without flag, got: {text}"
    );

    drop(client_write);
    let _ = server_task.await;
}

#[tokio::test]
async fn mcp_stdio_destructive_policy_allows_with_flag_and_confirm() {
    init_crypto_provider();
    // allow_destructive = true
    let server = build_stdio_mcp_server(vec![], true, false);

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        let _ = run_mcp_stdio(server, server_read, server_write).await;
    });

    let mut lines = tokio::io::BufReader::new(client_read);

    // Call k8s.helmUninstall with _confirm: true
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "tools/call",
        "params": {
            "name": "k8s.helmUninstall",
            "arguments": { "context": "nonexistent-context", "name": "my-release", "_confirm": true }
        }
    });
    client_write
        .write_all(format!("{}\n", call_req).as_bytes())
        .await
        .unwrap();

    use tokio::io::AsyncBufReadExt;
    let mut resp_line = String::new();
    lines.read_line(&mut resp_line).await.unwrap();
    let resp: Value = serde_json::from_str(&resp_line).expect("valid json response");
    assert_eq!(resp.get("id").and_then(Value::as_i64), Some(30));
    // The policy permitted the call; the result will report handler execution (e.g. context error) rather than "consent denied".
    let text = resp
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        !text.contains("consent denied"),
        "expected call to pass consent gate, got: {text}"
    );

    drop(client_write);
    let _ = server_task.await;
}

#[test]
fn cli_parses_mcp_subcommand_and_flags() {
    // 1. Subcommand default
    let cli =
        srelens_tui::Cli::try_parse_from(["srelens-tui", "mcp"]).expect("parse mcp subcommand");
    match cli.command {
        Some(srelens_tui::CliCommand::Mcp {
            allow_destructive,
            allow_sensitive_reads,
        }) => {
            assert!(!allow_destructive);
            assert!(!allow_sensitive_reads);
        }
        other => panic!("expected Mcp command, got {other:?}"),
    }

    // 2. Subcommand with flags
    let cli = srelens_tui::Cli::try_parse_from([
        "srelens-tui",
        "mcp",
        "--allow-destructive",
        "--allow-sensitive-reads",
    ])
    .expect("parse mcp subcommand with flags");
    match cli.command {
        Some(srelens_tui::CliCommand::Mcp {
            allow_destructive,
            allow_sensitive_reads,
        }) => {
            assert!(allow_destructive);
            assert!(allow_sensitive_reads);
        }
        other => panic!("expected Mcp command, got {other:?}"),
    }

    // 3. Top-level flags
    let cli = srelens_tui::Cli::try_parse_from([
        "srelens-tui",
        "--mcp-stdio",
        "--mcp-allow-destructive",
        "--mcp-allow-sensitive-reads",
    ])
    .expect("parse top-level flags");
    assert!(cli.mcp_stdio);
    assert!(cli.mcp_allow_destructive);
    assert!(cli.mcp_allow_sensitive_reads);
    assert!(cli.command.is_none());
}
