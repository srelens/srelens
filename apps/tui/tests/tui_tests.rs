#[cfg(test)]
mod tests {
    use srelens_tui::commands::{command_suggestions, resolve_command, CommandTarget, ResourceKind};
    use srelens_tui::views::ResourceTableState;
    use serde_json::json;

    #[test]
    fn test_resolve_command_aliases() {
        assert!(matches!(
            resolve_command(":po"),
            Some(CommandTarget::Resource(ResourceKind::Pods))
        ));
        assert!(matches!(
            resolve_command("pod"),
            Some(CommandTarget::Resource(ResourceKind::Pods))
        ));
        assert!(matches!(
            resolve_command(":deploy"),
            Some(CommandTarget::Resource(ResourceKind::Deployments))
        ));
        assert!(matches!(
            resolve_command(":svc"),
            Some(CommandTarget::Resource(ResourceKind::Services))
        ));
        assert!(matches!(
            resolve_command(":no"),
            Some(CommandTarget::Resource(ResourceKind::Nodes))
        ));
        assert!(matches!(
            resolve_command(":ns"),
            Some(CommandTarget::Namespaces)
        ));
        assert!(matches!(
            resolve_command(":helm"),
            Some(CommandTarget::Resource(ResourceKind::HelmReleases))
        ));
        assert!(matches!(
            resolve_command(":pf"),
            Some(CommandTarget::Resource(ResourceKind::PortForwards))
        ));
        assert!(matches!(
            resolve_command(":ctx"),
            Some(CommandTarget::Contexts)
        ));
        assert!(matches!(
            resolve_command(":ai"),
            Some(CommandTarget::Resource(ResourceKind::Assistant))
        ));
        assert!(matches!(
            resolve_command(":q"),
            Some(CommandTarget::Quit)
        ));
    }

    #[test]
    fn test_command_suggestions() {
        let matches = command_suggestions("po");
        assert!(!matches.is_empty());
        assert_eq!(matches[0].0.name, "pods");

        let dep_matches = command_suggestions("dp");
        assert!(!dep_matches.is_empty());
        assert_eq!(dep_matches[0].0.name, "deployments");

        let deplo_matches = command_suggestions("deplo");
        assert!(!deplo_matches.is_empty());
        assert_eq!(deplo_matches[0].0.name, "deployments");
    }

    #[test]
    fn test_crd_resolution_and_matching() {
        use srelens_tui::commands::{command_suggestions_with_crds, resolve_command_with_crds, CrdMeta};

        let cilium_crd = CrdMeta {
            crd_name: "ciliumloadbalancerippools.cilium.io".to_string(),
            group: "cilium.io".to_string(),
            version: "v2".to_string(),
            kind: "CiliumLoadBalancerIPPool".to_string(),
            plural: "ciliumloadbalancerippools".to_string(),
            singular: "ciliumloadbalancerippool".to_string(),
            namespaced: false,
            short_names: vec!["ippool".to_string(), "lbippool".to_string()],
            printer_columns: vec![],
        };
        let crds = vec![cilium_crd];

        // 1. Direct plural resolution
        let res1 = resolve_command_with_crds(":ciliumloadbalancerippools", &crds);
        assert!(matches!(res1, Some(CommandTarget::CustomResource(_))));

        // 2. Acronym/abbreviation resolution (:ciliumlbippool)
        let res2 = resolve_command_with_crds(":ciliumlbippool", &crds);
        assert!(matches!(res2, Some(CommandTarget::CustomResource(_))));

        // 3. Short name resolution (:ippool)
        let res3 = resolve_command_with_crds(":ippool", &crds);
        assert!(matches!(res3, Some(CommandTarget::CustomResource(_))));

        // 4. Prefix suggestions for "cilium"
        let suggs = command_suggestions_with_crds("cilium", &crds);
        assert!(!suggs.is_empty());
        assert_eq!(suggs[0].0.name, "ciliumloadbalancerippools");

        // 5. Prefix suggestions for "ciliumlbippool"
        let suggs2 = command_suggestions_with_crds("ciliumlbippool", &crds);
        assert!(!suggs2.is_empty());
        assert_eq!(suggs2[0].0.name, "ciliumloadbalancerippools");
    }

