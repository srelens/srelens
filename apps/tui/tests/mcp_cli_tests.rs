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
    // Verify downstream handler execution was reached (fails on nonexistent context rather than policy denial)
    assert!(
        text.contains("context")
            || text.contains("Unknown")
            || text.contains("not found")
            || text.contains("error")
            || text.contains("failed"),
        "expected handler-level failure for nonexistent-context, got: {text}"
    );

    drop(client_write);
    let _ = server_task.await;
}

#[tokio::test]
async fn mcp_stdio_sensitive_reads_policy_denies_without_flag() {
    init_crypto_provider();
    // allow_sensitive_reads = false
    let server = build_stdio_mcp_server(vec![], false, false);

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        let _ = run_mcp_stdio(server, server_read, server_write).await;
    });

    let mut lines = tokio::io::BufReader::new(client_read);

    // Call k8s.getSecret with _confirm: true but without allow_sensitive_reads flag
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "tools/call",
        "params": {
            "name": "k8s.getSecret",
            "arguments": { "context": "ctx", "namespace": "default", "name": "my-secret", "_confirm": true }
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
    assert_eq!(resp.get("id").and_then(Value::as_i64), Some(40));
    let text = resp
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        text.contains("consent denied") || text.contains("sensitive"),
        "expected consent denial for sensitive read tool without flag, got: {text}"
    );

    drop(client_write);
    let _ = server_task.await;
}

#[tokio::test]
async fn mcp_stdio_sensitive_reads_policy_allows_with_flag() {
    init_crypto_provider();
    // allow_sensitive_reads = true
    let server = build_stdio_mcp_server(vec![], false, true);

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        let _ = run_mcp_stdio(server, server_read, server_write).await;
    });

    let mut lines = tokio::io::BufReader::new(client_read);

    // Call k8s.getSecret with flag enabled and without _confirm
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 41,
        "method": "tools/call",
        "params": {
            "name": "k8s.getSecret",
            "arguments": { "context": "nonexistent-context", "namespace": "default", "name": "my-secret" }
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
    assert_eq!(resp.get("id").and_then(Value::as_i64), Some(41));
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
        "expected call to pass consent gate with flag, got: {text}"
    );
    assert!(
        text.contains("context")
            || text.contains("Unknown")
            || text.contains("not found")
            || text.contains("error")
            || text.contains("failed"),
        "expected handler-level failure for nonexistent-context, got: {text}"
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

/// An empty kubeconfig and an empty managed folder, so a startup test sees no
/// contexts whoever runs it.
///
/// Both MCP entry points resolve `all_kubeconfig_paths()` before they build
/// the server, which reads `KUBECONFIG`, then the developer's `~/.kube`, then
/// the app's own kubeconfig folder. Left ambient, these tests asked the
/// machine's real clusters about themselves: they passed or failed on whose
/// laptop they ran, and a context that needed an exec credential could make
/// startup wait on a login. The directory is returned so it outlives the
/// child.
fn isolated_kubeconfig() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir for the isolated kubeconfig");
    let config = dir.path().join("kubeconfig");
    std::fs::write(
        &config,
        "apiVersion: v1\nkind: Config\nclusters: []\ncontexts: []\nusers: []\n",
    )
    .expect("write the empty kubeconfig");
    let managed = dir.path().join("managed");
    std::fs::create_dir_all(&managed).expect("an empty managed kubeconfig folder");
    (dir, config)
}

async fn run_binary_mcp_init(args: &[&str]) -> serde_json::Value {
    let binary_path = env!("CARGO_BIN_EXE_srelens-tui");
    let (kubeconfig_dir, kubeconfig) = isolated_kubeconfig();
    let mut child = tokio::process::Command::new(binary_path)
        .args(args)
        // See `isolated_kubeconfig`: KUBECONFIG short-circuits the home-directory
        // search, and SRELENS_KUBECONFIG_DIR the app's own folder. Without both,
        // one of the two sources is still the developer's.
        .env("KUBECONFIG", &kubeconfig)
        .env(
            "SRELENS_KUBECONFIG_DIR",
            kubeconfig_dir.path().join("managed"),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary with {args:?}: {e}"));

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-bin", "version": "1.0.0" }
        }
    });

    let mut stdin = child.stdin.take().expect("child stdin");
    tokio::io::AsyncWriteExt::write_all(&mut stdin, format!("{}\n", init_req).as_bytes())
        .await
        .expect("write initialize");

    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut response_line = String::new();

    let read_res = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut response_line),
    )
    .await;

    let _ = child.kill().await;
    let _ = child.wait().await;

    match read_res {
        Ok(Ok(n)) if n > 0 => {}
        Ok(Ok(_)) => panic!("child stdout closed unexpectedly for args {args:?}"),
        Ok(Err(e)) => panic!("read error from child for args {args:?}: {e}"),
        Err(_) => panic!("timed out waiting for MCP initialize response for args {args:?}"),
    }

    serde_json::from_str(&response_line).expect("valid JSON response")
}

#[tokio::test]
async fn mcp_binary_subcommand_startup_initializes() {
    let resp = run_binary_mcp_init(&["mcp"]).await;
    assert_eq!(resp.get("id").and_then(serde_json::Value::as_i64), Some(1));
    let server_name = resp
        .get("result")
        .and_then(|r| r.get("serverInfo"))
        .and_then(|info| info.get("name"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(server_name, Some("srelens"));
}

#[tokio::test]
async fn mcp_binary_flag_startup_initializes() {
    let resp = run_binary_mcp_init(&["--mcp-stdio"]).await;
    assert_eq!(resp.get("id").and_then(serde_json::Value::as_i64), Some(1));
    let server_name = resp
        .get("result")
        .and_then(|r| r.get("serverInfo"))
        .and_then(|info| info.get("name"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(server_name, Some("srelens"));
}
