//! Shared helpers for the TUI's integration tests.
//!
//! Every test file under `tests/` is its own crate; `mod common;` pulls this
//! in. Nothing here touches a cluster: `App::new` with no kubeconfig paths
//! resolves zero contexts, falls back to a `default` context, and every
//! capability call fails fast with "context not found" instead of hanging.

#![allow(dead_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::{Frame, Terminal};
use srelens_tui::app::App;
use srelens_tui::event::AppEvent;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// An `App` with no kubeconfig, no cluster, and the given context/namespace,
/// plus the receiving end of its event channel so a test can observe what the
/// app emitted.
pub async fn app_with(context: &str, namespace: &str) -> (App, UnboundedReceiver<AppEvent>) {
    let (tx, rx) = unbounded_channel();
    let app = App::new(
        Some(context.to_string()),
        Some(namespace.to_string()),
        false,
        None,
        vec![],
        tx,
    )
    .await
    .expect("App::new never needs a cluster");
    (app, rx)
}

/// `app_with("test-cluster", "default")`.
pub async fn app() -> (App, UnboundedReceiver<AppEvent>) {
    app_with("test-cluster", "default").await
}

/// A plain key press.
pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A character key press.
pub fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

/// A key press with Ctrl held.
pub fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// A key press with Shift held.
pub fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

/// Type a string one character at a time through `App::handle_key_event`.
pub async fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key_event(ch(c)).await;
    }
}

/// A left-button mouse event of the given kind at a cell.
pub fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// A left click at a cell.
pub fn click(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

/// Render one frame at the given size through any drawing closure and return
/// the screen as text, one `String` per row, trailing spaces trimmed.
pub fn render_lines<F>(width: u16, height: u16, draw: F) -> Vec<String>
where
    F: FnOnce(&mut Frame),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(draw).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            line.trim_end().to_string()
        })
        .collect()
}

/// `render_lines` joined with newlines, for `contains` assertions.
pub fn render_text<F>(width: u16, height: u16, draw: F) -> String
where
    F: FnOnce(&mut Frame),
{
    render_lines(width, height, draw).join("\n")
}

/// Render the whole app at the given size and return the screen text.
pub fn render_app(app: &mut App, width: u16, height: u16) -> String {
    render_text(width, height, |f| app.render(f))
}

/// Drain every event the app has emitted so far.
pub fn drain(rx: &mut UnboundedReceiver<AppEvent>) -> Vec<AppEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}
