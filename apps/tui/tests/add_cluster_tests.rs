use srelens_tui::app::App;
use srelens_tui::event::AppEvent;
use srelens_tui::ui::dialogs::Modal;
use tokio::sync::mpsc::unbounded_channel;

mod common;

const VALID_KUBECONFIG_YAML: &str = r#"
apiVersion: v1
kind: Config
current-context: imported-test-ctx
clusters:
- name: imported-test-cluster
  cluster:
    server: https://127.0.0.1:6443
    insecure-skip-tls-verify: true
contexts:
- name: imported-test-ctx
  context:
    cluster: imported-test-cluster
    user: imported-test-user
users:
- name: imported-test-user
  user:
    token: fake-token-123
"#;

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct EnvVarGuard {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(name);
        std::env::set_var(name, value);
        Self { name, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}

#[tokio::test]
async fn test_import_kubeconfig_from_yaml_content() {
    let _lock = ENV_LOCK.lock().await;
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
    let managed_dir = temp.path().join("managed_configs");
    std::fs::create_dir_all(&managed_dir).unwrap();
    let _guard = EnvVarGuard::set("SRELENS_KUBECONFIG_DIR", &managed_dir);

    let initial_config = temp.path().join("config");
    std::fs::write(&initial_config, VALID_KUBECONFIG_YAML).unwrap();

    let mut app = App::new(None, None, false, None, vec![initial_config.clone()], tx)
        .await
        .unwrap();

    let new_yaml = r#"
apiVersion: v1
kind: Config
current-context: secondary-cluster-ctx
clusters:
- name: secondary-cluster
  cluster:
    server: https://10.0.0.1:6443
    insecure-skip-tls-verify: true
contexts:
- name: secondary-cluster-ctx
  context:
    cluster: secondary-cluster
    user: secondary-user
users:
- name: secondary-user
  user:
    token: sec-token-456
"#;

    let imported_ctx = app.import_kubeconfig(new_yaml).await.unwrap();
    assert_eq!(imported_ctx, "secondary-cluster-ctx");
    app.switch_context(imported_ctx.clone()).await;
    assert_eq!(app.active_context, "secondary-cluster-ctx");
    assert!(app
        .contexts
        .iter()
        .any(|c| c.name == "secondary-cluster-ctx"));

    // Verify it was persisted to the isolated managed directory
    let saved_files = srelens_kube::connect::kubeconfig_files_in(&managed_dir);
    assert_eq!(saved_files.len(), 1);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_perms = std::fs::metadata(&managed_dir).unwrap().permissions();
        assert_eq!(
            dir_perms.mode() & 0o777,
            0o700,
            "managed directory must have 0700 permissions"
        );
        let perms = std::fs::metadata(&saved_files[0]).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "file must have 0600 permissions"
        );
    }
}

#[tokio::test]
async fn test_import_kubeconfig_from_file_path() {
    let _lock = ENV_LOCK.lock().await;
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
    let managed_dir = temp.path().join("managed_configs");
    std::fs::create_dir_all(&managed_dir).unwrap();
    let _guard = EnvVarGuard::set("SRELENS_KUBECONFIG_DIR", &managed_dir);

    let initial_config = temp.path().join("config");
    std::fs::write(&initial_config, VALID_KUBECONFIG_YAML).unwrap();

    let mut app = App::new(None, None, false, None, vec![initial_config.clone()], tx)
        .await
        .unwrap();

    let external_yaml_path = temp.path().join("external-cluster.yaml");
    let external_yaml = r#"
apiVersion: v1
kind: Config
current-context: file-imported-ctx
clusters:
- name: file-imported-cluster
  cluster:
    server: https://192.168.1.100:6443
    insecure-skip-tls-verify: true
contexts:
- name: file-imported-ctx
  context:
    cluster: file-imported-cluster
    user: file-user
users:
- name: file-user
  user:
    token: file-token-789
"#;
    std::fs::write(&external_yaml_path, external_yaml).unwrap();

    let imported_ctx = app
        .import_kubeconfig(&external_yaml_path.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(imported_ctx, "file-imported-ctx");
    app.switch_context(imported_ctx.clone()).await;
    assert_eq!(app.active_context, "file-imported-ctx");
    assert!(app.contexts.iter().any(|c| c.name == "file-imported-ctx"));

    // Verify it was copied into managed dir for persistence across restarts
    let saved_files = srelens_kube::connect::kubeconfig_files_in(&managed_dir);
    assert_eq!(saved_files.len(), 1);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_perms = std::fs::metadata(&managed_dir).unwrap().permissions();
        assert_eq!(
            dir_perms.mode() & 0o777,
            0o700,
            "managed directory must have 0700 permissions"
        );
        let perms = std::fs::metadata(&saved_files[0]).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "file must have 0600 permissions"
        );
    }
}

