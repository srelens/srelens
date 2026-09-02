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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.30.0".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 5,
            pod_count: 50,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
            cluster_version: "v1.31.7".to_string(),
            cluster_name: "prod".to_string(),
            server_url: "https://127.0.0.1:6443".to_string(),
            node_count: 32,
            pod_count: 1,
            is_connected: true,
            ai_settings: srelens_tui::AiSettings::default(),
            assistant_state: srelens_tui::views::assistant_view::AssistantViewState::new(),
            pod_metrics_tick_counter: 0,
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
}
