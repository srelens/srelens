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