#[tokio::test]
async fn test_add_cluster_modal_bracketed_paste() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
    let initial_config = temp.path().join("config");
    std::fs::write(&initial_config, VALID_KUBECONFIG_YAML).unwrap();

    let mut app = App::new(None, None, false, None, vec![initial_config.clone()], tx)
        .await
        .unwrap();

    app.modal = Some(Modal::AddCluster {
        input: String::new(),
        cursor_pos: 0,
        error_message: None,
        preview_contexts: vec![],
    });

    let pasted_yaml = "apiVersion: v1\r\nkind: Config\r\nclusters:\r\n- name: c\r\n  cluster: {server: 'https://127.0.0.1:1'}\r\ncontexts:\r\n- name: pasted-ctx\r\n  context: {cluster: c, user: u}\r\nusers:\r\n- name: u\r\n  user: {}\r\n";
    app.handle_paste(pasted_yaml.to_string());

    if let Some(Modal::AddCluster {
        input,
        preview_contexts,
        error_message,
        ..
    }) = &app.modal
    {
        assert!(!input.contains("\r\n"), "CRLF should be converted to LF");
        assert_eq!(preview_contexts, &vec!["pasted-ctx".to_string()]);
        assert!(error_message.is_none());
    } else {
        panic!("expected Modal::AddCluster to be open");
    }
}

#[tokio::test]
async fn test_add_cluster_modal_oversized_paste() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
    let initial_config = temp.path().join("config");
    std::fs::write(&initial_config, VALID_KUBECONFIG_YAML).unwrap();

    let mut app = App::new(None, None, false, None, vec![initial_config.clone()], tx)
        .await
        .unwrap();

    let initial_input = "apiVersion: v1".to_string();
    app.modal = Some(Modal::AddCluster {
        input: initial_input.clone(),
        cursor_pos: initial_input.len(),
        error_message: None,
        preview_contexts: vec![],
    });

    let huge_content = "x".repeat(1024 * 1024 + 10);
    app.handle_paste(huge_content);

    if let Some(Modal::AddCluster {
        input,
        error_message,
        ..
    }) = &app.modal
    {
        assert_eq!(input, &initial_input, "input must remain unchanged");
        assert_eq!(
            error_message.as_deref(),
            Some("Pasted content exceeds 1 MB limit")
        );
    } else {
        panic!("expected Modal::AddCluster to be open");
    }
}

#[test]
fn test_render_add_cluster_modal() {
    let modal = Modal::AddCluster {
        input: "/path/to/my-cluster.yaml".to_string(),
        cursor_pos: 24,
        error_message: None,
        preview_contexts: vec!["my-cluster-ctx".to_string()],
    };

    let rendered = common::render_text(100, 30, |f| {
        srelens_tui::ui::dialogs::render_modal(f, f.area(), &modal);
    });

    assert!(rendered.contains("Import Cluster / Kubeconfig"));
    assert!(rendered.contains("/path/to/my-cluster.yaml"));
    assert!(rendered.contains("my-cluster-ctx"));
    assert!(rendered.contains("Import & Connect"));
}

