//! `:cost` - Cost & Compute for the selected cluster.
//!
//! What the nodes offer, what pods have asked for, what is idle, and what that
//! costs. The cost is an ESTIMATE from per-hour rates (see
//! `srelens_kube::cost`), and the screen says so; it is not billing data.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use srelens_kube::cost::{ClusterCompute, NamespaceCompute, HOURS_PER_MONTH};
use srelens_kube::kubecost_install::Step;

use crate::theme::Theme;

/// Where an install of Kubecost started from this screen stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    Idle,
    /// Includes `Step::WaitingForData`: installed, not yet reporting figures.
    Running(Step),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct CostViewState {
    /// The cluster this view is for; a result for any other is dropped.
    pub context: String,
    /// The fetch whose answer this view is waiting for; older answers are dropped.
    pub request: u64,
    pub data: Option<ClusterCompute>,
    pub error: Option<String>,
    pub is_loading: bool,
    pub scroll: u16,
    /// Columns scrolled off the left edge. The tables are wider than a small
    /// terminal, so the right-hand columns are reached by scrolling sideways.
    pub hscroll: u16,
    /// How far sideways the last frame could scroll; the keys stop there.
    max_hscroll: std::cell::Cell<u16>,
    pub install: InstallState,
    /// When the current install, or its wait for data, began.
    pub install_since: Option<std::time::Instant>,
}

impl CostViewState {
    pub fn new(context: &str) -> Self {
        Self {
            context: context.to_string(),
            request: 0,
            data: None,
            error: None,
            is_loading: true,
            scroll: 0,
            hscroll: 0,
            max_hscroll: std::cell::Cell::new(0),
            install: InstallState::Idle,
            install_since: None,
        }
    }

    pub fn set_data(&mut self, data: ClusterCompute) {
        self.data = Some(data);
        self.error = None;
        self.is_loading = false;
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.is_loading = false;
    }

    /// Start a reload. The old numbers stay on screen until the new ones land.
    pub fn start_reload(&mut self) {
        self.is_loading = true;
        self.error = None;
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn page_down(&mut self, page: u16) {
        self.scroll = self.scroll.saturating_add(page.max(1));
    }

    pub fn page_up(&mut self, page: u16) {
        self.scroll = self.scroll.saturating_sub(page.max(1));
    }

    pub fn scroll_right(&mut self, step: u16) {
        self.hscroll = self.hscroll.saturating_add(step).min(self.max_hscroll.get());
    }

    pub fn scroll_left(&mut self, step: u16) {
        self.hscroll = self.hscroll.saturating_sub(step);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = u16::MAX;
    }
}

/// `$12.34`, with thousands separators above a thousand and more places for
/// the small hourly figures that would otherwise round to nothing.
pub fn money(value: f64, hourly: bool) -> String {
    let v = if value.is_finite() { value.max(0.0) } else { 0.0 };
    if hourly && v < 10.0 {
        return format!("${v:.3}");
    }
    if v >= 1000.0 {
        let whole = format!("{:.0}", v);
        let mut out = String::new();
        for (i, c) in whole.chars().enumerate() {
            if i > 0 && (whole.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        return format!("${out}");
    }
    format!("${v:.2}")
}

fn cores(millicores: i64) -> String {
    format!("{:.1}", millicores as f64 / 1000.0)
}

fn mem(mib: i64) -> String {
    if mib >= 1024 {
        format!("{:.1}Gi", mib as f64 / 1024.0)
    } else {
        format!("{mib}Mi")
    }
}

fn percent(part: i64, whole: i64) -> f64 {
    if whole <= 0 {
        0.0
    } else {
        (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0)
    }
}

fn bar(pct: f64, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn heading(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
    ))
}

fn dim(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Theme::status_dim())
}

/// A resource row: `  CPU     8.0 cores   requested 2.0 (25%) ████░░░░   used ...`
fn resource_line(
    label: &str,
    allocatable: i64,
    requested: i64,
    usage: Option<i64>,
    fmt: fn(i64) -> String,
) -> Line<'static> {
    let req_pct = percent(requested, allocatable);
    let mut spans = vec![
        Span::styled(format!("  {label:<7}"), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("{} available   ", fmt(allocatable))),
        Span::raw(format!("Requested {} ({:.0}%) ", fmt(requested), req_pct)),
        Span::styled(bar(req_pct, 10), Style::default().fg(Theme::cyan())),
        Span::raw("   "),
    ];
    match usage {
        Some(u) => {
            let use_pct = percent(u, allocatable);
            spans.push(Span::raw(format!("Used {} ({:.0}%) ", fmt(u), use_pct)));
            spans.push(Span::styled(bar(use_pct, 10), Style::default().fg(Theme::green())));
        }
        None => spans.push(dim("Used n/a")),
    }
    Line::from(spans)
}

