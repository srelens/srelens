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

#[tokio::test]
async fn test_import_kubeconfig_from_yaml_content() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
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

    let imported_ctx = app.import_kubeconfig(new_yaml).unwrap();
    assert_eq!(imported_ctx, "secondary-cluster-ctx");
    assert_eq!(app.active_context, "secondary-cluster-ctx");
    assert!(app
        .contexts
        .iter()
        .any(|c| c.name == "secondary-cluster-ctx"));
}

#[tokio::test]
async fn test_import_kubeconfig_from_file_path() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let temp = tempfile::tempdir().unwrap();
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
        .unwrap();
    assert_eq!(imported_ctx, "file-imported-ctx");
    assert_eq!(app.active_context, "file-imported-ctx");
    assert!(app.contexts.iter().any(|c| c.name == "file-imported-ctx"));
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