#[test]
fn test_render_add_cluster_modal_with_error() {
    let modal = Modal::AddCluster {
        input: "invalid: yaml: content: [".to_string(),
        cursor_pos: 25,
        error_message: Some("Invalid YAML: missing closing bracket".to_string()),
        preview_contexts: vec![],
    };

    let rendered = common::render_text(100, 30, |f| {
        srelens_tui::ui::dialogs::render_modal(f, f.area(), &modal);
    });

    assert!(rendered.contains("Import Cluster / Kubeconfig"));
    assert!(rendered.contains("Invalid YAML: missing closing bracket"));
}

#[test]
fn test_resolve_import_command_and_aliases() {
    let cases = [
        ":import",
        "import",
        ":add-cluster",
        "add-cluster",
        ":import-kubeconfig",
        "import-kubeconfig",
        ":add-ctx",
        "add-ctx",
        ":kubeconfig-add",
        "kubeconfig-add",
    ];

    for cmd in cases {
        assert_eq!(
            srelens_tui::commands::resolve_command(cmd),
            Some(srelens_tui::commands::CommandTarget::AddCluster),
            "command '{}' must resolve to CommandTarget::AddCluster",
            cmd
        );
    }
}

#[test]
fn test_import_deep_link_parse_and_to_url() {
    use srelens_tui::commands::CommandTarget;
    use srelens_tui::deep_link::DeepLink;

    // 1. Parsing standard view deep link
    let parsed = DeepLink::parse("srelens://view/_/_/import").unwrap();
    assert_eq!(
        parsed,
        DeepLink::View {
            context: None,
            namespace: None,
            target: CommandTarget::AddCluster,
        }
    );

    // 2. Canonical serialization via to_url
    assert_eq!(parsed.to_url(), "srelens://view/_/_/import");

    // 3. Parsing with context & namespace
    let parsed_ctx_ns = DeepLink::parse("srelens://view/prod/kube-system/add-cluster").unwrap();
    assert_eq!(
        parsed_ctx_ns,
        DeepLink::View {
            context: Some("prod".to_string()),
            namespace: Some("kube-system".to_string()),
            target: CommandTarget::AddCluster,
        }
    );

    // 4. Parsing all aliases via deep link
    for alias in &[
        "add-cluster",
        "import-kubeconfig",
        "add-ctx",
        "kubeconfig-add",
    ] {
        let link = format!("srelens://view/_/_/{}", alias);
        let res = DeepLink::parse(&link).unwrap();
        assert_eq!(
            res,
            DeepLink::View {
                context: None,
                namespace: None,
                target: CommandTarget::AddCluster,
            },
            "deep link '{}' must resolve to AddCluster",
            link
        );
    }

    // 5. Direct view shorthand parse
    let direct = DeepLink::parse("import").unwrap();
    assert_eq!(
        direct,
        DeepLink::View {
            context: None,
            namespace: None,
            target: CommandTarget::AddCluster,
        }
    );
}

#[tokio::test]
async fn test_execute_colon_command_import_and_aliases() {
    let (mut app, _rx) = common::app().await;

    // Execute :import
    app.execute_colon_command("import").await;
    assert!(
        matches!(app.modal, Some(Modal::AddCluster { .. })),
        "execute_colon_command('import') must open Modal::AddCluster"
    );

    // Close modal
    app.modal = None;

    // Execute :add-cluster with leading colon
    app.execute_colon_command(":add-cluster").await;
    assert!(
        matches!(app.modal, Some(Modal::AddCluster { .. })),
        "execute_colon_command(':add-cluster') must open Modal::AddCluster"
    );

    // Close modal
    app.modal = None;

    // Execute :add-ctx
    app.execute_colon_command("add-ctx").await;
    assert!(
        matches!(app.modal, Some(Modal::AddCluster { .. })),
        "execute_colon_command('add-ctx') must open Modal::AddCluster"
    );
}

