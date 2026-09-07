use std::sync::atomic::{AtomicUsize, Ordering};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HeaderStyle {
    Standard,
    FinoTime,
    Minimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThemeId {
    CatppuccinMocha,
    TokyoNight,
    Dracula,
    Nord,
    GruvboxDark,
    SolarizedDark,
    MonokaiPro,
    CatppuccinLatte,
    FinoTime,
    RosePine,
    Cyberpunk,
    OneDark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub id: ThemeId,
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub is_light: bool,
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub cyan: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub orange: Color,
    pub border: Color,
    pub border_focus: Color,
    pub sel_bg: Color,
    pub sel_fg: Color,
    pub marked_bg: Color,
    pub marked_fg: Color,
    pub header_style: HeaderStyle,
    pub border_type: BorderType,
    pub prompt_glyph: &'static str,
    pub bullet_glyph: &'static str,
    pub brand_icon: &'static str,
    pub show_live_clock: bool,
}

pub static ALL_THEMES: &[ThemePalette] = &[
    ThemePalette {
        id: ThemeId::CatppuccinMocha,
        name: "catppuccin-mocha",
        display_name: "Catppuccin Mocha",
        description: "Brand lavender & violet on deep charcoal — SRElens original",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(220, 224, 232),
        dim: Color::Rgb(110, 115, 135),
        accent: Color::Rgb(139, 92, 246),
        cyan: Color::Rgb(56, 189, 248),
        green: Color::Rgb(34, 197, 94),
        yellow: Color::Rgb(234, 179, 8),
        red: Color::Rgb(239, 68, 68),
        orange: Color::Rgb(249, 115, 22),
        border: Color::Rgb(75, 85, 99),
        border_focus: Color::Rgb(139, 92, 246),
        sel_bg: Color::Rgb(45, 55, 72),
        sel_fg: Color::Rgb(255, 255, 255),
        marked_bg: Color::Rgb(60, 40, 90),
        marked_fg: Color::Yellow,
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Rounded,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::TokyoNight,
        name: "tokyo-night",
        display_name: "Tokyo Night",
        description: "Neon cyan and soft blue on dark midnight indigo",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(192, 202, 245),
        dim: Color::Rgb(86, 95, 137),
        accent: Color::Rgb(122, 162, 247),
        cyan: Color::Rgb(125, 207, 255),
        green: Color::Rgb(158, 206, 106),
        yellow: Color::Rgb(224, 175, 104),
        red: Color::Rgb(247, 118, 142),
        orange: Color::Rgb(255, 158, 100),
        border: Color::Rgb(59, 66, 97),
        border_focus: Color::Rgb(122, 162, 247),
        sel_bg: Color::Rgb(40, 52, 87),
        sel_fg: Color::Rgb(255, 255, 255),
        marked_bg: Color::Rgb(65, 45, 80),
        marked_fg: Color::Rgb(224, 175, 104),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Rounded,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::Dracula,
        name: "dracula",
        display_name: "Dracula",
        description: "High contrast purple, pink, and luminous neon",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(248, 248, 242),
        dim: Color::Rgb(98, 114, 164),
        accent: Color::Rgb(189, 147, 249),
        cyan: Color::Rgb(139, 233, 253),
        green: Color::Rgb(80, 250, 123),
        yellow: Color::Rgb(241, 250, 140),
        red: Color::Rgb(255, 85, 85),
        orange: Color::Rgb(255, 184, 108),
        border: Color::Rgb(68, 71, 90),
        border_focus: Color::Rgb(189, 147, 249),
        sel_bg: Color::Rgb(68, 71, 90),
        sel_fg: Color::Rgb(255, 255, 255),
        marked_bg: Color::Rgb(70, 45, 85),
        marked_fg: Color::Rgb(241, 250, 140),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Rounded,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::Nord,
        name: "nord",
        display_name: "Nord",
        description: "Arctic frost cyan, snow white, and aurora colors",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(236, 239, 244),
        dim: Color::Rgb(76, 86, 106),
        accent: Color::Rgb(136, 192, 208),
        cyan: Color::Rgb(129, 161, 193),
        green: Color::Rgb(163, 190, 140),
        yellow: Color::Rgb(235, 203, 139),
        red: Color::Rgb(191, 97, 106),
        orange: Color::Rgb(208, 135, 112),
        border: Color::Rgb(59, 66, 82),
        border_focus: Color::Rgb(136, 192, 208),
        sel_bg: Color::Rgb(67, 76, 94),
        sel_fg: Color::Rgb(236, 239, 244),
        marked_bg: Color::Rgb(60, 50, 75),
        marked_fg: Color::Rgb(235, 203, 139),
        header_style: HeaderStyle::Minimal,
        border_type: BorderType::Plain,
        prompt_glyph: "❯ ",
        bullet_glyph: "◆",
        brand_icon: "❄ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::GruvboxDark,
        name: "gruvbox-dark",
        display_name: "Gruvbox Dark",
        description: "Warm retro amber, olive green, and earthy tones",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(235, 219, 178),
        dim: Color::Rgb(146, 131, 116),
        accent: Color::Rgb(254, 128, 25),
        cyan: Color::Rgb(131, 165, 152),
        green: Color::Rgb(184, 187, 38),
        yellow: Color::Rgb(250, 189, 47),
        red: Color::Rgb(251, 73, 52),
        orange: Color::Rgb(254, 128, 25),
        border: Color::Rgb(80, 73, 69),
        border_focus: Color::Rgb(254, 128, 25),
        sel_bg: Color::Rgb(60, 56, 54),
        sel_fg: Color::Rgb(251, 241, 199),
        marked_bg: Color::Rgb(80, 50, 40),
        marked_fg: Color::Rgb(250, 189, 47),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Plain,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::SolarizedDark,
        name: "solarized-dark",
        display_name: "Solarized Dark",
        description: "Engineered low-contrast teal, cyan, and blue",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(131, 148, 150),
        dim: Color::Rgb(88, 110, 117),
        accent: Color::Rgb(38, 139, 210),
        cyan: Color::Rgb(42, 161, 152),
        green: Color::Rgb(133, 153, 0),
        yellow: Color::Rgb(181, 137, 0),
        red: Color::Rgb(220, 50, 47),
        orange: Color::Rgb(203, 75, 22),
        border: Color::Rgb(7, 54, 66),
        border_focus: Color::Rgb(42, 161, 152),
        sel_bg: Color::Rgb(7, 54, 66),
        sel_fg: Color::Rgb(147, 161, 161),
        marked_bg: Color::Rgb(20, 45, 60),
        marked_fg: Color::Rgb(181, 137, 0),
        header_style: HeaderStyle::Minimal,
        border_type: BorderType::Plain,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::MonokaiPro,
        name: "monokai-pro",
        display_name: "Monokai Pro",
        description: "Vibrant yellow, electric cyan, and magenta",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(252, 252, 250),
        dim: Color::Rgb(114, 112, 114),
        accent: Color::Rgb(255, 216, 102),
        cyan: Color::Rgb(120, 220, 232),
        green: Color::Rgb(169, 220, 118),
        yellow: Color::Rgb(255, 216, 102),
        red: Color::Rgb(255, 97, 136),
        orange: Color::Rgb(252, 152, 103),
        border: Color::Rgb(64, 62, 65),
        border_focus: Color::Rgb(255, 216, 102),
        sel_bg: Color::Rgb(64, 62, 65),
        sel_fg: Color::Rgb(255, 255, 255),
        marked_bg: Color::Rgb(70, 50, 55),
        marked_fg: Color::Rgb(255, 216, 102),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Rounded,
        prompt_glyph: "▶ ",
        bullet_glyph: "▪",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::CatppuccinLatte,
        name: "catppuccin-latte",
        display_name: "Catppuccin Latte (Light)",
        description: "Clean daytime light mode with dark text and crisp contrast",
        is_light: true,
        bg: Color::Reset,
        fg: Color::Rgb(76, 79, 105),
        dim: Color::Rgb(156, 160, 176),
        accent: Color::Rgb(136, 57, 239),
        cyan: Color::Rgb(30, 102, 245),
        green: Color::Rgb(64, 160, 43),
        yellow: Color::Rgb(223, 142, 29),
        red: Color::Rgb(210, 15, 57),
        orange: Color::Rgb(254, 100, 11),
        border: Color::Rgb(188, 192, 204),
        border_focus: Color::Rgb(136, 57, 239),
        sel_bg: Color::Rgb(204, 208, 218),
        sel_fg: Color::Rgb(76, 79, 105),
        marked_bg: Color::Rgb(220, 210, 230),
        marked_fg: Color::Rgb(136, 57, 239),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Rounded,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::FinoTime,
        name: "fino-time",
        display_name: "Fino Time (Zsh)",
        description: "Oh-My-Zsh fino-time tree-connected header, live clock & hollow circle prompt",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(225, 230, 235),
        dim: Color::Rgb(105, 115, 125),
        accent: Color::Rgb(250, 204, 21),
        cyan: Color::Rgb(56, 189, 248),
        green: Color::Rgb(52, 211, 153),
        yellow: Color::Rgb(251, 191, 36),
        red: Color::Rgb(248, 113, 113),
        orange: Color::Rgb(251, 146, 60),
        border: Color::Rgb(70, 80, 90),
        border_focus: Color::Rgb(250, 204, 21),
        sel_bg: Color::Rgb(40, 50, 62),
        sel_fg: Color::Rgb(255, 255, 255),
        marked_bg: Color::Rgb(55, 55, 75),
        marked_fg: Color::Rgb(250, 204, 21),
        header_style: HeaderStyle::FinoTime,
        border_type: BorderType::Rounded,
        prompt_glyph: "╰─○ ",
        bullet_glyph: "○",
        brand_icon: "╭─",
        show_live_clock: true,
    },
    ThemePalette {
        id: ThemeId::RosePine,
        name: "rose-pine",
        display_name: "Rosé Pine",
        description: "All natural pine, foam, iris, and love pastels on warm dark velvet",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(224, 222, 244),
        dim: Color::Rgb(110, 106, 134),
        accent: Color::Rgb(235, 188, 186),
        cyan: Color::Rgb(156, 207, 216),
        green: Color::Rgb(196, 167, 231),
        yellow: Color::Rgb(246, 193, 119),
        red: Color::Rgb(235, 111, 146),
        orange: Color::Rgb(235, 188, 186),
        border: Color::Rgb(68, 65, 90),
        border_focus: Color::Rgb(235, 188, 186),
        sel_bg: Color::Rgb(50, 47, 70),
        sel_fg: Color::Rgb(240, 240, 255),
        marked_bg: Color::Rgb(65, 45, 75),
        marked_fg: Color::Rgb(246, 193, 119),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Rounded,
        prompt_glyph: "❯ ",
        bullet_glyph: "◆",
        brand_icon: "✦ ",
        show_live_clock: false,
    },
    ThemePalette {
        id: ThemeId::Cyberpunk,
        name: "cyberpunk",
        display_name: "Cyberpunk / Synth",
        description: "High-voltage neon cyan, hot magenta, electric amber & thick cyber borders",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(240, 245, 255),
        dim: Color::Rgb(100, 90, 130),
        accent: Color::Rgb(255, 0, 127),
        cyan: Color::Rgb(0, 240, 255),
        green: Color::Rgb(0, 255, 159),
        yellow: Color::Rgb(255, 222, 0),
        red: Color::Rgb(255, 42, 109),
        orange: Color::Rgb(255, 140, 0),
        border: Color::Rgb(110, 40, 140),
        border_focus: Color::Rgb(0, 240, 255),
        sel_bg: Color::Rgb(60, 20, 75),
        sel_fg: Color::Rgb(0, 240, 255),
        marked_bg: Color::Rgb(70, 15, 60),
        marked_fg: Color::Rgb(255, 222, 0),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Thick,
        prompt_glyph: "▶ ",
        bullet_glyph: "▪",
        brand_icon: "⚡ ",
        show_live_clock: true,
    },
    ThemePalette {
        id: ThemeId::OneDark,
        name: "one-dark",
        display_name: "One Dark",
        description: "Iconic Atom & Neovim syntax theme with balanced chalky pastels",
        is_light: false,
        bg: Color::Reset,
        fg: Color::Rgb(171, 178, 191),
        dim: Color::Rgb(92, 99, 112),
        accent: Color::Rgb(97, 175, 239),
        cyan: Color::Rgb(86, 182, 194),
        green: Color::Rgb(152, 195, 121),
        yellow: Color::Rgb(229, 192, 123),
        red: Color::Rgb(224, 108, 117),
        orange: Color::Rgb(209, 154, 102),
        border: Color::Rgb(75, 82, 99),
        border_focus: Color::Rgb(97, 175, 239),
        sel_bg: Color::Rgb(44, 50, 60),
        sel_fg: Color::Rgb(255, 255, 255),
        marked_bg: Color::Rgb(55, 55, 75),
        marked_fg: Color::Rgb(229, 192, 123),
        header_style: HeaderStyle::Standard,
        border_type: BorderType::Plain,
        prompt_glyph: ":",
        bullet_glyph: "●",
        brand_icon: "⚡ ",
        show_live_clock: false,
    },
];

static ACTIVE_THEME_IDX: AtomicUsize = AtomicUsize::new(0);

/// SRElens TUI Theme palette and dynamic styling engine
pub struct Theme;

impl Theme {
    // Compile-time default constant fallbacks for tests and constant evaluation
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

    /// Active theme palette reference
    pub fn active_palette() -> &'static ThemePalette {
        let idx = ACTIVE_THEME_IDX.load(Ordering::Relaxed);
        &ALL_THEMES[idx.min(ALL_THEMES.len() - 1)]
    }

    /// Current active theme index
    pub fn active_index() -> usize {
        ACTIVE_THEME_IDX.load(Ordering::Relaxed).min(ALL_THEMES.len() - 1)
    }

    /// Set theme by index (0-based)
    pub fn set_theme_by_index(idx: usize) -> bool {
        if idx < ALL_THEMES.len() {
            ACTIVE_THEME_IDX.store(idx, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Set theme by case-insensitive name or alias
    pub fn set_theme_by_name(name: &str) -> Option<&'static ThemePalette> {
        let lower = name.trim().to_lowercase();
        for (i, p) in ALL_THEMES.iter().enumerate() {
            if p.name == lower
                || p.name.replace('-', "") == lower.replace(['-', '_', ' '], "")
                || (lower == "default" && p.id == ThemeId::CatppuccinMocha)
                || (lower == "light" && p.id == ThemeId::CatppuccinLatte)
                || (lower == "latte" && p.id == ThemeId::CatppuccinLatte)
                || (lower == "mocha" && p.id == ThemeId::CatppuccinMocha)
                || (lower == "gruvbox" && p.id == ThemeId::GruvboxDark)
                || (lower == "solarized" && p.id == ThemeId::SolarizedDark)
                || (lower == "monokai" && p.id == ThemeId::MonokaiPro)
                || (lower == "tokyo" && p.id == ThemeId::TokyoNight)
                || (lower == "fino" && p.id == ThemeId::FinoTime)
                || (lower == "finotime" && p.id == ThemeId::FinoTime)
                || (lower == "fino-time" && p.id == ThemeId::FinoTime)
                || (lower == "rosepine" && p.id == ThemeId::RosePine)
                || (lower == "rose-pine" && p.id == ThemeId::RosePine)
                || (lower == "cyberpunk" && p.id == ThemeId::Cyberpunk)
                || (lower == "synth" && p.id == ThemeId::Cyberpunk)
                || (lower == "synthwave" && p.id == ThemeId::Cyberpunk)
                || (lower == "onedark" && p.id == ThemeId::OneDark)
                || (lower == "one-dark" && p.id == ThemeId::OneDark)
                || (lower == "atom" && p.id == ThemeId::OneDark)
            {
                ACTIVE_THEME_IDX.store(i, Ordering::Relaxed);
                return Some(p);
            }
        }
        None
    }

    pub fn all_themes() -> &'static [ThemePalette] {
        ALL_THEMES
    }

    // Visual chrome and layout accessors
    pub fn header_style() -> HeaderStyle { Self::active_palette().header_style }
    pub fn border_type() -> BorderType { Self::active_palette().border_type }
    pub fn prompt_glyph() -> &'static str { Self::active_palette().prompt_glyph }
    pub fn bullet_glyph() -> &'static str { Self::active_palette().bullet_glyph }
    pub fn brand_icon() -> &'static str { Self::active_palette().brand_icon }
    pub fn show_live_clock() -> bool { Self::active_palette().show_live_clock }

    // Dynamic color accessors reading live from the active theme
    pub fn bg() -> Color { Self::active_palette().bg }
    pub fn fg() -> Color { Self::active_palette().fg }
    pub fn dim() -> Color { Self::active_palette().dim }
    pub fn accent() -> Color { Self::active_palette().accent }
    pub fn cyan() -> Color { Self::active_palette().cyan }
    pub fn green() -> Color { Self::active_palette().green }
    pub fn yellow() -> Color { Self::active_palette().yellow }
    pub fn red() -> Color { Self::active_palette().red }
    pub fn orange() -> Color { Self::active_palette().orange }
    pub fn border() -> Color { Self::active_palette().border }
    pub fn border_focus() -> Color { Self::active_palette().border_focus }
    pub fn sel_bg() -> Color { Self::active_palette().sel_bg }
    pub fn sel_fg() -> Color { Self::active_palette().sel_fg }
    pub fn marked_bg() -> Color { Self::active_palette().marked_bg }
    pub fn marked_fg() -> Color { Self::active_palette().marked_fg }

    pub fn header() -> Style {
        Style::default().fg(Self::cyan()).add_modifier(Modifier::BOLD)
    }

    pub fn header_label() -> Style {
        Style::default().fg(Self::dim())
    }

    pub fn header_val() -> Style {
        Style::default().fg(Self::fg()).add_modifier(Modifier::BOLD)
    }

    pub fn title() -> Style {
        Style::default().fg(Self::accent()).add_modifier(Modifier::BOLD)
    }

    pub fn table_header() -> Style {
        Style::default().fg(Self::cyan()).add_modifier(Modifier::BOLD)
    }

    pub fn selected_row() -> Style {
        Style::default().bg(Self::sel_bg()).fg(Self::sel_fg()).add_modifier(Modifier::BOLD)
    }

    pub fn marked_row() -> Style {
        Style::default().bg(Self::marked_bg()).fg(Self::marked_fg()).add_modifier(Modifier::BOLD)
    }

    pub fn status_ok() -> Style {
        Style::default().fg(Self::green()).add_modifier(Modifier::BOLD)
    }

    pub fn status_warn() -> Style {
        Style::default().fg(Self::yellow()).add_modifier(Modifier::BOLD)
    }

    pub fn status_error() -> Style {
        Style::default().fg(Self::red()).add_modifier(Modifier::BOLD)
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
        Style::default().fg(Self::dim())
    }

    pub fn key_hint_key() -> Style {
        Style::default().fg(Self::cyan()).add_modifier(Modifier::BOLD)
    }

    pub fn key_hint_desc() -> Style {
        Style::default().fg(Self::dim())
    }

    pub fn prompt() -> Style {
        Style::default().fg(Self::accent()).add_modifier(Modifier::BOLD)
    }

    pub fn badge(bg: Color, fg: Color) -> Style {
        Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)
    }
}

/// Helper function to colorize a status string (e.g. "Running" -> Green, "CrashLoopBackOff" -> Red)
pub fn status_style(status: &str) -> Style {
    let lower = status.to_lowercase();
    if lower.contains("crash")
        || lower.contains("error")
        || lower.contains("failed")
        || lower.contains("notready")
        || lower.contains("unknown")
        || lower.contains("backoff")
        || lower.contains("degraded")
        || lower == "false"
    {
        Theme::status_error()
    } else if lower.contains("scaled down")
        || lower.contains("not scheduled")
        || lower.contains("suspended")
    {
        Style::default().fg(Theme::dim())
    } else if lower.contains("pending")
        || lower.contains("containercreating")
        || lower.contains("terminating")
        || lower.contains("warning")
        || lower.contains("schedulingdisabled")
        || lower.contains("cordon")
    {
        Theme::status_warn()
    } else if lower.contains("running")
        || lower.contains("active")
        || lower.contains("ready")
        || lower.contains("completed")
        || lower.contains("succeeded")
        || lower.contains("scheduled")
        || lower == "true"
        || lower == "secretsynced"
    {
        Theme::status_ok()
    } else {
        Style::default().fg(Theme::fg())
    }
}

