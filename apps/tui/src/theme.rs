use ratatui::style::{Color, Modifier, Style};

/// SRElens TUI Theme palette
pub struct Theme;

impl Theme {
    pub const BG: Color = Color::Reset;
    pub const FG: Color = Color::Rgb(220, 224, 232);
    pub const DIM: Color = Color::Rgb(110, 115, 135);
    pub const ACCENT: Color = Color::Rgb(139, 92, 246);      // Purple / Violet (SRElens brand)
    pub const CYAN: Color = Color::Rgb(56, 189, 248);        // Sky blue
    pub const GREEN: Color = Color::Rgb(34, 197, 94);        // Success / Running
    pub const YELLOW: Color = Color::Rgb(234, 179, 8);       // Warning / Pending
    pub const RED: Color = Color::Rgb(239, 68, 68);          // Error / CrashLoop / Failed
    pub const ORANGE: Color = Color::Rgb(249, 115, 22);      // Terminating / Evicted
    pub const BORDER: Color = Color::Rgb(75, 85, 99);        // Border gray
    pub const BORDER_FOCUS: Color = Color::Rgb(139, 92, 246);// Border active
    pub const SEL_BG: Color = Color::Rgb(45, 55, 72);        // Selected row background
    pub const SEL_FG: Color = Color::Rgb(255, 255, 255);     // Selected row foreground

    pub fn header() -> Style {
        Style::default().fg(Self::CYAN).add_modifier(Modifier::BOLD)
    }

    pub fn header_label() -> Style {
        Style::default().fg(Self::DIM)
    }

    pub fn header_val() -> Style {
        Style::default().fg(Self::FG).add_modifier(Modifier::BOLD)
    }

    pub fn title() -> Style {
        Style::default().fg(Self::ACCENT).add_modifier(Modifier::BOLD)
    }

    pub fn table_header() -> Style {
        Style::default().fg(Self::CYAN).add_modifier(Modifier::BOLD)
    }

    pub fn selected_row() -> Style {
        Style::default().bg(Self::SEL_BG).fg(Self::SEL_FG).add_modifier(Modifier::BOLD)
    }

    pub fn marked_row() -> Style {
        Style::default().bg(Color::Rgb(60, 40, 90)).fg(Color::Yellow).add_modifier(Modifier::BOLD)
    }

    pub fn status_ok() -> Style {
        Style::default().fg(Self::GREEN).add_modifier(Modifier::BOLD)
    }

    pub fn status_warn() -> Style {
        Style::default().fg(Self::YELLOW).add_modifier(Modifier::BOLD)
    }

    pub fn status_error() -> Style {
        Style::default().fg(Self::RED).add_modifier(Modifier::BOLD)
    }

    pub fn context_color(ctx_name: &str, is_local: bool) -> Color {
        let lower = ctx_name.to_lowercase();
        if lower.contains("prod") || lower.contains("prd") || lower.contains("live") {
            Color::Rgb(255, 110, 110) // Coral red for production
        } else if lower.contains("stage") || lower.contains("stg") || lower.contains("uat") || lower.contains("qa") {
            Color::Rgb(255, 200, 80) // Amber gold for staging
        } else if is_local || lower.contains("dev") || lower.contains("kind") || lower.contains("minikube") || lower.contains("k3d") || lower.contains("local") {
            Color::Rgb(80, 220, 140) // Mint green for local dev
        } else {
            Color::Rgb(100, 200, 255) // Sky blue for general clusters
        }
    }

    pub fn status_dim() -> Style {
        Style::default().fg(Self::DIM)
    }

    pub fn key_hint_key() -> Style {
        Style::default().fg(Self::CYAN).add_modifier(Modifier::BOLD)
    }

    pub fn key_hint_desc() -> Style {
        Style::default().fg(Self::DIM)
    }

    pub fn prompt() -> Style {
        Style::default().fg(Self::ACCENT).add_modifier(Modifier::BOLD)
    }

    pub fn badge(bg: Color, fg: Color) -> Style {
        Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)
    }
}

/// Helper function to colorize a status string (e.g. "Running" -> Green, "CrashLoopBackOff" -> Red)
pub fn status_style(status: &str) -> Style {
    let lower = status.to_lowercase();
    if lower.contains("running") || lower.contains("active") || lower.contains("ready") || lower.contains("completed") || lower.contains("succeeded") || lower == "true" {
        Theme::status_ok()
    } else if lower.contains("pending") || lower.contains("containercreating") || lower.contains("terminating") || lower.contains("warning") {
        Theme::status_warn()
    } else if lower.contains("crash") || lower.contains("error") || lower.contains("failed") || lower.contains("notready") || lower.contains("unknown") || lower.contains("backoff") {
        Theme::status_error()
    } else {
        Style::default().fg(Theme::FG)
    }
}