#[tokio::test]
async fn test_context_picker_shortcuts_open_add_cluster_modal() {
    let (mut app, _rx) = common::app().await;

    // Open context picker
    app.open_context_picker();
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));

    // Press Ctrl+i -> should open AddCluster modal
    app.handle_key_event(common::ctrl('i')).await;
    assert!(
        matches!(app.modal, Some(Modal::AddCluster { .. })),
        "Ctrl+i in ContextPicker must open Modal::AddCluster"
    );

    // Reset back to context picker
    app.open_context_picker();
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));

    // Press Ctrl+a -> should also open AddCluster modal
    app.handle_key_event(common::ctrl('a')).await;
    assert!(
        matches!(app.modal, Some(Modal::AddCluster { .. })),
        "Ctrl+a in ContextPicker must open Modal::AddCluster"
    );
}

#[tokio::test]
async fn test_add_cluster_modal_key_editing_and_navigation() {
    let (mut app, _rx) = common::app().await;

    // Open modal directly
    app.open_add_cluster_modal();
    assert!(matches!(app.modal, Some(Modal::AddCluster { .. })));

    // Type "apiVersion: v1"
    common::type_str(&mut app, "apiVersion: v1").await;
    if let Some(Modal::AddCluster {
        input, cursor_pos, ..
    }) = &app.modal
    {
        assert_eq!(input, "apiVersion: v1");
        assert_eq!(*cursor_pos, 14);
    } else {
        panic!("expected Modal::AddCluster");
    }

    // Left arrow moves cursor
    app.handle_key_event(common::key(crossterm::event::KeyCode::Left))
        .await;
    if let Some(Modal::AddCluster { cursor_pos, .. }) = &app.modal {
        assert_eq!(*cursor_pos, 13);
    }

    // Home moves cursor to 0
    app.handle_key_event(common::key(crossterm::event::KeyCode::Home))
        .await;
    if let Some(Modal::AddCluster { cursor_pos, .. }) = &app.modal {
        assert_eq!(*cursor_pos, 0);
    }

    // End moves cursor to end
    app.handle_key_event(common::key(crossterm::event::KeyCode::End))
        .await;
    if let Some(Modal::AddCluster { cursor_pos, .. }) = &app.modal {
        assert_eq!(*cursor_pos, 14);
    }

    // Backspace removes last character
    app.handle_key_event(common::key(crossterm::event::KeyCode::Backspace))
        .await;
    if let Some(Modal::AddCluster {
        input, cursor_pos, ..
    }) = &app.modal
    {
        assert_eq!(input, "apiVersion: v");
        assert_eq!(*cursor_pos, 13);
    }

    // Ctrl+u clears input
    app.handle_key_event(common::ctrl('u')).await;
    if let Some(Modal::AddCluster {
        input,
        cursor_pos,
        preview_contexts,
        ..
    }) = &app.modal
    {
        assert_eq!(input, "");
        assert_eq!(*cursor_pos, 0);
        assert!(preview_contexts.is_empty());
    }

    // Esc closes modal
    app.handle_key_event(common::key(crossterm::event::KeyCode::Esc))
        .await;
    assert!(app.modal.is_none(), "Esc must close Modal::AddCluster");
}

#[tokio::test]
async fn test_add_cluster_modal_submit_flow() {
    let _lock = ENV_LOCK.lock().await;
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
    let managed_dir = temp.path().join("managed_configs");
    std::fs::create_dir_all(&managed_dir).unwrap();
    let _guard = EnvVarGuard::set("SRELENS_KUBECONFIG_DIR", &managed_dir);

    let initial_config = temp.path().join("config");
    std::fs::write(&initial_config, VALID_KUBECONFIG_YAML).unwrap();

    let mut app = App::new(None, None, false, None, vec![initial_config.clone()], tx)
        .await
        .unwrap();

    // 1. Submit invalid input -> sets error_message
    app.open_add_cluster_modal();
    common::type_str(&mut app, "not valid yaml content").await;
    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter))
        .await;
    if let Some(Modal::AddCluster { error_message, .. }) = &app.modal {
        assert!(
            error_message.is_some(),
            "submitting invalid input should produce error"
        );
    } else {
        panic!("expected modal to stay open with error");
    }

    // 2. Submit valid YAML -> closes modal and switches context
    let valid_yaml = r#"
