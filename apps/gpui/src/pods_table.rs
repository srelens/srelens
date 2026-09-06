//! The pod list's data: rows, their health, sorting, and the table delegate.
//!
//! Kept apart from the view so the parts that can be wrong in a testable way
//! — how a summary becomes a row, what counts as unhealthy, what a sort does
//! — are plain functions with plain tests, and the delegate is a thin owner
//! of them. The coding guide's separation: source data and IDs, sort state,
//! selection (the table's), viewport (the table's), and row rendering.

use gpui_kit::base::h_flex;
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use srelens_kube::workloads::PodSummary;

/// What a pod's status means, decided once here for this shell.
///
/// This duplicates a decision `@srelens/core`'s `podStatus` makes for the web
/// frontends — which waiting reasons are a failure and which are a pod on its
/// way up. `crates/kube` deliberately does not decide it (see `PodSummary::
/// waiting_reason`), so a second frontend has to, and this is the second copy.
/// That is exactly the drift the feasibility asked about; the fix is to move
/// the rule down into `crates/kube` so every shell reads one verdict. Until
/// then the rule is kept minimal and the copy is named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Running with nothing waiting.
    Running,
    /// A container is in a back-off or a config failure; the phase hides it.
    Failing(String),
    /// A container is waiting for an ordinary reason — pulling, creating.
    Starting(String),
    /// The pod itself failed.
    Failed,
    /// Pending, Succeeded, Unknown, anything else — shown as the phase says.
    Other(String),
}

impl Health {
    pub fn from_summary(phase: &str, waiting_reason: &str) -> Self {
        if phase == "Failed" {
            return Health::Failed;
        }
        if !waiting_reason.is_empty() {
            return match waiting_reason {
                "CrashLoopBackOff"
                | "ImagePullBackOff"
                | "ErrImagePull"
                | "CreateContainerConfigError"
                | "CreateContainerError"
                | "InvalidImageName"
                | "RunContainerError" => Health::Failing(waiting_reason.to_string()),
                other => Health::Starting(other.to_string()),
            };
        }
        match phase {
            "Running" => Health::Running,
            other => Health::Other(other.to_string()),
        }
    }

    /// The word shown in the Status column.
    pub fn label(&self) -> &str {
        match self {
            Health::Running => "Running",
            Health::Failing(reason) | Health::Starting(reason) | Health::Other(reason) => reason,
            Health::Failed => "Failed",
        }
    }
}

/// One row: a pod summary with its health decided and a stable key.
#[derive(Debug, Clone, PartialEq)]
pub struct PodRow {
    /// `namespace/name`: the row's identity for element IDs and selection,
    /// stable across sorting and refreshes. Never the row's index.
    pub key: SharedString,
    pub name: SharedString,
    pub namespace: SharedString,
    pub ready: SharedString,
    pub health: Health,
    pub restarts: i32,
    pub node: SharedString,
    pub age: SharedString,
    pub image: SharedString,
}

impl From<PodSummary> for PodRow {
    fn from(pod: PodSummary) -> Self {
        let health = Health::from_summary(&pod.phase, &pod.waiting_reason);
        Self {
            key: SharedString::from(format!("{}/{}", pod.namespace, pod.name)),
            name: pod.name.into(),
            namespace: pod.namespace.into(),
            ready: pod.ready.into(),
            health,
            restarts: pod.restarts,
            node: pod.node.into(),
            age: pod.age.into(),
            image: pod.image.into(),
        }
    }
}

/// Column keys, in display order. The delegate matches on these.
const NAME: &str = "name";
const NAMESPACE: &str = "namespace";
const READY: &str = "ready";
const STATUS: &str = "status";
const RESTARTS: &str = "restarts";
const NODE: &str = "node";
const AGE: &str = "age";
const IMAGE: &str = "image";

fn columns() -> Vec<Column> {
    vec![
        Column::new(NAME, "Name")
            .width(px(300.))
            .min_width(px(160.))
            .sortable(),
        Column::new(NAMESPACE, "Namespace").width(px(180.)),
        Column::new(READY, "Ready").width(px(72.)),
        Column::new(STATUS, "Status").width(px(180.)).sortable(),
        Column::new(RESTARTS, "Restarts")
            .width(px(88.))
            .text_right()
            .sortable(),
        Column::new(NODE, "Node").width(px(200.)),
        Column::new(AGE, "Age").width(px(72.)),
        Column::new(IMAGE, "Image").width(px(360.)),
    ]
}

