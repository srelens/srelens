use crossterm::event::KeyCode;
use srectl::theme::Theme;

use crate::common::{self, ch, key, press, render_app, type_str};

/// D6: Truthfulness & Feedback
/// The NOC Telemetry Deck header must never display a false "ONLINE" status
/// when a cluster connection fails or is disconnected. It must say "OFFLINE".
#[tokio::test]
async fn journey_d6_truthfulness_noc_offline_vs_online() {
    let _settings = common::env::isolate_settings();
    let mut theme = common::theme::lock();
    theme.set_by_name("noc-deck");

    let (mut app, _rx) = common::app().await;

    // Simulate disconnected state
    app.is_connected = false;
    let screen_disconnected = render_app(&mut app, 120, 24);
    assert!(
        screen_disconnected.contains("OFFLINE"),
        "Disconnected cluster must display OFFLINE in NOC telemetry header"
    );
    assert!(
        !screen_disconnected.contains("ONLINE //"),
        "Disconnected cluster must not display ONLINE"
    );

    // Simulate connected state
    app.is_connected = true;
    let screen_connected = render_app(&mut app, 120, 24);
    assert!(
        screen_connected.contains("ONLINE"),
        "Connected cluster must display ONLINE in NOC telemetry header"
    );
}

/// D7: Visual & Perceptual Reality on 80x24 Terminal
/// On an 80-column terminal, the active view (e.g. "Pods") must not be clipped
/// or hidden behind navigation hints in Turbo Vision and NOC Telemetry Deck themes.
#[tokio::test]
async fn journey_d7_perceptual_narrow_80x24_header_renders_active_view() {
    let _settings = common::env::isolate_settings();
    let mut theme = common::theme::lock();

    let (mut app, _rx) = common::app().await;

    // 1. Verify Turbo Vision layout on 80x24 terminal
    theme.set_by_name("turbo-vision");
    let screen_tv_80x24 = render_app(&mut app, 80, 24);
    assert!(
        screen_tv_80x24.contains("VIEW: Pods"),
        "Turbo Vision on 80-column terminal must render active view name without truncation"
    );
    assert!(
        screen_tv_80x24.contains("SRELENS: CONTROL"),
        "Turbo Vision framed title banner must be visible"
    );

    // 2. Verify NOC Telemetry Deck layout on 80x24 terminal
    theme.set_by_name("noc-deck");
    let screen_noc_80x24 = render_app(&mut app, 80, 24);
    assert!(
        screen_noc_80x24.contains("SURFACE: PODS"),
        "NOC Telemetry Deck on 80-column terminal must render active surface name without truncation"
    );
    assert!(
        screen_noc_80x24.contains("SRELENS // NOC"),
        "NOC brand header must be visible"
    );
}

/// D1: Input Disorder & Scrolling in Theme Picker
/// On a compact 24-row terminal, the theme picker list only has ~15 visible rows.
/// Navigating to Turbo Vision (index 15) and NOC Deck (index 16) must scroll them
/// into view with their descriptive visual badges.
#[tokio::test]
async fn journey_d1_theme_picker_scrolls_to_new_themes_on_narrow_terminal() {
    let _settings = common::env::isolate_settings();
    let _theme = common::theme::lock();
    let (mut app, _rx) = common::app().await;

    // Open theme picker
    press(&mut app, ch(':')).await;
    type_str(&mut app, "theme").await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Scroll down to Turbo Vision (index 15)
    for _ in 0..15 {
        press(&mut app, key(KeyCode::Down)).await;
    }

    let screen_tv = render_app(&mut app, 80, 24);
    assert!(
        screen_tv.contains("Turbo Vision"),
        "Theme picker must scroll to make Turbo Vision visible on 24-row terminal"
    );
    assert!(
        screen_tv.contains("[Double Frames]"),
        "Theme picker must show Turbo Vision descriptive badge"
    );

    // Scroll down to NOC Deck (index 16)
    press(&mut app, key(KeyCode::Down)).await;
    let screen_noc = render_app(&mut app, 80, 24);
    assert!(
        screen_noc.contains("NOC Telemetry Deck"),
        "Theme picker must scroll to make NOC Telemetry Deck visible"
    );
    assert!(
        screen_noc.contains("[Hairlines • UTC]"),
        "Theme picker must show NOC Telemetry descriptive badge"
    );

    // Close picker with Esc
    press(&mut app, key(KeyCode::Esc)).await;
}

/// D7: Tactile Action Buttons in Turbo Vision
/// The status bar in Turbo Vision mode must render inverted button blocks
/// for real shortcuts instead of hardcoded non-functional hotkeys.
#[tokio::test]
async fn journey_d7_turbo_vision_tactile_action_buttons() {
    let _settings = common::env::isolate_settings();
    let mut theme = common::theme::lock();
    theme.set_by_name("turbo-vision");

    let (mut app, _rx) = common::app().await;

    let screen = render_app(&mut app, 120, 24);
    assert!(
        screen.contains("Cmd") && screen.contains("Filter"),
        "Status bar must render actual command and filter action buttons"
    );
    assert_eq!(Theme::border_type(), ratatui::widgets::BorderType::Double);
}