apiVersion: v1
kind: Config
current-context: submit-test-ctx
clusters:
- name: submit-test-cluster
  cluster:
    server: https://10.99.0.1:6443
    insecure-skip-tls-verify: true
contexts:
- name: submit-test-ctx
  context:
    cluster: submit-test-cluster
    user: submit-user
users:
- name: submit-user
  user:
    token: tok-999
"#;
    app.modal = Some(Modal::AddCluster {
        input: valid_yaml.to_string(),
        cursor_pos: valid_yaml.len(),
        error_message: None,
        preview_contexts: vec!["submit-test-ctx".to_string()],
    });

    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter))
        .await;
    assert!(app.modal.is_none(), "successful import must close modal");
    assert_eq!(app.active_context, "submit-test-ctx");
}

#[tokio::test]
async fn test_same_context_import_refreshes_metadata_and_reconnects() {
    let _lock = ENV_LOCK.lock().await;
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
    let managed_dir = temp.path().join("managed_configs");
    std::fs::create_dir_all(&managed_dir).unwrap();
    let _guard = EnvVarGuard::set("SRELENS_KUBECONFIG_DIR", &managed_dir);

    let initial_config = temp.path().join("config");
    std::fs::write(&initial_config, VALID_KUBECONFIG_YAML).unwrap();

    let mut app = App::new(None, None, false, None, vec![initial_config.clone()], tx)
        .await
        .unwrap();

    // 1. Initial import of secondary-cluster
    let initial_yaml = r#"
apiVersion: v1
kind: Config
current-context: same-ctx-test
clusters:
- name: initial-cluster
  cluster:
    server: https://10.100.0.1:6443
    insecure-skip-tls-verify: true
contexts:
- name: same-ctx-test
  context:
    cluster: initial-cluster
    user: initial-user
    namespace: default
users:
- name: initial-user
  user:
    token: fake-tok-1
"#;
    let imported_ctx = app.import_kubeconfig(initial_yaml).await.unwrap();
    app.switch_context_forced(imported_ctx.clone()).await;
    assert_eq!(app.active_context, imported_ctx);
    assert_eq!(app.cluster_name, "initial-cluster");
    assert_eq!(app.server_url, "https://10.100.0.1:6443");
    assert_eq!(app.active_namespace, "default");

    // Populate resource cache and set connected
    app.resource_cache.insert(
        (
            imported_ctx.clone(),
            "default".to_string(),
            "pods".to_string(),
        ),
        vec![serde_json::json!({"kind": "Pod", "metadata": {"name": "test-pod"}})],
    );
    app.is_connected = true;
    assert!(!app.resource_cache.is_empty());

    // 2. Re-import / update with the SAME active context via Modal::AddCluster Enter key event
    let updated_yaml = r#"
apiVersion: v1
kind: Config
current-context: same-ctx-test
clusters:
- name: updated-cluster
  cluster:
    server: https://10.200.0.1:6443
    insecure-skip-tls-verify: true
contexts:
- name: same-ctx-test
  context:
    cluster: updated-cluster
    user: updated-user
    namespace: custom-ns
users:
- name: updated-user
  user:
    token: fake-tok-2
"#;

    app.modal = Some(Modal::AddCluster {
        input: updated_yaml.to_string(),
        cursor_pos: updated_yaml.len(),
        error_message: None,
        preview_contexts: vec!["same-ctx-test".to_string()],
    });

    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter))
        .await;

    assert!(app.modal.is_none(), "modal should close after re-import");
    assert_eq!(
        app.cluster_name, "updated-cluster",
        "cluster_name must be refreshed"
    );
    assert_eq!(
        app.server_url, "https://10.200.0.1:6443",
        "server_url must be refreshed"
    );
    assert_eq!(
        app.active_namespace, "custom-ns",
        "active_namespace must be refreshed"
    );
    assert!(
        app.resource_cache.is_empty(),
        "resource_cache must be cleared on same-context re-import"
    );
    assert_eq!(
        app.cluster_version, "Connecting...",
        "connection state must be reset to connecting"
    );
    assert!(!app.is_connected);
}
