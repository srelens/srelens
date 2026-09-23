//! Keyboard input must not wait for a failed or stalled cluster connection.
mod common;

use std::time::Duration;

use crossterm::event::KeyCode;
use srelens_tui::{app::App, event::AppEvent, ui::Modal};

async fn recovery_controls_remain_responsive(stalled: bool) {
    let _settings = common::env::isolate_settings();
    // Dropping the listener models a deleted cluster. Keeping it open without
    // answering models a stopped Docker VM whose API endpoint never replies.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let _listener = if stalled {
        Some(listener)
    } else {
        drop(listener);
        None
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut config = serde_json::json!({
        "apiVersion": "v1", "kind": "Config", "current-context": "offline",
        "clusters": [{"name": "test", "cluster": {"server": format!("http://{address}")}}],
        "contexts": [
            {"name": "offline", "context": {"cluster": "test", "user": "test"}},
            {"name": "replacement", "context": {"cluster": "test", "user": "test"}}
        ],
        "users": [{"name": "test", "user": {"token": "fake-test-token"}}]
    });
    std::fs::write(&path, config.to_string()).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        Some("offline".into()),
        None,
        true,
        None,
        vec![path.clone()],
        tx.clone(),
    )
    .await
    .unwrap();
    app.cluster_unreachable = true;
    // Let the real background watch encounter the endpoint before queuing a
    // key on the same channel used by the terminal's input reader.
    tokio::time::sleep(Duration::from_millis(200)).await;
    if !stalled {
        config["contexts"].as_array_mut().unwrap().remove(0);
        config["current-context"] = "replacement".into();
        std::fs::write(&path, config.to_string()).unwrap();
    }
    tx.send(AppEvent::Key(common::ctrl('x'))).unwrap();
    let mut reached_key = false;
    for _ in 0..32 {
        match rx.try_recv().expect("the queued key must be available") {
            AppEvent::Key(key) => {
                tokio::time::timeout(Duration::from_millis(500), app.handle_key_event(key))
                    .await
                    .expect("opening the picker must not await the cluster");
                reached_key = true;
                break;
            }
            AppEvent::StreamEvent { channel, payload } => app.handle_stream_event(channel, payload),
            _ => {}
        }
    }
    assert!(
        reached_key,
        "watch failures must not flood the queue ahead of keyboard input"
    );
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));
    assert!(common::render_app(&mut app, 140, 40).contains("replacement"));

    tokio::time::timeout(Duration::from_millis(500), async {
        common::type_str(&mut app, "replacement").await;
        app.handle_key_event(common::key(KeyCode::Enter)).await;
        assert_eq!(app.active_context, "replacement");
        assert!(app.modal.is_none());
        app.handle_key_event(common::ch('?')).await;
        assert!(app.show_help);
        app.handle_key_event(common::key(KeyCode::Esc)).await;
        app.handle_key_event(common::ch(':')).await;
        common::type_str(&mut app, "ctx").await;
        app.handle_key_event(common::key(KeyCode::Enter)).await;
        assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));
        app.handle_key_event(common::key(KeyCode::Esc)).await;
        app.handle_key_event(common::ctrl('c')).await;
        assert!(!app.is_running);
    })
    .await
    .expect("switching context, help, commands and exit must remain responsive");
    app.watch_manager.shutdown_all();
}

#[tokio::test]
async fn deleted_cluster_does_not_starve_keyboard_input() {
    recovery_controls_remain_responsive(false).await;
}

#[tokio::test]
async fn stalled_cluster_does_not_block_keyboard_input() {
    recovery_controls_remain_responsive(true).await;
}