fn lines_for(data: &ClusterCompute, width: usize, can_offer_install: bool) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();

    if data.nodes.is_empty() {
        out.push(Line::from(Span::styled(
            "No nodes were returned for this cluster.",
            Theme::status_warn(),
        )));
        return out;
    }

    out.push(heading("CLUSTER"));
    out.push(Line::from(format!("  {} nodes   {} pods running", data.nodes.len(), data.pods)));
    let pct = data.requested_percent();
    let row = |label: &str, hourly: f64, extra: Vec<Span<'static>>| {
        let mut spans = vec![
            Span::styled(format!("  {label:<22}"), Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("{:>9}/hr   ", money(hourly, true))),
            Span::styled(
                format!("{:>9}/mo", money(hourly * HOURS_PER_MONTH, false)),
                Style::default().fg(Theme::green()).add_modifier(Modifier::BOLD),
            ),
        ];
        spans.extend(extra);
        Line::from(spans)
    };
    out.push(row("Estimated cost (nodes)", data.cluster_hourly, vec![]));
    out.push(row(
        "Requested by pods",
        data.requested_hourly,
        vec![
            Span::raw(format!("   {pct:.0}%  ")),
            Span::styled(bar(pct, 10), Style::default().fg(Theme::cyan())),
        ],
    ));
    out.push(row(
        "Idle (not requested)",
        data.idle_hourly(),
        vec![Span::raw(format!("   {:.0}%", (100.0 - pct).max(0.0)))],
    ));
    if let Some(p) = &data.provider {
        out.push(Line::from(vec![
            Span::styled("  Source: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{} ({})", p.source.label(), p.location),
                Style::default().fg(Theme::green()).add_modifier(Modifier::BOLD),
            ),
            dim(format!("  real spend over {} of data, scaled to a month", p.window_label())),
        ]));
        out.push(row(
            &format!("Real cost ({})", p.source.label()),
            p.total_monthly() / HOURS_PER_MONTH,
            vec![],
        ));
        out.push(row(
            &format!("Real idle ({})", p.source.label()),
            p.idle_monthly() / HOURS_PER_MONTH,
            vec![],
        ));
    }
    out.push(Line::raw(""));

    out.push(heading("COMPUTE"));
    out.push(resource_line("CPU", data.allocatable_cpu_m, data.requested_cpu_m, data.usage_cpu_m, |m| {
        format!("{} cores", cores(m))
    }));
    out.push(resource_line("Memory", data.allocatable_mem_mib, data.requested_mem_mib, data.usage_mem_mib, mem));
    out.push(Line::raw(""));

    out.push(heading("NODES"));
    out.push(Line::from(Span::styled(
        format!(
            "  {:<27}{:<15}{:<14}{:>14}{:>16}{:>10}{:>10}{:>6}{:>11}",
            "NAME", "TYPE", "ZONE", "CPU req/all", "MEM req/all", "CPU used", "MEM used", "PODS", "$/MO"
        ),
        Theme::status_dim(),
    )));
    for n in &data.nodes {
        out.push(Line::from(format!(
            "  {:<27}{:<15}{:<14}{:>14}{:>16}{:>10}{:>10}{:>6}{:>11}",
            truncate(&n.name, 25),
            truncate(n.instance_type.as_deref().unwrap_or("-"), 13),
            truncate(n.zone.as_deref().unwrap_or("-"), 12),
            format!("{}/{}", cores(n.requested_cpu_m), cores(n.allocatable_cpu_m)),
            format!("{}/{}", mem(n.requested_mem_mib), mem(n.allocatable_mem_mib)),
            n.usage_cpu_m.map(cores).unwrap_or_else(|| "n/a".into()),
            n.usage_mem_mib.map(mem).unwrap_or_else(|| "n/a".into()),
            n.pods,
            money(n.hourly * HOURS_PER_MONTH, false),
        )));
    }
    out.push(Line::raw(""));

    out.push(heading("NAMESPACES  (priced by what their pods request, largest first)"));
    let real_cols = if data.provider.is_some() {
        format!("{:>15}{:>7}", "REAL $/MO", "EFF")
    } else {
        String::new()
    };
    out.push(Line::from(Span::styled(
        format!(
            "  {:<30}{:>6}{:>10}{:>10}{:>11}{:>11}{:>8}{}",
            "NAMESPACE", "PODS", "CPU req", "MEM req", "$/HR", "$/MO", "SHARE", real_cols
        ),
        Theme::status_dim(),
    )));
    // The rows are what pods request now, plus any namespace the cost tool
    // billed over its window that has nothing running now (its Jobs finished):
    // that spend is in the total, so it must have a row.
    let mut rows: Vec<NamespaceCompute> = data.namespaces.clone();
    if let Some(p) = &data.provider {
        let mut gone: Vec<&srelens_kube::opencost::NamespaceSpend> = p
            .namespaces
            .iter()
            .filter(|r| !rows.iter().any(|ns| ns.namespace == r.name))
            .collect();
        gone.sort_by(|a, b| b.total.partial_cmp(&a.total).unwrap_or(std::cmp::Ordering::Equal));
        rows.extend(gone.into_iter().map(|r| NamespaceCompute {
            namespace: r.name.clone(),
            pods: 0,
            requested_cpu_m: 0,
            requested_mem_mib: 0,
            hourly: 0.0,
        }));
    }
    for ns in &rows {
        let share = if data.requested_hourly > 0.0 { ns.hourly / data.requested_hourly * 100.0 } else { 0.0 };
        let real = match &data.provider {
            None => String::new(),
            Some(p) => match p.for_namespace(&ns.namespace) {
                Some(r) => format!(
                    "{:>15}{:>7}",
                    money(p.monthly(r.total), false),
                    r.efficiency.map_or("-".to_string(), |e| format!("{:.0}%", e * 100.0)),
                ),
                None => format!("{:>15}{:>7}", "-", "-"),
            },
        };
        out.push(Line::from(format!(
            "  {:<30}{:>6}{:>10}{:>10}{:>11}{:>11}{:>7.0}%{}",
            truncate(&ns.namespace, 29),
            ns.pods,
            cores(ns.requested_cpu_m),
            mem(ns.requested_mem_mib),
            money(ns.hourly, true),
            money(ns.monthly(), false),
            share,
            real,
        )));
    }
    out.push(Line::raw(""));

    let mut notes: Vec<(&str, String)> = vec![(
        "ESTIMATE ",
        format!(
            "Not billing data. Priced at ${} per core-hour and ${} per GiB-hour from node allocatable and pod \
             requests; instance prices, discounts, spot, storage and network are not included. Set the rates in \
             tui.json (costCpuCoreHour, costMemGibHour).",
            data.rates.cpu_core_hour, data.rates.mem_gib_hour
        ),
    )];
    match (&data.provider, &data.provider_note) {
        // The look itself failed: say so, and do not claim anything either way.
        (None, None) if data.discovery_note.is_some() => notes.push((
            "COST SOURCE ",
            format!(
                "Could not check whether Kubecost or OpenCost is installed: {}. Showing the estimate; press r to retry.",
                data.discovery_note.as_deref().unwrap_or_default()
            ),
        )),
        (Some(p), _) => notes.push((
            "REAL ",
            format!(
                "The REAL columns are {}'s own allocation over {} of data, scaled to a month; the other \
                 cost columns are the requests-based estimate.",
                p.source.label(),
                p.window_label()
            ),
        )),
        (None, Some(why)) => notes.push((
            "COST SOURCE ",
            format!("A cost tool is installed but is not giving cost data: {why}. Showing the estimate."),
        )),
        (None, None) => notes.push((
            "COST SOURCE ",
            if can_offer_install {
                "No Kubecost or OpenCost was found in this cluster, so the cost shown is the estimate. \
                 Press i to install Kubecost for real spend (it asks first)."
                    .to_string()
            } else {
                "No Kubecost or OpenCost was found in this cluster, so the cost shown is the estimate."
                    .to_string()
            },
        )),
    }
    if let Some(p) = data.provider.as_ref().filter(|p| p.is_short_sample()) {
        notes.push((
            "SHORT SAMPLE ",
            format!(
                "{} has only {} of history, so its monthly figures are extrapolated from a very short sample and will \
                 move a lot as it collects more.",
                p.source.label(),
                p.window_label()
            ),
        ));
    }
    if data.usage_cpu_m.is_none() {
        let why = data.usage_note.as_deref().unwrap_or("metrics-server did not respond");
        notes.push((
            "USAGE ",
            format!("Live usage is unavailable: {why}. The requested figures above are unaffected."),
        ));
    }
    if data.truncated {
        notes.push((
            "PARTIAL ",
            "The node or pod list hit the list limit, so these figures cover only part of the cluster.".to_string(),
        ));
    }
    for (label, text) in notes {
        let room = width.saturating_sub(label.len() + 1).max(20);
        for (i, piece) in wrap_text(&text, room).into_iter().enumerate() {
            if i == 0 {
                out.push(Line::from(vec![Span::styled(label.to_string(), Theme::status_warn()), dim(piece)]));
            } else {
                out.push(Line::from(dim(format!("{}{piece}", " ".repeat(label.len())))));
            }
        }
    }
    out
}

