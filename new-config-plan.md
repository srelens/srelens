# TUI `:config` menu and live command-popup size

## Summary

Command mode (`:`) draws a suggestions popup with hardcoded caps (65 max columns, 6 visible rows). On modern terminal displays, this is often too constrained. Terminals cannot change font size per widget, so sizing the `:` popup means controlling **the popup's column width and visible row count**.

This feature introduces:
1. `TuiConfig` persisted in `~/.config/srelens/tui.json` (`$SRELENS_CONFIG_DIR/tui.json`, or overridden by `SRELENS_TUI_CONFIG_PATH`).
2. Driving the statusbar command popup geometry from `TuiConfig`.
3. A dedicated `:config` view (`ResourceKind::TuiConfig`) featuring live adjustments with `j`/`k`/`h`/`l`/`-`/`+`/arrows and an embedded live preview of the command popup.
4. Reclaiming `:config` for TUI chrome settings while keeping AI settings accessible on `:ai-settings` (and `:settings`).

## Persistence & Model

New module `apps/tui/src/tui_config.rs` exported from `apps/tui/src/lib.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TuiConfig {
    pub command_popup_max_width: u16,    // default 65, clamped 40..=200
    pub command_popup_max_visible: usize, // default 6, clamped 3..=20
}
```

Path resolution order:
1. `SRELENS_TUI_CONFIG_PATH` if set (for isolated tests)
2. else `$SRELENS_CONFIG_DIR/tui.json`
3. else `dirs::config_dir()/srelens/tui.json`

`TuiConfig::load()` returns clamped values or defaults if missing/corrupt.
`TuiConfig::save()` writes pretty JSON atomically or directly to the resolved path.

## Popup Geometry

Extracted into `command_popup_rect(area: Rect, item_count: usize, max_width: u16, max_visible: usize) -> Rect`:
- `visible_count = item_count.min(max_visible)`
- `popup_height = visible_count as u16 + 2`
- `popup_width = area.width.saturating_sub(4).min(max_width)`
- `Rect { x: area.x + 2, y: area.y.saturating_sub(popup_height), width: popup_width, height: popup_height }`

Passed to statusbar via `StatusBarProps` and `App.tui_config`.

## `:config` View with Live Preview

- `ResourceKind::TuiConfig` added to `commands.rs`.
- `COMMAND_REGISTRY` registers primary command `config` with aliases `["tui-config", "tui"]`.
- Removed `config` from `ai-settings` aliases.
- View rendered in `apps/tui/src/views/tui_config_view.rs`:
  - Setting 0: Command Popup Max Width [40..=200]
  - Setting 1: Command Popup Max Visible Rows [3..=20]
  - Live preview widget rendering sample command suggestions using the current width and row count.
  - Value adjustments immediately update `App.tui_config` and call `save()`.