    #[test]
    fn test_table_filtering_and_navigation() {
        let mut table = ResourceTableState::new(ResourceKind::Pods);
        let items = vec![
            json!({ "name": "nginx-auth", "namespace": "default", "status": "Running" }),
            json!({ "name": "postgres-db", "namespace": "default", "status": "Running" }),
            json!({ "name": "redis-cache", "namespace": "cache", "status": "Pending" }),
        ];

        table.set_items(items, "");
        assert_eq!(table.filtered_indices.len(), 3);
        assert_eq!(table.selected_resource_name().as_deref(), Some("nginx-auth"));

        // Navigate down
        table.select_next();
        assert_eq!(table.selected_resource_name().as_deref(), Some("postgres-db"));

        // Filter by "redis"
        table.apply_filter("redis");
        assert_eq!(table.filtered_indices.len(), 1);
        assert_eq!(table.selected_resource_name().as_deref(), Some("redis-cache"));

        // Clear filter
        table.apply_filter("");
        assert_eq!(table.filtered_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_informer_cache_instant_screen_switching() {
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::logs::LogStreamManager;
        use srelens_streams::watch::WatchManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            kubeconfig_paths: Vec::new(),
            contexts: Vec::new(),
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(ResourceTableState::new(ResourceKind::Namespaces)),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: Some("watch:prod-cluster:default:namespaces".to_string()),
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Simulate streaming snapshot arrival for pods
        let pod_payload = json!([
            { "name": "pod-1", "namespace": "default", "status": "Running" },
            { "name": "pod-2", "namespace": "default", "status": "Running" },
        ]);
        app.handle_stream_event("watch:prod-cluster:default:pods".to_string(), pod_payload);

        // Verify cache ingested the snapshot
        let cached = app.resource_cache.get(&("prod-cluster".to_string(), "default".to_string(), "pods".to_string()));
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 2);

        // 2. Switch view to Pods -> table should prime immediately from cache (0ms, is_loading = false)
        app.switch_view_to_kind(ResourceKind::Pods).await;

        if let ActiveView::Table(table) = &app.active_view {
            assert_eq!(table.is_loading, false);
            assert_eq!(table.raw_items.len(), 2);
            assert_eq!(table.selected_resource_name().as_deref(), Some("pod-1"));
        } else {
            panic!("Expected ActiveView::Table");
        }
    }

    #[tokio::test]
    async fn test_command_mode_ctrl_w_and_word_deletion() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(ResourceTableState::new(ResourceKind::Pods)),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Enter command mode by typing ':'
        app.handle_key_event(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Command);
        assert_eq!(app.command_buffer, "");

        // 2. Type "pods -n default"
        for c in "pods -n default".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        assert_eq!(app.command_buffer, "pods -n default");

        // 3. Press Option+Backspace (macOS) -> should rubout "default" to "pods -n "
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)).await;
        assert_eq!(app.command_buffer, "pods -n ");
        assert_eq!(app.input_mode, InputMode::Command);

        // 4. Press Ctrl+Backspace (Windows/Linux) -> should rubout "-n" to "pods "
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)).await;
        assert_eq!(app.command_buffer, "pods ");

        // 5. Press Ctrl+W (Unix/Vim) -> should rubout "pods" to ""
        app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)).await;
        assert_eq!(app.command_buffer, "");
        assert_eq!(app.input_mode, InputMode::Command); // still in command mode!

        // 6. Press Option+Backspace on empty buffer -> should exit command mode to Normal!
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)).await;
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[tokio::test]
    async fn test_ai_settings_navigation_and_editing() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::views::SettingField;
        use srelens_tui::ai_config::AiProvider;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(ResourceTableState::new(ResourceKind::Pods)),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Switch to settings view via :settings command
        app.execute_colon_command("settings").await;
        assert!(matches!(app.active_view, ActiveView::Settings(_)));
        if let ActiveView::Settings(s) = &mut app.active_view {
            s.settings = srelens_tui::AiSettings::default();
            s.selected_provider_idx = 0; // Anthropic (index 0)
        }

        // 2. Select next provider (OpenAI)
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.current_provider(), AiProvider::OpenAi);
        } else {
            panic!("Expected ActiveView::Settings");
        }

        // 3. Toggle OpenAI as active provider with [Space]
        app.handle_key_event(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.settings.default_provider, AiProvider::OpenAi);
        }

        // 4. Tab to API Key field
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.selected_field, SettingField::ApiKey);
        }

        // 5. Press 'e' to edit API Key
        app.handle_key_event(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.is_editing, true);
        }

        // Type "sk-test-openai-key"
        for c in "sk-test-openai-key".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }

        // Press Enter to confirm edit
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.is_editing, false);
            assert_eq!(s.settings.get_api_key(AiProvider::OpenAi).as_deref(), Some("sk-test-openai-key"));
        }

        // 6. Press 's' to save settings to memory/disk
        app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)).await;
        assert_eq!(app.ai_settings.default_provider, AiProvider::OpenAi);
        assert_eq!(app.ai_settings.get_api_key(AiProvider::OpenAi).as_deref(), Some("sk-test-openai-key"));

        // 7. Test navigating all the way to Cursor Agent
        // Currently at OpenAi (index 1), press 'j' 3 times to get to Cursor (index 4)
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)).await; // Gemini (2)
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)).await; // OpenAICompatible (3)
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)).await; // Cursor (4)

        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.current_provider(), AiProvider::Cursor);
        }

        // Select Cursor as active provider
        app.handle_key_event(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)).await;
        assert_eq!(app.ai_settings.default_provider, AiProvider::Cursor);

        // 8. Currently at ApiKey, Tab twice to get to Timeout field
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await; // Model
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await; // Timeout
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.selected_field, SettingField::Timeout);
        }

        // Press 'e' to edit Timeout
        app.handle_key_event(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.is_editing, true);
        }

        // Backspace default 120 and type 240
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)).await;
        for c in "240".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)).await;

        assert_eq!(app.ai_settings.get_timeout_seconds(AiProvider::Cursor), 240);

        // 9. Test pasting into an edit buffer
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await; // ProviderToggle
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await; // ApiKey
        app.handle_key_event(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)).await; // Open edit dialog
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.is_editing, true);
        }
        app.handle_paste("pasted-cursor-api-key-12345".to_string());
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.edit_buffer, "pasted-cursor-api-key-12345");
        }
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        if let ActiveView::Settings(s) = &app.active_view {
            assert_eq!(s.is_editing, false);
            assert_eq!(s.settings.get_api_key(AiProvider::Cursor).as_deref(), Some("pasted-cursor-api-key-12345"));
        }
    }

    #[tokio::test]
    async fn test_assistant_typing_s_and_ctrl_s_shortcut() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Assistant,
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. In Assistant view, typing 's' when input is empty should type 's' into prompt, NOT jump to settings!
        app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)).await;
        assert!(matches!(app.active_view, ActiveView::Assistant));
        assert_eq!(app.assistant_state.input, "s");

        // Type "how me pods"
        for c in "how me pods".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        assert_eq!(app.assistant_state.input, "show me pods");

        // 2. Test word deletion in Assistant view:
        // Option + Backspace (macOS) -> rubs out "pods" -> "show me "
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)).await;
        assert_eq!(app.assistant_state.input, "show me ");

        // Ctrl + Backspace (Windows/Linux) -> rubs out "me" -> "show "
        app.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)).await;
        assert_eq!(app.assistant_state.input, "show ");

        // Ctrl + w (Unix/Vim) -> rubs out "show" -> ""
        app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)).await;
        assert_eq!(app.assistant_state.input, "");

        // 3. Pressing Ctrl+s should open Settings!
        app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)).await;
        assert!(matches!(app.active_view, ActiveView::Settings(_)));
    }

    #[tokio::test]
    async fn test_assistant_scrolling_and_auto_follow() {
        use srelens_tui::views::assistant_view::AssistantViewState;

        let mut ai = AssistantViewState::new();
        // Set last_max_scroll to 50
        ai.last_max_scroll.set(50);
        ai.last_total_lines.set(75);
        assert!(ai.auto_scroll);

        // 1. Scrolling up disengages auto_scroll and scrolls up from bottom
        ai.scroll_up(5);
        assert!(!ai.auto_scroll);
        assert_eq!(ai.scroll_offset, 45);

        // Scroll up more
        ai.scroll_up(10);
        assert_eq!(ai.scroll_offset, 35);

        // 2. Scroll to top
        ai.scroll_to_top();
        assert_eq!(ai.scroll_offset, 0);
        assert!(!ai.auto_scroll);

        // 3. Scroll down
        ai.scroll_down(20);
        assert_eq!(ai.scroll_offset, 20);
        assert!(!ai.auto_scroll);

        // Scroll down past max_scroll re-engages auto_scroll
        ai.scroll_down(40);
        assert_eq!(ai.scroll_offset, 50);
        assert!(ai.auto_scroll);

        // 4. scroll_to_bottom re-engages auto_scroll
        ai.scroll_up(10);
        assert!(!ai.auto_scroll);
        ai.scroll_to_bottom();
        assert!(ai.auto_scroll);
        assert_eq!(ai.scroll_offset, 50);

        // 5. Sending new message re-engages auto_scroll
        ai.scroll_up(15);
        assert!(!ai.auto_scroll);
        ai.start_turn("new question".to_string());
        assert!(ai.auto_scroll);
    }

    #[tokio::test]
    async fn test_assistant_tool_calls_and_token_usage_lifecycle() {
        use srelens_tui::views::assistant_view::{AssistantViewState, TokenUsage, ToolCallStatus};

        let mut ai = AssistantViewState::new();
        ai.start_turn("Which pods are crashing?".to_string());

        // 1. Tool call starts
        ai.add_tool_call_start(
            "call_1".to_string(),
            "bash".to_string(),
            "kubectl get pods -A".to_string(),
        );

        let last_msg = ai.messages.last().expect("last message");
        assert_eq!(last_msg.tool_calls.len(), 1);
        assert_eq!(last_msg.tool_calls[0].tool, "bash");
        assert_eq!(last_msg.tool_calls[0].args_summary, "kubectl get pods -A");
        assert_eq!(last_msg.tool_calls[0].status, ToolCallStatus::Running);

        // 2. Tool call completes
        ai.finish_tool_call("call_1", ToolCallStatus::Success);
        let last_msg = ai.messages.last().expect("last message");
        assert_eq!(last_msg.tool_calls[0].status, ToolCallStatus::Success);

        // 3. Second tool call
        ai.add_tool_call_start(
            "call_2".to_string(),
            "read".to_string(),
            "path: k8s/deploy.yaml".to_string(),
        );
        ai.finish_tool_call(
            "call_2",
            ToolCallStatus::Error("file not found".to_string()),
        );
        let last_msg = ai.messages.last().expect("last message");
        assert_eq!(last_msg.tool_calls.len(), 2);
        assert_eq!(
            last_msg.tool_calls[1].status,
            ToolCallStatus::Error("file not found".to_string())
        );

        // 4. Stream response text
        ai.append_stream_chunk("Found 2 crashing pods.");
        let last_msg = ai.messages.last().expect("last message");
        assert_eq!(last_msg.content, "Found 2 crashing pods.");

        // 5. Token usage
        ai.set_token_usage(TokenUsage {
            prompt_tokens: 3500,
            completion_tokens: 120,
            cached_tokens: 2400,
            total_tokens: 3620,
            duration_ms: Some(2150),
        });

        let last_msg = ai.messages.last().expect("last message");
        let usage = last_msg.token_usage.as_ref().expect("usage set");
        assert_eq!(usage.prompt_tokens, 3500);
        assert_eq!(usage.completion_tokens, 120);
        assert_eq!(usage.cached_tokens, 2400);
        assert_eq!(usage.total_tokens, 3620);
        assert_eq!(usage.duration_ms, Some(2150));
    }

    #[tokio::test]
    async fn test_assistant_conversation_persistence_across_view_switches() {
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::views::assistant_view::ToolCallStatus;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Assistant,
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. User starts turn in Assistant view
        app.assistant_state.start_turn("Which nodes have GPUs?".to_string());
        app.assistant_state.add_tool_call_start(
            "call_gpu_1".to_string(),
            "bash".to_string(),
            "kubectl get nodes -l nvidia.com/gpu".to_string(),
        );
        app.assistant_state.finish_tool_call("call_gpu_1", ToolCallStatus::Success);
        app.assistant_state.append_stream_chunk("Node worker-gpu-1 has an NVIDIA A100 GPU.");
        app.assistant_state.finish_turn();

        assert_eq!(app.assistant_state.messages.len(), 3);
        assert_eq!(app.assistant_state.messages[2].tool_calls.len(), 1);

        // 2. User navigates away to Pods view (:pods)
        app.switch_view_to_kind(ResourceKind::Pods).await;
        assert!(matches!(app.active_view, ActiveView::Table(_)));

        // 3. While in Pods view, a background stream chunk or status update arrives
        app.assistant_state.append_stream_chunk(" Also, driver version is 535.129.");

        // 4. User navigates back to Assistant view (via Tab or switch_view_to_kind)
        app.switch_view_to_kind(ResourceKind::Assistant).await;
        assert!(matches!(app.active_view, ActiveView::Assistant));

        // 5. Verify entire conversation is intact!
        assert_eq!(app.assistant_state.messages.len(), 3);
        assert_eq!(app.assistant_state.messages[1].content, "Which nodes have GPUs?");
        assert!(app.assistant_state.messages[2].content.contains("Node worker-gpu-1 has an NVIDIA A100 GPU. Also, driver version is 535.129."));
        assert_eq!(app.assistant_state.messages[2].tool_calls[0].tool, "bash");
    }

    #[tokio::test]
    async fn test_assistant_conversation_export_and_clear() {
        use srelens_tui::views::assistant_view::{AssistantViewState, TokenUsage, ToolCallStatus};

        let mut ai = AssistantViewState::new();
        ai.start_turn("Test query".to_string());
        ai.add_tool_call_start("c1".to_string(), "bash".to_string(), "kubectl get nodes".to_string());
        ai.finish_tool_call("c1", ToolCallStatus::Success);
        ai.append_stream_chunk("Cluster has 3 nodes.");
        ai.set_token_usage(TokenUsage {
            prompt_tokens: 1500,
            completion_tokens: 25,
            cached_tokens: 500,
            total_tokens: 1525,
            duration_ms: Some(1200),
        });

        // 1. Test markdown export
        let md = ai.export_to_markdown("Cursor Agent", "default");
        assert!(md.contains("# SRElens AI Assistant Conversation Export"));
        assert!(md.contains("Test query"));
        assert!(md.contains("Cluster has 3 nodes."));
        assert!(md.contains("`bash`: `kubectl get nodes` [ok]"));
        assert!(md.contains("1,525 tokens"));

        // 2. Test saving to custom file
        let temp_dir = tempfile::tempdir().unwrap();
        let export_path = temp_dir.path().join("saved_chat.md");
        let result = ai.save_conversation_to_file("Cursor Agent", "default", Some(export_path.to_str().unwrap()));
        assert!(result.is_ok());
        let saved_content = std::fs::read_to_string(&export_path).unwrap();
        assert_eq!(saved_content, md);

        // 3. Test clear conversation
        ai.clear_conversation();
        assert_eq!(ai.messages.len(), 1);
        assert_eq!(ai.messages[0].role, "assistant");
        assert!(ai.messages[0].content.contains("Hello! I am your SRElens AI Assistant"));
        assert!(ai.messages[0].tool_calls.is_empty());
    }

    #[test]
    fn test_filter_out_hook_additional_contexts() {
        use srelens_tui::app::extract_tool_call_start_info;

        // JSON emitted by cursor-agent with both real tool (bashToolCall) and hookAdditionalContexts
        let json_str = r#"{
            "type": "tool_call",
            "subtype": "started",
            "call_id": "call_123",
            "tool_call": {
                "hookAdditionalContexts": [],
                "bashToolCall": {
                    "args": {
                        "command": "kubectl get pods -A"
                    }
                },
                "toolCallId": "call_123",
                "startedAtMs": "1786359191055"
            }
        }"#;
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let extracted = extract_tool_call_start_info(&v);
        assert!(extracted.is_some());
        let (id, tool, args) = extracted.unwrap();
        assert_eq!(id, "call_123");
        assert_eq!(tool, "bash");
        assert_eq!(args, "kubectl get pods -A");

        // JSON with ONLY hookAdditionalContexts metadata and no real ToolCall
        let json_hook_only = r#"{
            "type": "tool_call",
            "subtype": "started",
            "call_id": "call_999",
            "tool_call": {
                "hookAdditionalContexts": [],
                "toolCallId": "call_999"
            }
        }"#;
        let v_hook: serde_json::Value = serde_json::from_str(json_hook_only).unwrap();
        let extracted_hook = extract_tool_call_start_info(&v_hook);
        assert!(extracted_hook.is_none(), "hookAdditionalContexts without a ToolCall key should be ignored");
    }

    #[tokio::test]
    async fn test_regex_search_single_item_combines_enter_to_go_to_resource() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::views::resource_table::ResourceTableState;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut table = ResourceTableState::new(ResourceKind::Namespaces);
        table.set_items(vec![
            serde_json::json!({ "name": "default" }),
            serde_json::json!({ "name": "kube-system" }),
            serde_json::json!({ "name": "production" }),
        ], "");

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string(), "kube-system".to_string(), "production".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Enter filter mode with '/'
        app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Filter);

        // 2. Type regex "prod.*" -> filters down to exactly 1 item: "production"
        for c in "prod.*".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        if let ActiveView::Table(t) = &app.active_view {
            assert_eq!(t.filtered_indices.len(), 1);
            assert_eq!(t.selected_resource_name().as_deref(), Some("production"));
        }

        // 3. Press Enter ONCE.
        // Because only 1 item remained in search, it should immediately exit filter mode
        // AND drill-down to that resource (switching namespace to 'production' and view to Pods)!
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.active_namespace, "production");
        if let ActiveView::Table(t) = &app.active_view {
            assert_eq!(t.kind, ResourceKind::Pods);
        } else {
            panic!("Expected active_view to switch to Pods table");
        }
    }

    #[test]
    fn test_regex_filtering_supports_regex_syntax() {
        use srelens_tui::views::resource_table::ResourceTableState;

        let mut table = ResourceTableState::new(ResourceKind::Pods);
        table.set_items(vec![
            serde_json::json!({ "name": "api-gateway-7f89d", "namespace": "prod" }),
            serde_json::json!({ "name": "auth-service-5d6b", "namespace": "prod" }),
            serde_json::json!({ "name": "db-postgres-0", "namespace": "database" }),
            serde_json::json!({ "name": "frontend-webapp-1", "namespace": "staging" }),
        ], "");

        // Pattern matching start of string ^api
        table.apply_filter("^api");
        assert_eq!(table.filtered_indices.len(), 1);
        assert_eq!(table.selected_resource_name().as_deref(), Some("api-gateway-7f89d"));

        // Alternation regex api|frontend
        table.apply_filter("api|frontend");
        assert_eq!(table.filtered_indices.len(), 2);

        // Character class with digit
        table.apply_filter(r"postgres-\d");
        assert_eq!(table.filtered_indices.len(), 1);
        assert_eq!(table.selected_resource_name().as_deref(), Some("db-postgres-0"));
    }

    #[tokio::test]
    async fn test_port_forward_keybinding_on_pods() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::{InputMode, Modal};
        use srelens_tui::views::resource_table::ResourceTableState;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut table = ResourceTableState::new(ResourceKind::Pods);
        table.set_items(vec![
            serde_json::json!({
                "name": "my-api-pod-xyz",
                "namespace": "default",
                "spec": {
                    "containers": [{
                        "name": "api",
                        "ports": [{ "containerPort": 3000 }]
                    }]
                }
            }),
        ], "");

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // Press 'f' (or 'F') on the selected pod -> opens PortForward modal with detected port 3000
        app.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)).await;
        match app.modal {
            Some(Modal::PortForward { pod_name, container_port, .. }) => {
                assert_eq!(pod_name, "my-api-pod-xyz");
                assert_eq!(container_port, 3000);
            }
            other => panic!("Expected PortForward modal, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_pod_metrics_usage_update_and_column_extraction() {
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::views::resource_table::{extract_field_str, ResourceTableState};
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut table = ResourceTableState::new(ResourceKind::Pods);
        table.set_items(vec![
            serde_json::json!({
                "name": "copy-controller-7b44647bcd-rzd8x",
                "namespace": "copy-controller",
                "phase": "Running",
                "ready": "1/1",
                "restarts": 1,
                "node": "data-processing-prod",
                "age": "70d",
            }),
        ], "");

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-cluster".to_string(),
            active_namespace: "copy-controller".to_string(),
            contexts: vec![],
            namespaces: vec!["copy-controller".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "copy-controller".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.31.7".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 32,
            pod_count: 1,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // Initially without metrics, extract_field_str returns "-"
        if let ActiveView::Table(t) = &app.active_view {
            let item = &t.raw_items[0];
            assert_eq!(extract_field_str(item, "cpu"), "-");
            assert_eq!(extract_field_str(item, "memory"), "-");
        }

        // Simulate metrics update received from metrics.k8s.io
        let metrics_json = serde_json::json!([
            {
                "name": "copy-controller-7b44647bcd-rzd8x",
                "namespace": "copy-controller",
                "cpuMillicores": 15,
                "memoryMiB": 128
            }
        ]).to_string();

        app.handle_pod_metrics_update(&metrics_json);

        // Verify that CPU and Memory columns now show the live formatted usage
        if let ActiveView::Table(t) = &app.active_view {
            let item = &t.raw_items[0];
            assert_eq!(extract_field_str(item, "cpu"), "15m");
            assert_eq!(extract_field_str(item, "memory"), "128Mi");
        } else {
            panic!("Expected active_view to be Table");
        }
    }

    #[tokio::test]
    async fn test_native_mcp_agent_invoker_and_tool_execution() {
        use srelens_llm::ToolInvoker;
        use srelens_tui::agent::{build_mcp_server, McpToolInvoker};
        use srelens_kube::client_cache::ClientCache;
        use std::path::PathBuf;

        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let server = build_mcp_server(client_cache, vec![]);
        let invoker = McpToolInvoker::new(server);

        // 1. Tool listing returns all k8s capabilities with provider-safe names
        let tools = invoker.list_tools().await.expect("tools list succeeds");
        assert!(tools.len() >= 30, "expected at least 30 K8s tools, got {}", tools.len());

        let list_pods_tool = tools.iter().find(|t| t.name == "k8s_listPods").expect("k8s_listPods tool exists");
        assert!(list_pods_tool.read_only, "k8s_listPods should be marked read-only");

        // 2. Tool invocation executes through srelens_mcp in-process
        let res = invoker.call_tool("k8s_listPods", &serde_json::json!({
            "context": "nonexistent-cluster",
            "namespace": "default"
        })).await.expect("call_tool returns result");

        // Should return a response without panicking (even on nonexistent cluster it surfaces error message)
        assert!(!res.content.is_empty(), "result content should not be empty");
    }

    #[tokio::test]
    async fn test_assistant_mouse_selection_copy_and_bracketed_paste() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "kind-dev".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Assistant,
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Test paste handling into Assistant input
        app.handle_paste("paste line 1\npaste line 2".to_string());
        assert_eq!(app.assistant_state.input, "paste line 1 paste line 2");

        // 2. Test mouse selection and copy with 'c'
        *app.assistant_state.plain_lines.borrow_mut() = vec![
            "Pod crash occurred in container backend: OutOfMemory".to_string()
        ];
        app.assistant_state.start_selection(0, 41); // start of "OutOfMemory"
        app.assistant_state.update_selection(0, 52);
        app.assistant_state.finish_selection(0, 52);

        assert_eq!(app.assistant_state.get_selected_text().as_deref(), Some("OutOfMemory"));

        // Pressing 'c' copies selection and toasts
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)).await;
        assert!(app.toast.is_some());
        assert!(app.toast.as_ref().unwrap().0.contains("Copied selection"));

        // Pressing Esc clears selection
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.selection, None);
    }

    #[tokio::test]
    async fn test_assistant_bottom_bar_hints_only_cmd_and_help() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "kind-dev".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Assistant,
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // When input is empty, typing '?' opens help modal
        app.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)).await;
        assert_eq!(app.show_help, true);
        app.show_help = false;

        // When typing question in prompt, '?' is typed into prompt, NOT triggering help modal!
        app.assistant_state.input = "how to fix".to_string();
        app.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)).await;
        assert_eq!(app.show_help, false);
        assert_eq!(app.assistant_state.input, "how to fix?");
    }

    #[tokio::test]
    async fn test_assistant_mouse_click_tool_chip_and_tab_view_toggle() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use ratatui::layout::Rect;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;

        use srelens_tui::commands::ResourceKind;
        use srelens_tui::views::resource_table::ResourceTableState;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "kind-dev".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Assistant,
            nav_stack: vec![ActiveView::Table(ResourceTableState::new(ResourceKind::Nodes))],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // Set viewport and simulate tool chip line at index 2
        app.assistant_state.last_viewport_rect.set(Rect { x: 0, y: 0, width: 80, height: 24 });
        app.assistant_state.tool_chip_lines.borrow_mut().push(2);

        // 1. Left clicking on row 2 toggles expand_tools!
        assert_eq!(app.assistant_state.expand_tools, false);
        let click_chip = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(click_chip).await;
        assert_eq!(app.assistant_state.expand_tools, true);

        // Clicking again collapses it
        app.handle_mouse(click_chip).await;
        assert_eq!(app.assistant_state.expand_tools, false);

        // 2. Ctrl+t also toggles expand_tools
        app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)).await;
        assert_eq!(app.assistant_state.expand_tools, true);

        // 3. Pressing Tab toggles back to previous view (Nodes)!
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
        assert!(matches!(app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Nodes));
    }

    #[tokio::test]
    async fn test_header_context_chips_mouse_click_and_hotbar() {
        use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Rect;
        use tokio::sync::mpsc::unbounded_channel;
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::collections::{HashMap, HashSet};
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::ui::dialogs::Modal;
        use srelens_tui::views::resource_table::ResourceTableState;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-eu".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![
                srelens_kube::contexts::ContextDto {
                    name: "prod-eu".to_string(),
                    stable_id: "kube/prod-eu".to_string(),
                    cluster: "prod-cluster".to_string(),
                    server: "https://127.0.0.1:6443".to_string(),
                    namespace: "default".to_string(),
                    is_current: true,
                    is_local: false,
                    provider: Some("EKS".to_string()),
                    source_file: "config".to_string(),
                    auth_kind: "token".to_string(),
                },
                srelens_kube::contexts::ContextDto {
                    name: "kind-dev".to_string(),
                    stable_id: "kube/kind-dev".to_string(),
                    cluster: "kind-cluster".to_string(),
                    server: "https://127.0.0.1:6444".to_string(),
                    namespace: "default".to_string(),
                    is_current: false,
                    is_local: true,
                    provider: Some("kind".to_string()),
                    source_file: "config".to_string(),
                    auth_kind: "client certificate".to_string(),
                },
            ],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(ResourceTableState::new(ResourceKind::Pods)),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod-cluster".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // Simulate header chips at columns 12..25 (prod-eu) and 26..40 (kind-dev) on row 0
        app.context_chip_rects.borrow_mut().push((Rect { x: 12, y: 0, width: 14, height: 1 }, "prod-eu".to_string()));
        app.context_chip_rects.borrow_mut().push((Rect { x: 27, y: 0, width: 14, height: 1 }, "kind-dev".to_string()));

        // Clicking on kind-dev chip switches context!
        assert_eq!(app.active_context, "prod-eu");
        let click_kind = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 30,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(click_kind).await;
        assert_eq!(app.active_context, "kind-dev");

        // Open context picker via :ctx command
        app.open_context_picker();
        assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));

        // Filter contexts by typing 'prod'
        app.handle_key_event(KeyEvent::new(KeyCode::Char('p'), crossterm::event::KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), crossterm::event::KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), crossterm::event::KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Char('d'), crossterm::event::KeyModifiers::NONE)).await;

        if let Some(Modal::ContextPicker { filter, .. }) = &app.modal {
            assert_eq!(filter, "prod");
        } else {
            panic!("Expected Modal::ContextPicker with filter");
        }

        // Hitting Enter selects the filtered prod-eu context!
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE)).await;
        assert_eq!(app.active_context, "prod-eu");
        assert!(app.modal.is_none());
    }

    #[tokio::test]
    async fn test_per_cluster_assistant_state_isolation() {
        use tokio::sync::mpsc::unbounded_channel;
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::collections::{HashMap, HashSet};
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "data-processing-prod-eu-dus1".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![
                srelens_kube::contexts::ContextDto {
                    name: "data-processing-prod-eu-dus1".to_string(),
                    stable_id: "kube/prod".to_string(),
                    cluster: "prod-cluster".to_string(),
                    server: "https://127.0.0.1:6443".to_string(),
                    namespace: "default".to_string(),
                    is_current: true,
                    is_local: false,
                    provider: Some("EKS".to_string()),
                    source_file: "config".to_string(),
                    auth_kind: "token".to_string(),
                },
                srelens_kube::contexts::ContextDto {
                    name: "harvester-amd-eu-dus1".to_string(),
                    stable_id: "kube/harvester".to_string(),
                    cluster: "harvester-cluster".to_string(),
                    server: "https://127.0.0.1:6444".to_string(),
                    namespace: "kube-system".to_string(),
                    is_current: false,
                    is_local: true,
                    provider: Some("kind".to_string()),
                    source_file: "config".to_string(),
                    auth_kind: "client certificate".to_string(),
                },
            ],
            namespaces: vec!["default".to_string(), "kube-system".to_string()],
            active_view: ActiveView::Assistant,
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod-cluster".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 32,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("data-processing-prod-eu-dus1"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. In data-processing-prod-eu-dus1, user has a conversation
        app.assistant_state.start_turn("Show me nodes".to_string());
        app.assistant_state.add_assistant_message("32 nodes on data-processing-prod-eu-dus1: 3 control plane, 8 flink amd64, 16 general, 4 gpu".to_string());
        app.assistant_state.finish_turn();

        assert_eq!(app.assistant_state.messages.len(), 3);
        assert!(app.assistant_state.messages.iter().any(|m| m.content.contains("32 nodes on data-processing-prod-eu-dus1")));

        // 2. Switch context to harvester-amd-eu-dus1
        app.switch_context("harvester-amd-eu-dus1".to_string()).await;
        assert_eq!(app.active_context, "harvester-amd-eu-dus1");
        assert_eq!(app.active_namespace, "kube-system"); // uses ctx.namespace!

        // The assistant state for harvester should be fresh, NOT showing data-processing nodes!
        assert_eq!(app.assistant_state.context_name, "harvester-amd-eu-dus1");
        assert_eq!(app.assistant_state.messages.len(), 1);
        assert!(!app.assistant_state.messages.iter().any(|m| m.content.contains("32 nodes on data-processing-prod-eu-dus1")));

        // User chats in harvester
        app.assistant_state.start_turn("Show me pods".to_string());
        app.assistant_state.add_assistant_message("7 pods on harvester-amd-eu-dus1".to_string());
        app.assistant_state.finish_turn();
        assert_eq!(app.assistant_state.messages.len(), 3);

        // 3. Switch back to data-processing-prod-eu-dus1
        app.switch_context("data-processing-prod-eu-dus1".to_string()).await;
        assert_eq!(app.active_context, "data-processing-prod-eu-dus1");
        assert_eq!(app.active_namespace, "default");

        // The original conversation from data-processing is completely restored!
        assert_eq!(app.assistant_state.context_name, "data-processing-prod-eu-dus1");
        assert!(app.assistant_state.messages.iter().any(|m| m.content.contains("32 nodes on data-processing-prod-eu-dus1")));
        assert!(!app.assistant_state.messages.iter().any(|m| m.content.contains("7 pods on harvester-amd-eu-dus1")));
    }

    #[tokio::test]
    async fn test_deep_link_navigation_and_copy_shortcuts() {
        use tokio::sync::mpsc::unbounded_channel;
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::collections::{HashMap, HashSet};
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::views::resource_table::ResourceTableState;
        use srelens_tui::deep_link::DeepLink;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut table = ResourceTableState::new(ResourceKind::Pods);
        table.set_items(
            vec![
                serde_json::json!({
                    "name": "payment-api-pod-1",
                    "namespace": "production",
                    "ready": "1/1",
                    "status": "Running",
                    "restarts": 0,
                    "age": "2d"
                }),
                serde_json::json!({
                    "name": "auth-api-pod-2",
                    "namespace": "production",
                    "ready": "1/1",
                    "status": "Running",
                    "restarts": 0,
                    "age": "1d"
                }),
            ],
            "",
        );

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "prod-eu".to_string(),
            active_namespace: "production".to_string(),
            contexts: vec![
                srelens_kube::contexts::ContextDto {
                    name: "prod-eu".to_string(),
                    stable_id: "kube/prod".to_string(),
                    cluster: "prod-cluster".to_string(),
                    server: "https://127.0.0.1:6443".to_string(),
                    namespace: "production".to_string(),
                    is_current: true,
                    is_local: false,
                    provider: Some("EKS".to_string()),
                    source_file: "config".to_string(),
                    auth_kind: "token".to_string(),
                },
                srelens_kube::contexts::ContextDto {
                    name: "staging-us".to_string(),
                    stable_id: "kube/staging".to_string(),
                    cluster: "staging-cluster".to_string(),
                    server: "https://127.0.0.1:6444".to_string(),
                    namespace: "staging-ns".to_string(),
                    is_current: false,
                    is_local: false,
                    provider: Some("GKE".to_string()),
                    source_file: "config".to_string(),
                    auth_kind: "token".to_string(),
                },
            ],
            namespaces: vec!["default".to_string(), "production".to_string(), "staging-ns".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "production".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod-cluster".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 12,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("prod-eu"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Pressing 'c' on the selected row copies the resource name!
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)).await;
        assert!(app.toast.is_some());
        let toast = app.toast.as_ref().unwrap();
        assert!(toast.0.contains("Copied 'payment-api-pod-1' to clipboard"));

        // Pressing '<Ctrl+y>' copies the canonical deep link URL!
        app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)).await;
        assert!(app.toast.is_some());
        let toast = app.toast.as_ref().unwrap();
        assert!(toast.0.contains("Copied deep link: srelens://resource/prod-eu/production/Pod/payment-api-pod-1"));

        // 2. In-app navigation via :open <url>
        let cmd = ":open srelens://resource/staging-us/staging-ns/Deployments/frontend";
        let target = srelens_tui::commands::resolve_command(cmd).expect("resolve :open");
        app.execute_command_target(target).await;

        // Context, namespace, and view all switched seamlessly!
        assert_eq!(app.active_context, "staging-us");
        assert_eq!(app.active_namespace, "staging-ns");
        assert!(matches!(app.active_view, ActiveView::Table(ref t) if t.kind == ResourceKind::Deployments));
        assert!(app.toast.is_some());
        assert!(app.toast.as_ref().unwrap().0.contains("Navigated to Deployments 'frontend'"));

        // 3. Cluster deep link: srelens://cluster/prod-eu
        let link = DeepLink::parse("srelens://cluster/prod-eu").unwrap();
        app.navigate_deep_link(&link).await.expect("navigate cluster");
        assert_eq!(app.active_context, "prod-eu");
    }

    #[tokio::test]
    async fn test_live_cluster_overview_and_capacity_gauges() {
        use tokio::sync::mpsc::unbounded_channel;
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::collections::{HashMap, HashSet};
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::views::resource_table::ResourceTableState;
        use srelens_tui::views::overview_view::ClusterOverviewData;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            kubeconfig_paths: vec![],
            active_context: "data-processing-prod-eu-dus1".to_string(),
            active_namespace: "default".to_string(),
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(ResourceTableState::new(ResourceKind::Pods)),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.34.7+rke2r1".to_string(),
            cluster_name: "prod-cluster-dus1".to_string(),
            server_url: "https://k8s.prod.dus1:6443".to_string(),
            node_count: 32,
            pod_count: 184,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Switch view to Overview
        app.switch_view_to_kind(ResourceKind::Overview).await;

        // Verify that overview view is immediately initialized with reachable status and real cluster info
        if let ActiveView::Overview(ref ov) = app.active_view {
            assert!(ov.data.is_reachable, "Cluster must be marked reachable");
            assert_eq!(ov.data.context_name, "data-processing-prod-eu-dus1");
            assert_eq!(ov.data.k8s_version, "v1.34.7+rke2r1");
            assert_eq!(ov.data.node_count, 32);
            assert_eq!(ov.data.total_pods, 184);
        } else {
            panic!("Expected ActiveView::Overview");
        }

        // 2. Simulate live background metrics arriving from cluster
        let live_data = ClusterOverviewData {
            context_name: "data-processing-prod-eu-dus1".to_string(),
            cluster_name: "prod-cluster-dus1".to_string(),
            server_url: "https://k8s.prod.dus1:6443".to_string(),
            k8s_version: "v1.34.7+rke2r1".to_string(),
            is_reachable: true,
            node_count: 32,
            ready_nodes: 32,
            total_pods: 184,
            running_pods: 178,
            pending_pods: 2,
            failed_pods: 4,
            total_cpu_millicores: 64_000,
            used_cpu_millicores: 28_500,
            total_mem_mib: 256 * 1024,
            used_mem_mib: 142 * 1024,
            total_gpus: 8,
            allocated_gpus: 6,
            total_gpu_mem_mib: 128 * 1024,
            used_gpu_mem_mib: 35 * 1024,
        };
        let payload = serde_json::to_string(&live_data).unwrap();
        app.handle_cluster_overview_update(&payload);

        // 3. Verify that overview view and app state updated with full telemetry
        if let ActiveView::Overview(ref ov) = app.active_view {
            assert_eq!(ov.data.ready_nodes, 32);
            assert_eq!(ov.data.running_pods, 178);
            assert_eq!(ov.data.pending_pods, 2);
            assert_eq!(ov.data.failed_pods, 4);
            assert_eq!(ov.data.total_cpu_millicores, 64_000);
            assert_eq!(ov.data.used_cpu_millicores, 28_500);
            assert_eq!(ov.data.total_gpus, 8);
            assert_eq!(ov.data.allocated_gpus, 6);
        } else {
            panic!("Expected ActiveView::Overview");
        }

        // 4. Test copying from Overview: 'c' copies summary report, '<Ctrl+y>' copies cluster deep link
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('c'), crossterm::event::KeyModifiers::NONE)).await;
        let toast = app.toast.as_ref().expect("toast after pressing c in overview");
        assert_eq!(toast.0, "Copied cluster overview summary to clipboard");

        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('y'), crossterm::event::KeyModifiers::CONTROL)).await;
        let toast = app.toast.as_ref().expect("toast after pressing Ctrl+y in overview");
        assert!(toast.0.contains("Copied deep link: srelens://cluster/data-processing-prod-eu-dus1"));
    }

    #[tokio::test]
    async fn test_cluster_events_stream_and_warning_triage() {
        use srelens_tui::app::{ActiveView, App};
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            Some("prod-cluster".to_string()),
            Some("prod".to_string()),
            false,
            None,
            vec![],
            tx,
        ).await.unwrap();

        // 1. Switch to Events view
        app.switch_view_to_kind(ResourceKind::Events).await;

        let ev1 = serde_json::json!({
            "name": "default/pod-normal.17b",
            "namespace": "default",
            "type": "Normal",
            "reason": "Scheduled",
            "object": "Pod/frontend-web",
            "message": "Successfully assigned default/frontend-web to node-1",
            "age": "3m"
        });

        let ev2 = serde_json::json!({
            "name": "prod/pod-crash.17c",
            "namespace": "prod",
            "type": "Warning",
            "reason": "CrashLoopBackOff",
            "object": "Pod/api-backend-xyz",
            "message": "Back-off restarting failed container",
            "age": "30s"
        });

        let ev3 = serde_json::json!({
            "name": "prod/pod-oom.17d",
            "namespace": "prod",
            "type": "Warning",
            "reason": "OOMKilled",
            "object": "Pod/ml-worker-gpu-0",
            "message": "Container exceeded 32Gi memory limit and was killed",
            "age": "10s"
        });

        let events_payload = serde_json::json!([ev1, ev2, ev3]);
        app.handle_stream_event("watch:prod-cluster:prod:events".to_string(), events_payload);

        if let ActiveView::Table(ref table) = app.active_view {
            assert_eq!(table.filtered_indices.len(), 3);
        } else {
            panic!("Expected ActiveView::Table(Events)");
        }

        // 2. Press 'w' to toggle Warning Triage ON
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('w'), crossterm::event::KeyModifiers::NONE)).await;
        let toast = app.toast.as_ref().expect("toast after toggling warning triage");
        assert!(toast.0.contains("Warning Triage: ON (2 warnings)"));

        if let ActiveView::Table(ref table) = app.active_view {
            assert_eq!(table.filtered_indices.len(), 2);
            assert!(table.warning_triage);
        }

        // 3. Test Copy on Event: 'c' copies event message, '<Ctrl+y>' copies deep link
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('c'), crossterm::event::KeyModifiers::NONE)).await;
        let toast = app.toast.as_ref().expect("toast after pressing c on event");
        assert!(toast.0.contains("Copied event message to clipboard"));

        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('y'), crossterm::event::KeyModifiers::CONTROL)).await;
        let toast = app.toast.as_ref().expect("toast after pressing Ctrl+y on event");
        assert!(toast.0.contains("Copied deep link: srelens://resource/prod-cluster/prod/Pod/api-backend-xyz"));

        // 4. Press Enter on the event row -> jumps to Pod table with filter applied!
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE)).await;
        if let ActiveView::Table(ref table) = app.active_view {
            assert_eq!(table.kind, ResourceKind::Pods);
            assert_eq!(app.filter_buffer, "api-backend-xyz");
        } else {
            panic!("Expected ActiveView::Table(Pods) after Enter on Pod event");
        }

        // 5. First Esc clears filter, second Esc pops nav stack back to Events view
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE)).await;
        assert_eq!(app.filter_buffer, "");
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE)).await;
        if let ActiveView::Table(ref table) = app.active_view {
            assert_eq!(table.kind, ResourceKind::Events);
        } else {
            panic!("Expected to return to Events view after Esc");
        }

        // 6. Press 'w' again to toggle Warning Triage OFF
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('w'), crossterm::event::KeyModifiers::NONE)).await;
        let toast = app.toast.as_ref().expect("toast after toggling warning triage off");
        assert!(toast.0.contains("Warning Triage: OFF (all 3 events)"));

        if let ActiveView::Table(ref table) = app.active_view {
            assert_eq!(table.filtered_indices.len(), 3);
            assert!(!table.warning_triage);
        }
    }

    #[tokio::test]
    async fn test_crd_dynamic_printer_columns_kubectl_parity() {
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::commands::{resolve_command_with_crds, CrdMeta, PrinterColumn};
        use srelens_tui::views::resource_table::extract_field_str;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            Some("prod-cluster".to_string()),
            Some("prod".to_string()),
            false,
            None,
            vec![],
            tx,
        ).await.unwrap();

        let es_meta = CrdMeta {
            crd_name: "externalsecrets.external-secrets.io".to_string(),
            group: "external-secrets.io".to_string(),
            version: "v1".to_string(),
            kind: "ExternalSecret".to_string(),
            plural: "externalsecrets".to_string(),
            singular: "externalsecret".to_string(),
            namespaced: true,
            short_names: vec!["es".to_string()],
            printer_columns: vec![
                PrinterColumn {
                    name: "StoreType".to_string(),
                    json_path: ".spec.secretStoreRef.kind".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Store".to_string(),
                    json_path: ".spec.secretStoreRef.name".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Refresh Interval".to_string(),
                    json_path: ".spec.refreshInterval".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Status".to_string(),
                    json_path: ".status.conditions[?(@.type==\"Ready\")].reason".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Ready".to_string(),
                    json_path: ".status.conditions[?(@.type==\"Ready\")].status".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Last Sync".to_string(),
                    json_path: ".status.refreshTime".to_string(),
                    col_type: "date".to_string(),
                    priority: 0,
                    description: None,
                },
            ],
        };

        app.crds = vec![es_meta.clone()];

        // Switch to CRD view via command
        let target = resolve_command_with_crds(":es", &app.crds).expect("resolve :es");
        app.execute_view_target(target).await;

        let items = vec![
            serde_json::json!({
                "name": "aip-secrets-binding",
                "namespace": "accommodation-identification-pipeline",
                "spec": {
                    "refreshInterval": "1h",
                    "secretStoreRef": {
                        "kind": "SecretStore",
                        "name": "trv-acc-ident-pipeline-prod"
                    }
                },
                "status": {
                    "conditions": [{ "type": "Ready", "status": "False", "reason": "SecretSyncedError" }]
                },
                "age": "1y"
            }),
            serde_json::json!({
                "name": "harvester-token-binding",
                "namespace": "cluster-autoscaler",
                "spec": {
                    "refreshInterval": "1h",
                    "secretStoreRef": {
                        "kind": "SecretStore",
                        "name": "harvester-token"
                    }
                },
                "status": {
                    "conditions": [{ "type": "Ready", "status": "True", "reason": "SecretSynced" }],
                    "refreshTime": "2026-09-03T09:04:38Z"
                },
                "age": "7m20s"
            }),
        ];

        app.handle_crd_instances_update("crd_instances:ExternalSecret", &serde_json::to_string(&items).unwrap());

        if let ActiveView::Table(ref table) = app.active_view {
            // Verify columns match kubectl get externalsecrets
            let col_names: Vec<&str> = table.columns.iter().map(|c| c.name).collect();
            assert_eq!(
                col_names,
                vec![
                    "NAMESPACE",
                    "NAME",
                    "STORETYPE",
                    "STORE",
                    "REFRESH INTERVAL",
                    "STATUS",
                    "READY",
                    "LAST SYNC",
                    "AGE"
                ]
            );

            assert_eq!(table.filtered_indices.len(), 2);
            assert_eq!(table.raw_items.len(), 2);

            // Test field extraction
            let row0 = &table.raw_items[0];
            assert_eq!(extract_field_str(row0, "printer:.spec.secretStoreRef.kind"), "SecretStore");
            assert_eq!(extract_field_str(row0, "printer:.spec.secretStoreRef.name"), "trv-acc-ident-pipeline-prod");
            assert_eq!(extract_field_str(row0, "printer:.status.conditions[?(@.type==\"Ready\")].reason"), "SecretSyncedError");
            assert_eq!(extract_field_str(row0, "printer:.status.conditions[?(@.type==\"Ready\")].status"), "False");

            let row1 = &table.raw_items[1];
            assert_eq!(extract_field_str(row1, "printer:.spec.secretStoreRef.name"), "harvester-token");
            assert_eq!(extract_field_str(row1, "printer:.status.conditions[?(@.type==\"Ready\")].reason"), "SecretSynced");
            assert_eq!(extract_field_str(row1, "printer:.status.conditions[?(@.type==\"Ready\")].status"), "True");
        } else {
            panic!("Expected ActiveView::Table for ExternalSecret");
        }

        // Apply filter to simulate filtering
        if let ActiveView::Table(ref mut table) = app.active_view {
            table.apply_filter("aip-secrets");
            assert_eq!(table.filtered_indices.len(), 1);
            assert_eq!(table.raw_items.len(), 2);
        }

        // Press Esc to clear filter and restore all items
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE)).await;
        if let srelens_tui::app::ActiveView::Table(ref table) = app.active_view {
            assert_eq!(table.filtered_indices.len(), 2);
            assert_eq!(table.raw_items.len(), 2);
        }
    }

    #[tokio::test]
    async fn test_table_copy_and_bulk_mark_copy() {
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::logs::LogStreamManager;
        use srelens_streams::watch::WatchManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;

        let mut table = ResourceTableState::new(ResourceKind::Pods);
        table.set_items(vec![
            serde_json::json!({
                "metadata": { "name": "pod-1", "namespace": "default" }
            }),
            serde_json::json!({
                "metadata": { "name": "pod-2", "namespace": "default" }
            }),
        ], "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            active_context: "prod".to_string(),
            active_namespace: "default".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: vec![],
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            input_mode: InputMode::Normal,
            toast: None,
            modal: None,
            show_help: false,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            requires_terminal_suspend: None,
            active_log_channel: None,
            last_active_namespace: "default".to_string(),
            is_running: true,
            resource_cache: HashMap::new(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            crds: vec![],
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 1,
            pod_count: 2,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("prod"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Press 'c' -> Copies selected pod name
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('c'), crossterm::event::KeyModifiers::NONE)).await;
        assert!(app.toast.is_some());
        assert!(app.toast.as_ref().unwrap().0.contains("Copied 'pod-1' to clipboard"));

        // 2. Mark both pods with Space and press 'c' -> Copies both names
        if let ActiveView::Table(ref mut t) = app.active_view {
            t.toggle_mark_selected();
            t.select_next();
            t.toggle_mark_selected();
        }
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('c'), crossterm::event::KeyModifiers::NONE)).await;
        assert!(app.toast.is_some());
        assert!(app.toast.as_ref().unwrap().0.contains("Copied 2 resource names to clipboard"));

        // 3. Press 'C' (Shift+c) -> Copies full YAML
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('C'), crossterm::event::KeyModifiers::SHIFT)).await;
        assert!(app.toast.is_some());
        assert!(app.toast.as_ref().unwrap().0.contains("Copied resource YAML to clipboard"));
    }

    #[tokio::test]
    async fn test_yaml_view_mouse_drag_selection_and_copy() {
        use srelens_tui::views::yaml_view::YamlViewState;
        let yaml_text = "apiVersion: v1\nkind: Pod\nmetadata:\n  name: test-pod\nspec:\n  containers: []";
        let mut yaml_state = YamlViewState::new("test-pod".to_string(), "Pod".to_string(), Some("default".to_string()), yaml_text.to_string());

        // 1. Initial state: no selection
        assert!(yaml_state.selected_text().is_none());

        // 2. Start selection at line 1, drag to line 3
        yaml_state.start_selection(1);
        yaml_state.update_selection(3);
        assert!(yaml_state.is_selecting);
        assert_eq!(yaml_state.selection, Some((1, 3)));

        // 3. Finish selection
        let selected = yaml_state.finish_selection(3).expect("selected text");
        assert_eq!(selected, "kind: Pod\nmetadata:\n  name: test-pod");

        // 4. Clear selection
        yaml_state.clear_selection();
        assert!(yaml_state.selection.is_none());
    }

    #[tokio::test]
    async fn test_delete_resource_and_rollout_restart_modal_action_format() {
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::logs::LogStreamManager;
        use srelens_streams::watch::WatchManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::dialogs::Modal;
        use srelens_tui::ui::InputMode;

        let mut table = ResourceTableState::new(ResourceKind::Deployments);
        table.set_items(vec![
            serde_json::json!({
                "metadata": { "name": "nginx-deploy", "namespace": "prod" }
            }),
        ], "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            active_context: "prod".to_string(),
            active_namespace: "prod".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["prod".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: vec![],
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            input_mode: InputMode::Normal,
            toast: None,
            modal: None,
            show_help: false,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx: tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            requires_terminal_suspend: None,
            active_log_channel: None,
            last_active_namespace: "prod".to_string(),
            is_running: true,
            resource_cache: HashMap::new(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            crds: vec![],
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 1,
            pod_count: 2,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("prod"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Press 'Ctrl+d' on Deployment -> Delete confirmation modal
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('d'), crossterm::event::KeyModifiers::CONTROL)).await;
        assert!(app.modal.is_some());
        if let Some(Modal::Confirm { action_name, is_destructive, .. }) = &app.modal {
            assert_eq!(action_name, "delete:Deployment:prod:nginx-deploy");
            assert!(is_destructive);
        } else {
            panic!("Expected Modal::Confirm for delete");
        }

        // Close modal
        app.modal = None;

        // 2. Press 'r' on Deployment -> Restart confirmation modal
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('r'), crossterm::event::KeyModifiers::NONE)).await;
        assert!(app.modal.is_some());
        if let Some(Modal::Confirm { action_name, is_destructive, .. }) = &app.modal {
            assert_eq!(action_name, "restart:Deployment:prod:nginx-deploy");
            assert!(!is_destructive);
        } else {
            panic!("Expected Modal::Confirm for restart");
        }
    }

    /// The age column must recompute from `createdAt` at render time, so a
    /// row whose watch snapshot was taken long ago never shows a stale age.
    #[test]
    fn test_age_recomputed_live_from_created_at() {
        use srelens_tui::views::resource_table::extract_field_str;

        // A pod created ~2 hours ago whose cached age string is stale ("2s").
        let two_hours_ago = srelens_kube::k8s_openapi::jiff::Timestamp::now()
            - std::time::Duration::from_secs(2 * 60 * 60);
        let item = serde_json::json!({
            "name": "web-0",
            "age": "2s",
            "createdAt": two_hours_ago.to_string(),
        });
        assert_eq!(extract_field_str(&item, "age"), "2h");

        // Without createdAt the cached string is all we have — fall through.
        let legacy = serde_json::json!({ "name": "web-0", "age": "70d" });
        assert_eq!(extract_field_str(&legacy, "age"), "70d");

        // An empty createdAt must not shadow the cached age either.
        let empty_ts = serde_json::json!({ "name": "web-0", "age": "5m", "createdAt": "" });
        assert_eq!(extract_field_str(&empty_ts, "age"), "5m");
    }

    /// Raw tabs / ANSI escapes / control chars in a log line desync ratatui's
    /// buffer from the terminal (ghost text after leaving the logs view), so
    /// every pushed line must come out clean.
    #[test]
    fn test_log_lines_sanitized_on_push() {
        use srelens_tui::views::logs_view::{sanitize_log_line, LogsViewState};

        // istio-proxy style tab-delimited line: tabs expand to 8-col stops.
        assert_eq!(
            sanitize_log_line("info\tsds\tStarting"),
            "info    sds     Starting"
        );
        // ANSI color escapes are dropped, carriage returns removed.
        assert_eq!(
            sanitize_log_line("\u{1b}[31merror\u{1b}[0m done\r"),
            "error done"
        );
        // Plain lines pass through untouched.
        assert_eq!(sanitize_log_line("2026-09-03 INFO ok"), "2026-09-03 INFO ok");

        let mut state = LogsViewState::new("p".into(), "ns".into(), None, "ch".into());
        state.push_line("a\tb".to_string());
        assert_eq!(state.lines[0], "a       b");
    }

    /// Any cluster-controlled text rendered as a Span — event messages,
    /// describe output, YAML lines — must be sanitized the same way log
    /// lines are, or embedded tabs/escapes corrupt the terminal.
    #[test]
    fn test_cluster_text_sanitized_in_views() {
        use srelens_tui::views::describe_view::DescribeViewState;
        use srelens_tui::views::sanitize_span_text;
        use srelens_tui::views::yaml_view::YamlViewState;

        // Event-message shaped text: tabs expand, newlines flatten to spaces
        // (table cells are one line tall), escapes and controls are dropped.
        assert_eq!(
            sanitize_span_text("Back-off\trestarting\ncontainer \u{1b}[31mfailed\u{1b}[0m\u{7}"),
            "Back-off        restarting container failed"
        );
        // Plain text takes the fast path untouched.
        assert_eq!(sanitize_span_text("Scaled up replica set"), "Scaled up replica set");

        let desc = DescribeViewState::new(
            "web-0".into(),
            "Pod".into(),
            Some("default".into()),
            "Name:\tweb-0\nMessage: ok\u{1b}[0m".into(),
        );
        assert_eq!(desc.lines, vec!["Name:   web-0", "Message: ok"]);

        let yaml = YamlViewState::new(
            "web-0".into(),
            "Pod".into(),
            Some("default".into()),
            "note: a\tb".into(),
        );
        assert_eq!(yaml.lines, vec!["note: a b"]);
    }

    /// Global drag-to-copy selection: the highlighted screen range must come
    /// back as the visible text (terminal-style linear range, trailing
    /// whitespace trimmed), and the covered cells must be restyled.
    #[test]
    fn test_screen_selection_extracts_visible_text() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::{Position, Rect};
        use ratatui::style::Modifier;
        use srelens_tui::app::apply_screen_selection;

        let mut buf = Buffer::with_lines(vec![
            "istio-system  istio-ingress   ",
            "istio-system  istiod          ",
            "kube-system   coredns         ",
        ]);

        // Drag from row 0 col 14 down to row 1 col 19 (reading order).
        let text = apply_screen_selection(&mut buf, (14, 0), (19, 1));
        assert_eq!(text, "istio-ingress\nistio-system  istiod");
        // Selected cells are reverse-video; unselected ones are not.
        assert!(buf.cell(Position::new(14, 0)).unwrap().style().add_modifier.contains(Modifier::REVERSED));
        assert!(buf.cell(Position::new(0, 1)).unwrap().style().add_modifier.contains(Modifier::REVERSED));
        assert!(!buf.cell(Position::new(0, 0)).unwrap().style().add_modifier.contains(Modifier::REVERSED));
        assert!(!buf.cell(Position::new(0, 2)).unwrap().style().add_modifier.contains(Modifier::REVERSED));

        // A backwards drag (cursor above anchor) selects the same range.
        let mut buf2 = Buffer::with_lines(vec!["abc", "def"]);
        assert_eq!(apply_screen_selection(&mut buf2, (1, 1), (1, 0)), "bc\nde");

        // Out-of-bounds coordinates clamp instead of panicking.
        let mut buf3 = Buffer::with_lines(vec!["xy"]);
        assert_eq!(apply_screen_selection(&mut buf3, (0, 0), (500, 500)), "xy");
        let mut empty = Buffer::empty(Rect::new(0, 0, 0, 0));
        assert_eq!(apply_screen_selection(&mut empty, (0, 0), (5, 5)), "");
    }

    #[tokio::test]
    async fn test_action_palette_generation_and_execution() {
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::logs::LogStreamManager;
        use srelens_streams::watch::WatchManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::ui::dialogs::{Modal, QuickActionId};
        use srelens_tui::ui::InputMode;
        use srelens_tui::views::resource_table::ResourceTableState;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;

        let (event_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let items = vec![serde_json::json!({
            "name": "payment-service",
            "namespace": "prod",
            "replicas": 3,
            "ready": 3,
        })];

        let mut table = ResourceTableState::new(ResourceKind::Deployments);
        table.set_items(items, "");
        table.selected_idx = 0;

        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            active_context: "prod-cluster".to_string(),
            active_namespace: "prod".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["prod".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            event_tx,
            client_cache,
            watch_manager,
            logs_manager,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            active_log_channel: None,
            is_running: true,
            resource_cache: HashMap::new(),
            cluster_name: "prod".to_string(),
            server_url: "https://k8s.example.com".to_string(),
            cluster_version: "1.30.0".to_string(),
            node_count: 10,
            pod_count: 100,
            is_connected: true,
            toast: None,
            requires_terminal_suspend: None,
            crds: vec![],
            last_active_namespace: "prod".to_string(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("prod"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Press 'x' on Deployment -> Opens Action Palette
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('x'), crossterm::event::KeyModifiers::NONE)).await;
        assert!(app.modal.is_some());

        if let Some(Modal::ActionPalette { resource_kind, resource_name, actions, .. }) = &app.modal {
            assert_eq!(resource_kind, "Deployment");
            assert_eq!(resource_name, "payment-service");
            assert!(actions.iter().any(|a| a.id == QuickActionId::AskAi));
            assert!(actions.iter().any(|a| a.id == QuickActionId::RelationshipTree));
            assert!(actions.iter().any(|a| a.id == QuickActionId::RolloutRestart));
            assert!(actions.iter().any(|a| a.id == QuickActionId::Scale));
            assert!(actions.iter().any(|a| a.id == QuickActionId::JumpToPods));
            assert!(actions.iter().any(|a| a.id == QuickActionId::Delete));
        } else {
            panic!("Expected Modal::ActionPalette");
        }

        // 2. Type "rest" to filter actions
        for c in "rest".chars() {
            app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE)).await;
        }

        // 3. Press Enter on filtered "Rollout Restart"
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE)).await;
        assert!(app.modal.is_some());
        if let Some(Modal::Confirm { action_name, is_destructive, .. }) = &app.modal {
            assert_eq!(action_name, "restart:Deployment:prod:payment-service");
            assert!(!is_destructive);
        } else {
            panic!("Expected Modal::Confirm for restart after selecting from action palette");
        }
    }

    #[tokio::test]
    async fn test_resource_relationship_tree_navigation() {
        use srelens_kube::client_cache::ClientCache;
        use srelens_kube::lineage::{LineageNode, LineageRelation};
        use srelens_streams::logs::LogStreamManager;
        use srelens_streams::watch::WatchManager;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::ui::InputMode;
        use srelens_tui::views::resource_table::ResourceTableState;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;

        let (event_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let items = vec![serde_json::json!({
            "name": "cart-api-987-xyz",
            "namespace": "shop",
            "status": "Running",
        })];

        let mut table = ResourceTableState::new(ResourceKind::Pods);
        table.set_items(items, "");
        table.selected_idx = 0;

        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut app = App {
            active_context: "prod-cluster".to_string(),
            active_namespace: "shop".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["shop".to_string()],
            active_view: ActiveView::Table(table),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            event_tx,
            client_cache,
            watch_manager,
            logs_manager,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            active_log_channel: None,
            is_running: true,
            resource_cache: HashMap::new(),
            cluster_name: "prod".to_string(),
            server_url: "https://k8s.example.com".to_string(),
            cluster_version: "1.30.0".to_string(),
            node_count: 10,
            pod_count: 100,
            is_connected: true,
            toast: None,
            requires_terminal_suspend: None,
            crds: vec![],
            last_active_namespace: "shop".to_string(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("shop"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Press 't' on Pod -> Opens Tree View
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('t'), crossterm::event::KeyModifiers::NONE)).await;
        assert!(matches!(app.active_view, ActiveView::Tree(_)));

        // 2. Simulate lineage resolution result delivery
        let mut root = LineageNode::new("Deployment", "cart-api", Some("shop".into()), LineageRelation::Owner);
        let mut rs = LineageNode::new("ReplicaSet", "cart-api-987", Some("shop".into()), LineageRelation::Owner);
        let pod = LineageNode::new("Pod", "cart-api-987-xyz", Some("shop".into()), LineageRelation::Target);
        rs.children.push(pod);
        root.children.push(rs);

        app.handle_lineage_result("Pod", "cart-api-987-xyz", Ok(root));

        if let ActiveView::Tree(tree) = &app.active_view {
            assert_eq!(tree.nodes.len(), 3);
            assert_eq!(tree.selected_node().unwrap().name, "cart-api-987-xyz");
            let summary = tree.tree_as_text();
            assert!(summary.contains("Deployment/cart-api"));
            assert!(summary.contains("ReplicaSet/cart-api-987"));
            assert!(summary.contains("Pod/cart-api-987-xyz"));
        } else {
            panic!("Expected ActiveView::Tree");
        }

        // 3. Press 'Esc' to pop back to Table view
        app.handle_key_event(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE)).await;
        assert!(matches!(app.active_view, ActiveView::Table(_)));
    }

    #[tokio::test]
    async fn test_node_inspector_enter_navigation_and_pod_jump() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::{InputMode, Modal};
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::views::resource_table::ResourceTableState;
        use srelens_kube::node_inspector::{NodeInspectorDetails, NodePodItem, NodeConditionInfo, NodeTaintInfo};

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut node_table = ResourceTableState::new(ResourceKind::Nodes);
        let node_json = serde_json::json!({
            "name": "gpu-node-alpha",
            "status": "Ready",
            "roles": "worker",
            "version": "v1.30.2"
        });
        node_table.set_items(vec![node_json], "");

        let mut app = App {
            active_context: "prod".to_string(),
            active_namespace: "default".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["default".to_string(), "ai-prod".to_string()],
            active_view: ActiveView::Table(node_table),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            event_tx: tx,
            client_cache,
            watch_manager,
            logs_manager,
            resource_cache: HashMap::new(),
            active_log_channel: None,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            crds: vec![],
            last_active_namespace: "default".to_string(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.2".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 10,
            pod_count: 80,
            is_connected: true,
            toast: None,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("default"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Pressing 'x' on Node table opens Action Palette with 'InspectNode'
        app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)).await;
        if let Some(Modal::ActionPalette { actions, .. }) = &app.modal {
            assert!(actions.iter().any(|a| a.id == srelens_tui::ui::dialogs::QuickActionId::InspectNode));
        } else {
            panic!("Expected ActionPalette modal");
        }
        app.modal = None;

        // 2. Pressing Enter on 'gpu-node-alpha' opens Node Inspector
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert!(matches!(app.active_view, ActiveView::NodeInspector(_)));

        // 3. Deliver Node Inspector results with GPU details
        let mock_details = NodeInspectorDetails {
            name: "gpu-node-alpha".to_string(),
            status: "Ready".to_string(),
            unschedulable: false,
            roles: "worker".to_string(),
            instance_type: "g4dn.2xlarge".to_string(),
            zone: Some("eu-west-1b".to_string()),
            region: Some("eu-west-1".to_string()),
            nodepool: Some("gpu-pool".to_string()),
            internal_ip: Some("10.0.1.50".to_string()),
            external_ip: None,
            os_image: "Ubuntu 22.04".to_string(),
            kernel_version: "5.15.0".to_string(),
            container_runtime: "containerd".to_string(),
            kubelet_version: "v1.30.2".to_string(),
            architecture: "amd64".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            cpu_capacity_millicores: 8000,
            cpu_allocatable_millicores: 7800,
            cpu_requests_millicores: 2500,
            mem_capacity_mib: 32768,
            mem_allocatable_mib: 31000,
            mem_requests_mib: 14000,
            pods_capacity: 110,
            pods_allocatable: 110,
            pods_count: 2,
            has_gpu: true,
            gpu_model: Some("Tesla T4".to_string()),
            gpu_driver_version: Some("535.129".to_string()),
            gpu_cuda_version: Some("12.2".to_string()),
            gpu_capacity_count: 1,
            gpu_allocatable_count: 1,
            gpu_requests_count: 1,
            gpu_memory_total_mib: Some(15360),
            gpu_memory_requests_mib: 7168,
            conditions: vec![
                NodeConditionInfo {
                    type_: "Ready".to_string(),
                    status: "True".to_string(),
                    reason: None,
                    message: None,
                },
            ],
            taints: vec![
                NodeTaintInfo {
                    key: "nvidia.com/gpu".to_string(),
                    value: Some("present".to_string()),
                    effect: "NoSchedule".to_string(),
                }
            ],
            pods: vec![
                NodePodItem {
                    name: "vllm-serve-7b".to_string(),
                    namespace: "ai-prod".to_string(),
                    phase: "Running".to_string(),
                    ready_containers: "1/1".to_string(),
                    restarts: 0,
                    age: "3d".to_string(),
                    cpu_requests_millicores: 2000,
                    mem_requests_mib: 12000,
                    gpu_requests: 1,
                    gpu_mem_requests_mib: 7168,
                    pod_ip: "10.244.1.5".to_string(),
                },
                NodePodItem {
                    name: "node-exporter".to_string(),
                    namespace: "monitoring".to_string(),
                    phase: "Running".to_string(),
                    ready_containers: "1/1".to_string(),
                    restarts: 0,
                    age: "10d".to_string(),
                    cpu_requests_millicores: 100,
                    mem_requests_mib: 128,
                    gpu_requests: 0,
                    gpu_mem_requests_mib: 0,
                    pod_ip: "10.244.1.6".to_string(),
                },
            ],
        };

        app.handle_node_inspector_result("gpu-node-alpha", Ok(mock_details));

        if let ActiveView::NodeInspector(ni) = &app.active_view {
            assert_eq!(ni.node_name, "gpu-node-alpha");
            let d = ni.details.as_ref().unwrap();
            assert!(d.has_gpu);
            assert_eq!(d.gpu_model.as_deref(), Some("Tesla T4"));
            assert_eq!(d.pods.len(), 2);
            assert_eq!(d.pods[0].name, "vllm-serve-7b");
            assert_eq!(d.pods[0].gpu_requests, 1);
        } else {
            panic!("Expected ActiveView::NodeInspector");
        }

        // 4. Pressing Enter on selected pod 'vllm-serve-7b' jumps to Pods table
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert!(matches!(app.active_view, ActiveView::Table(_)));
        if let ActiveView::Table(t) = &app.active_view {
            assert_eq!(t.kind, ResourceKind::Pods);
        }
        assert_eq!(app.active_namespace, "ai-prod");
        assert_eq!(app.filter_buffer, "vllm-serve-7b");
    }

    #[tokio::test]
    async fn test_mouse_row_selection_and_scrolling_in_table_and_node_inspector() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Rect;
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::commands::ResourceKind;
        use srelens_tui::views::resource_table::ResourceTableState;
        use srelens_kube::node_inspector::{NodeInspectorDetails, NodePodItem};

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let mut node_table = ResourceTableState::new(ResourceKind::Nodes);
        let mut items = Vec::new();
        for i in 0..20 {
            items.push(serde_json::json!({
                "name": format!("node-{}", i),
                "status": "Ready",
                "podIp": format!("10.0.0.{}", i),
            }));
        }
        node_table.set_items(items, "");
        // Simulate rendered table viewport at y=3, height=20
        node_table.last_viewport_rect.set(Rect::new(0, 3, 100, 20));
        node_table.last_start_idx.set(0);

        let mut app = App {
            active_context: "prod".to_string(),
            active_namespace: "default".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Table(node_table),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            event_tx: tx,
            client_cache,
            watch_manager,
            logs_manager,
            resource_cache: HashMap::new(),
            active_log_channel: None,
            current_watch_channel: None,
            active_watch_channels: std::collections::HashSet::new(),
            active_watch_pool: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            crds: vec![],
            last_active_namespace: "default".to_string(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.2".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 20,
            pod_count: 50,
            is_connected: true,
            toast: None,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("default"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. Click on row at y = 3 + 2 + 5 = 10 -> row 5
        let click_row_5 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 15,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(click_row_5).await;
        if let ActiveView::Table(t) = &app.active_view {
            assert_eq!(t.selected_idx, 5);
        }

        // 2. Mouse ScrollDown -> moves forward by 3
        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 15,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(scroll_down).await;
        if let ActiveView::Table(t) = &app.active_view {
            assert_eq!(t.selected_idx, 8);
        }

        // 3. Mouse ScrollUp -> moves back by 3
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 15,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(scroll_up).await;
        if let ActiveView::Table(t) = &app.active_view {
            assert_eq!(t.selected_idx, 5);
        }

        // 4. Test Node Inspector mouse selection
        let mut ni_state = srelens_tui::views::node_inspector_view::NodeInspectorState::new("node-1".to_string());
        let mock_pods = vec![
            NodePodItem {
                name: "pod-0".to_string(),
                namespace: "default".to_string(),
                phase: "Running".to_string(),
                ready_containers: "1/1".to_string(),
                restarts: 0,
                age: "1d".to_string(),
                cpu_requests_millicores: 100,
                mem_requests_mib: 128,
                gpu_requests: 0,
                gpu_mem_requests_mib: 0,
                pod_ip: "10.0.0.1".to_string(),
            },
            NodePodItem {
                name: "pod-1".to_string(),
                namespace: "default".to_string(),
                phase: "Running".to_string(),
                ready_containers: "1/1".to_string(),
                restarts: 0,
                age: "1d".to_string(),
                cpu_requests_millicores: 200,
                mem_requests_mib: 256,
                gpu_requests: 0,
                gpu_mem_requests_mib: 0,
                pod_ip: "10.0.0.2".to_string(),
            },
        ];
        let details = NodeInspectorDetails {
            name: "node-1".to_string(),
            status: "Ready".to_string(),
            unschedulable: false,
            roles: "worker".to_string(),
            instance_type: "m5.large".to_string(),
            zone: None,
            region: None,
            nodepool: None,
            internal_ip: None,
            external_ip: None,
            os_image: "Ubuntu".to_string(),
            kernel_version: "5.15".to_string(),
            container_runtime: "containerd".to_string(),
            kubelet_version: "v1.30.0".to_string(),
            architecture: "amd64".to_string(),
            created_at: "".to_string(),
            cpu_capacity_millicores: 2000,
            cpu_allocatable_millicores: 1900,
            cpu_requests_millicores: 300,
            mem_capacity_mib: 8192,
            mem_allocatable_mib: 8000,
            mem_requests_mib: 384,
            pods_capacity: 110,
            pods_allocatable: 110,
            pods_count: 2,
            has_gpu: false,
            gpu_model: None,
            gpu_driver_version: None,
            gpu_cuda_version: None,
            gpu_capacity_count: 0,
            gpu_allocatable_count: 0,
            gpu_requests_count: 0,
            gpu_memory_total_mib: None,
            gpu_memory_requests_mib: 0,
            conditions: vec![],
            taints: vec![],
            pods: mock_pods,
        };
        ni_state.set_details(details);
        ni_state.last_pods_table_rect.set(Rect::new(0, 10, 100, 15));
        ni_state.last_scroll_offset.set(0);
        app.active_view = ActiveView::NodeInspector(ni_state);

        // Click on second pod (header is row 10, data starts at 11, so row 12 is offset 1)
        let click_pod_1 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 15,
            row: 12,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(click_pod_1).await;
        if let ActiveView::NodeInspector(ni) = &app.active_view {
            assert_eq!(ni.selected_pod_idx, 1);
        }
    }

    #[tokio::test]
    async fn test_text_search_in_describe_yaml_and_logs() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::InputMode;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::mpsc::unbounded_channel;
        use srelens_kube::client_cache::ClientCache;
        use srelens_streams::watch::WatchManager;
        use srelens_streams::logs::LogStreamManager;
        use srelens_tui::views::describe_view::DescribeViewState;
        use srelens_tui::views::yaml_view::YamlViewState;
        use srelens_tui::views::logs_view::LogsViewState;

        let (tx, _rx) = unbounded_channel();
        let client_cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        let desc_text = "Name: my-pod\nNamespace: default\nContainers:\n  app:\n    Image: nginx:latest\nEvents:\n  Type: Normal\n  Reason: Started";
        let desc_view = DescribeViewState::new("my-pod".to_string(), "Pod".to_string(), Some("default".to_string()), desc_text.to_string());

        let mut app = App {
            active_context: "prod".to_string(),
            active_namespace: "default".to_string(),
            kubeconfig_paths: vec![],
            contexts: vec![],
            namespaces: vec!["default".to_string()],
            active_view: ActiveView::Describe(desc_view),
            nav_stack: vec![],
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            event_tx: tx,
            client_cache,
            watch_manager,
            logs_manager,
            resource_cache: HashMap::new(),
            active_log_channel: None,
            current_watch_channel: None,
            active_watch_channels: std::collections::HashSet::new(),
            active_watch_pool: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            crds: vec![],
            last_active_namespace: "default".to_string(),
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "v1.30.2".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 10,
            is_connected: true,
            toast: None,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::for_context("default"),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        // 1. In Describe mode: press '/' to enter search mode
        app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Filter);

        // Type "normal"
        for c in "normal".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        assert_eq!(app.filter_buffer, "normal");

        if let ActiveView::Describe(d) = &app.active_view {
            assert_eq!(d.search_query, "normal");
            assert_eq!(d.search_matches.len(), 1);
            assert_eq!(d.scroll_offset, 6); // Line 6 contains "Type: Normal"
        }

        // Press Enter to finalize search and return to Normal mode
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Normal);

        // Press Esc to clear search
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        if let ActiveView::Describe(d) = &app.active_view {
            assert!(d.search_query.is_empty());
        }

        // 2. In YAML mode: test '/' search and n/N cycling
        let yaml_text = "apiVersion: v1\nkind: Service\nmetadata:\n  name: my-svc\nspec:\n  ports:\n  - port: 80\n    targetPort: 8080\n  - port: 443\n    targetPort: 8443";
        let yaml_view = YamlViewState::new("my-svc".to_string(), "Service".to_string(), Some("default".to_string()), yaml_text.to_string());
        app.active_view = ActiveView::Yaml(yaml_view);

        // Press '/' and search "port"
        app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Filter);
        for c in "port".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        if let ActiveView::Yaml(y) = &app.active_view {
            assert_eq!(y.search_matches.len(), 5); // ports, port 80, targetPort 8080, port 443, targetPort 8443
            assert_eq!(y.current_match_idx, Some(0));
        }

        // Press Enter to lock search
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert_eq!(app.input_mode, InputMode::Normal);

        // Press 'n' to go to next match
        app.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)).await;
        if let ActiveView::Yaml(y) = &app.active_view {
            assert_eq!(y.current_match_idx, Some(1));
        }

        // Press 'N' to go back to prev match
        app.handle_key_event(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE)).await;
        if let ActiveView::Yaml(y) = &app.active_view {
            assert_eq!(y.current_match_idx, Some(0));
        }

        // 3. In Logs mode: test '/' search
        let mut logs_view = LogsViewState::new("my-pod".to_string(), "default".to_string(), None, "chan-1".to_string());
        logs_view.push_line("INFO Server listening on :8080".to_string());
        logs_view.push_line("WARN High memory pressure detected".to_string());
        logs_view.push_line("ERROR Connection reset by peer".to_string());
        app.active_view = ActiveView::Logs(logs_view);

        app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)).await;
        for c in "error".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        }
        if let ActiveView::Logs(l) = &app.active_view {
            assert_eq!(l.search_matches.len(), 1);
            assert_eq!(l.scroll_offset, 2); // 3rd line has ERROR
        }
    }

    #[tokio::test]
    async fn test_slash_commands_and_ai_playbooks() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ai_skills::{expand_slash_command, match_slash_commands};

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            Some("test-cluster".to_string()),
            Some("production".to_string()),
            false,
            None,
            vec![],
            tx,
        ).await.unwrap();

        app.active_view = ActiveView::Assistant;
        app.active_context = "test-cluster".to_string();
        app.active_namespace = "production".to_string();

        // 1. Typing '/' triggers slash suggestions popup
        app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.input, "/");
        assert!(!app.assistant_state.slash_suggestions.is_empty());
        assert!(app.assistant_state.slash_suggestions.len() >= 9);

        // 2. Typing 'c' then 'r' narrows suggestions to crashloop
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)).await;
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.input, "/cr");
        assert_eq!(app.assistant_state.slash_suggestions.len(), 1);
        assert_eq!(app.assistant_state.slash_suggestions[0].command, "crashloop");

        // 3. Pressing Tab applies the suggestion with trailing space
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.input, "/crashloop ");
        assert!(app.assistant_state.slash_suggestions.is_empty());

        // 4. Test Up / Down navigation when suggestions are active
        app.assistant_state.input = "/".to_string();
        app.assistant_state.update_slash_suggestions();
        assert_eq!(app.assistant_state.slash_suggestion_idx, 0);

        app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.slash_suggestion_idx, 1);

        app.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.slash_suggestion_idx, 0);

        // 5. Pressing Esc closes suggestions popup without leaving Assistant view
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert!(app.assistant_state.slash_suggestions.is_empty());
        assert!(matches!(app.active_view, ActiveView::Assistant));

        // 6. Test Playbook expansion (targeted vs discovery fallback)
        let targeted = expand_slash_command("crashloop", Some("api-gateway-7f"), "test-cluster", "production").unwrap();
        assert!(targeted.contains("Focus on Pod 'api-gateway-7f' in namespace 'production'"));
        assert!(targeted.contains("PREVIOUS container's logs"));

        let discovery = expand_slash_command("crashloop", None, "test-cluster", "production").unwrap();
        assert!(discovery.contains("Scan namespace 'production' for any pods experiencing this issue"));

        let node_playbook = expand_slash_command("nodepressure", None, "test-cluster", "").unwrap();
        assert!(node_playbook.contains("MemoryPressure, DiskPressure, PIDPressure"));

        let cluster_briefing = expand_slash_command("summarise", None, "prod-cluster", "").unwrap();
        assert!(cluster_briefing.contains("executive briefing"));

        // 7. Test utility command execution via Enter
        app.assistant_state.add_assistant_message("Previous message".to_string());
        assert!(!app.assistant_state.messages.is_empty());

        app.assistant_state.input = "/clear".to_string();
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert_eq!(app.assistant_state.messages.len(), 1);
        assert!(app.assistant_state.messages[0].content.contains("Hello! I am your SRElens AI Assistant"));
        assert_eq!(app.assistant_state.input, "");
    }

    #[tokio::test]
    async fn test_action_palette_playbooks() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use srelens_tui::app::{ActiveView, App};
        use srelens_tui::ui::dialogs::{Modal, QuickActionId};

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            Some("test-cluster".to_string()),
            Some("default".to_string()),
            false,
            None,
            vec![],
            tx,
        ).await.unwrap();

        // 1. Pod palette has PlaybookCrashLoop, PlaybookPending, PlaybookOom
        app.open_action_palette("Pod".to_string(), "my-failing-pod".to_string(), Some("default".to_string()));
        if let Some(Modal::ActionPalette { actions, .. }) = &app.modal {
            assert!(actions.iter().any(|a| a.id == QuickActionId::PlaybookCrashLoop));
            assert!(actions.iter().any(|a| a.id == QuickActionId::PlaybookPending));
            assert!(actions.iter().any(|a| a.id == QuickActionId::PlaybookOom));
        } else {
            panic!("Expected ActionPalette modal");
        }

        // 2. Select PlaybookCrashLoop and execute
        if let Some(Modal::ActionPalette { ref mut selected_idx, ref actions, .. }) = app.modal {
            let idx = actions.iter().position(|a| a.id == QuickActionId::PlaybookCrashLoop).unwrap();
            *selected_idx = idx;
        }
        app.execute_action_palette().await;

        assert!(matches!(app.active_view, ActiveView::Assistant));
        assert_eq!(app.assistant_state.input, "/crashloop my-failing-pod");

        // 3. Workload palette has PlaybookRollout
        app.open_action_palette("Deployment".to_string(), "web-app".to_string(), Some("prod".to_string()));
        if let Some(Modal::ActionPalette { actions, .. }) = &app.modal {
            assert!(actions.iter().any(|a| a.id == QuickActionId::PlaybookRollout));
        }

        // 4. Service palette has PlaybookEndpoints
        app.open_action_palette("Service".to_string(), "api-svc".to_string(), Some("prod".to_string()));
        if let Some(Modal::ActionPalette { actions, .. }) = &app.modal {
            assert!(actions.iter().any(|a| a.id == QuickActionId::PlaybookEndpoints));
        }

        // 5. Node palette has PlaybookNodePressure
        app.open_action_palette("Node".to_string(), "worker-01".to_string(), None);
        if let Some(Modal::ActionPalette { actions, .. }) = &app.modal {
            assert!(actions.iter().any(|a| a.id == QuickActionId::PlaybookNodePressure));
        }
    }
}