/// Sort `rows` by one column. `ColumnSort::Default` is the caller's job — it
/// means "the order they arrived in", which this function no longer has.
pub fn sort_rows(rows: &mut [PodRow], key: &str, sort: ColumnSort) {
    let descending = match sort {
        ColumnSort::Ascending => false,
        ColumnSort::Descending => true,
        ColumnSort::Default => return,
    };
    match key {
        NAME => rows.sort_by(|a, b| a.name.cmp(&b.name)),
        STATUS => rows.sort_by(|a, b| a.health.label().cmp(b.health.label())),
        RESTARTS => rows.sort_by_key(|row| row.restarts),
        _ => return,
    }
    if descending {
        rows.reverse();
    }
}

/// The table's data owner. Holds the rows as they arrived and the rows as
/// they are shown, so a sort can be undone without a reload.
pub struct PodsDelegate {
    arrived: Vec<PodRow>,
    shown: Vec<PodRow>,
    columns: Vec<Column>,
    sort: Option<(usize, ColumnSort)>,
}

impl Default for PodsDelegate {
    fn default() -> Self {
        Self {
            arrived: Vec::new(),
            shown: Vec::new(),
            columns: columns(),
            sort: None,
        }
    }
}

impl PodsDelegate {
    /// Replace every row. The current sort, if any, is applied to the new
    /// rows, so a refresh does not silently unsort the table.
    pub fn set_rows(&mut self, rows: Vec<PodRow>) {
        self.arrived = rows;
        self.apply_sort();
    }

    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    fn apply_sort(&mut self) {
        self.shown = self.arrived.clone();
        if let Some((col_ix, sort)) = self.sort {
            if let Some(column) = self.columns.get(col_ix) {
                sort_rows(&mut self.shown, column.key.as_ref(), sort);
            }
        }
    }
}

impl TableDelegate for PodsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.shown.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort = match sort {
            ColumnSort::Default => None,
            other => Some((col_ix, other)),
        };
        self.apply_sort();
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        // Identity is the pod, not its position: the default `("row", ix)` would
        // hand one row's element state to another the moment the sort changed.
        let key = self
            .shown
            .get(row_ix)
            .map(|row| row.key.clone())
            .unwrap_or_else(|| SharedString::from(format!("missing-{row_ix}")));
        div().id(ElementId::Name(key))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(row) = self.shown.get(row_ix) else {
            return div().into_any_element();
        };
        let key = self.columns[col_ix].key.as_ref();
        match key {
            NAME => div().child(row.name.clone()).into_any_element(),
            NAMESPACE => div()
                .text_color(cx.theme().muted_foreground)
                .child(row.namespace.clone())
                .into_any_element(),
            READY => div().child(row.ready.clone()).into_any_element(),
            STATUS => {
                // A word and a dot in the same tone: the colour is never the
                // only signal, and the word is what a screen reader gets.
                let tone = match &row.health {
                    Health::Running => cx.theme().success,
                    Health::Failing(_) | Health::Failed => cx.theme().danger,
                    Health::Starting(_) => cx.theme().warning,
                    Health::Other(_) => cx.theme().muted_foreground,
                };
                h_flex()
                    .gap_2()
                    .text_color(tone)
                    .child("●")
                    .child(SharedString::from(row.health.label().to_string()))
                    .into_any_element()
            }
            RESTARTS => div()
                .child(SharedString::from(row.restarts.to_string()))
                .into_any_element(),
            NODE => div()
                .text_color(cx.theme().muted_foreground)
                .child(row.node.clone())
                .into_any_element(),
            AGE => div()
                .text_color(cx.theme().muted_foreground)
                .child(row.age.clone())
                .into_any_element(),
            IMAGE => div()
                .text_color(cx.theme().muted_foreground)
                .child(row.image.clone())
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .size_full()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child("No pods in this namespace")
    }
}

#[cfg(test)]
mod tests {
    // Named imports, not `use super::*`: the parent globs `gpui_kit::*`, which
    // carries a `test` attribute macro of its own, and inside this module a
    // bare `#[test]` then resolves to it and expands into itself until the
    // recursion limit. Std's `#[test]` and `#[gpui_kit::test]` both need
    // `test` to mean only one thing here.
    use super::{sort_rows, Health, PodRow, PodsDelegate, NAME, RESTARTS};
    use gpui_kit::component::table::{ColumnSort, TableDelegate};
    use gpui_kit::TestAppContext;
    use srelens_kube::workloads::PodSummary;