/// The install's status, above everything else while there is one.
fn install_banner(install: &InstallState, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    match install {
        InstallState::Idle => {}
        InstallState::Running(step) => {
            out.push(Line::from(vec![
                Span::styled("INSTALLING KUBECOST ", Theme::status_warn()),
                Span::styled(step.label().to_string(), Style::default().add_modifier(Modifier::BOLD)),
            ]));
            out.push(Line::from(dim(
                "You can keep using srectl. The estimate below stays until Kubecost has figures.",
            )));
            out.push(Line::raw(""));
        }
        InstallState::Failed(why) => {
            for (i, piece) in wrap_text(&format!("Kubecost was not installed: {why}"), width.saturating_sub(21).max(20))
                .into_iter()
                .enumerate()
            {
                if i == 0 {
                    out.push(Line::from(vec![Span::styled("INSTALL FAILED ", Theme::status_error()), dim(piece)]));
                } else {
                    out.push(Line::from(dim(format!("{}{piece}", " ".repeat(15)))));
                }
            }
            out.push(Line::from(dim("Press i to try again. The estimate is shown below.")));
            out.push(Line::raw(""));
        }
    }
    out
}

/// Break `text` at spaces into lines of at most `width` characters.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

pub fn render(f: &mut Frame, area: Rect, state: &CostViewState) {
    let status = if state.is_loading && state.data.is_some() {
        " (refreshing)"
    } else {
        ""
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::accent()))
        .title(Span::styled(
            format!(" Cost & Compute · {}{status} ", state.context),
            Theme::title(),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let can_offer_install = matches!(state.install, InstallState::Idle | InstallState::Failed(_));
    let mut lines: Vec<Line<'static>> = install_banner(&state.install, inner.width as usize);
    lines.extend(match (&state.data, &state.error) {
        (_, Some(err)) => vec![
            Line::from(Span::styled(format!("Could not load cost and compute: {err}"), Theme::status_error())),
            Line::raw(""),
            Line::from(dim("Press r to retry.")),
        ],
        (Some(data), None) => lines_for(data, inner.width as usize, can_offer_install),
        (None, None) => vec![Line::from(dim(format!("Loading compute for {}...", state.context)))],
    });

    // Keep the scroll inside the content so a held `G` or `j` cannot leave a
    // blank screen.
    let max_scroll = (lines.len() as u16).saturating_sub(inner.height);
    let scroll = state.scroll.min(max_scroll);
    // Same for sideways: only as far as the widest line reaches past the edge.
    let widest = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let max_h = widest.saturating_sub(inner.width as usize).min(u16::MAX as usize) as u16;
    state.max_hscroll.set(max_h);
    let hscroll = state.hscroll.min(max_h);
    f.render_widget(Paragraph::new(lines).scroll((scroll, hscroll)), inner);
}