    fn pod(name: &str, phase: &str, waiting: &str, restarts: i32) -> PodSummary {
        PodSummary {
            name: name.into(),
            namespace: "checkout".into(),
            phase: phase.into(),
            ready: "1/1".into(),
            restarts,
            node: "n1".into(),
            created: None,
            age: "2d".into(),
            image: "acme/api:1".into(),
            waiting_reason: waiting.into(),
        }
    }

    #[test]
    fn a_running_pod_in_a_back_off_loop_is_failing_not_running() {
        // The phase says Running; the waiting reason says otherwise. The
        // reason wins, which is the whole point of carrying it.
        assert_eq!(
            Health::from_summary("Running", "CrashLoopBackOff"),
            Health::Failing("CrashLoopBackOff".into())
        );
        assert_eq!(Health::from_summary("Running", ""), Health::Running);
        assert_eq!(
            Health::from_summary("Pending", "ContainerCreating"),
            Health::Starting("ContainerCreating".into())
        );
        assert_eq!(Health::from_summary("Failed", ""), Health::Failed);
        assert_eq!(
            Health::from_summary("Succeeded", ""),
            Health::Other("Succeeded".into())
        );
    }

    #[test]
    fn a_row_keys_itself_by_namespace_and_name() {
        let row = PodRow::from(pod("api-1", "Running", "", 0));
        assert_eq!(row.key.as_ref(), "checkout/api-1");
        assert_eq!(row.health, Health::Running);
    }

    #[test]
    fn sorting_by_restarts_orders_numerically_and_reverses_for_descending() {
        let mut rows: Vec<PodRow> = [("a", 2), ("b", 10), ("c", 1)]
            .into_iter()
            .map(|(name, restarts)| PodRow::from(pod(name, "Running", "", restarts)))
            .collect();
        sort_rows(&mut rows, RESTARTS, ColumnSort::Ascending);
        assert_eq!(
            rows.iter().map(|r| r.restarts).collect::<Vec<_>>(),
            vec![1, 2, 10]
        );
        sort_rows(&mut rows, RESTARTS, ColumnSort::Descending);
        assert_eq!(
            rows.iter().map(|r| r.restarts).collect::<Vec<_>>(),
            vec![10, 2, 1]
        );
    }

    #[test]
    fn clearing_the_sort_restores_arrival_order_and_a_refresh_keeps_the_sort() {
        let mut delegate = PodsDelegate::default();
        let arrivals = |names: &[&str]| -> Vec<PodRow> {
            names
                .iter()
                .enumerate()
                .map(|(ix, name)| PodRow::from(pod(name, "Running", "", ix as i32)))
                .collect()
        };
        delegate.set_rows(arrivals(&["c", "a", "b"]));
        assert_eq!(
            delegate
                .shown
                .iter()
                .map(|r| r.name.as_ref())
                .collect::<Vec<_>>(),
            ["c", "a", "b"]
        );

        // Column 0 is Name.
        delegate.sort = Some((0, ColumnSort::Ascending));
        delegate.apply_sort();
        assert_eq!(
            delegate
                .shown
                .iter()
                .map(|r| r.name.as_ref())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        // New rows arrive; the sort holds.
        delegate.set_rows(arrivals(&["z", "y"]));
        assert_eq!(
            delegate
                .shown
                .iter()
                .map(|r| r.name.as_ref())
                .collect::<Vec<_>>(),
            ["y", "z"]
        );

        // Back to Default: the order they arrived in.
        delegate.sort = None;
        delegate.apply_sort();
        assert_eq!(
            delegate
                .shown
                .iter()
                .map(|r| r.name.as_ref())
                .collect::<Vec<_>>(),
            ["z", "y"]
        );
    }

    #[gpui_kit::test]
    fn the_delegate_reports_its_shape_to_the_table(cx: &mut TestAppContext) {
        let mut delegate = PodsDelegate::default();
        delegate.set_rows(vec![PodRow::from(pod("api-1", "Running", "", 0))]);
        cx.update(|cx| {
            assert_eq!(delegate.columns_count(cx), 8);
            assert_eq!(delegate.rows_count(cx), 1);
            assert_eq!(delegate.column(0, cx).key.as_ref(), NAME);
        });
    }
}
