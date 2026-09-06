//! The pod list: one context, one namespace, the pods in it.
//!
//! Written down before the screen, as the design guide asks:
//!
//! - **Primary task.** See the pods of one namespace on one cluster and tell
//!   the unhealthy ones from the rest without reading every row.
//! - **Object.** Pods. The context and namespace are the scope, not the
//!   object, so they sit in the toolbar and the table gets the window.
//! - **Actions that stay available.** Change the context, change the
//!   namespace, refresh. Nothing per row yet: this increment is about whether
//!   the list itself is right, and a row action with no place to land would be
//!   a capability exposed because the backend has it.
//! - **Information for a decision.** Name, namespace, ready count, status
//!   with the waiting reason the phase hides, restarts, node, age, image.
//! - **States.** No kubeconfig contexts at all; loading contexts, namespaces
//!   or pods; a load that failed, with the backend's words and a way to retry;
//!   an empty namespace.
//! - **Keyboard path.** Tab reaches the two pickers and Refresh in order; the
//!   table owns arrow navigation and row selection through `TableState`.
//!
//! State lives in the narrowest owner that keeps it correct: the pickers own
//! their open/close and search, the table owns selection, scroll and sort
//! coordination, and this view owns what is being looked at and whether it
//! has arrived. Every load is stamped with a revision; a result for an older
//! revision is dropped, so switching context twice quickly never paints the
//! first cluster's pods under the second cluster's name.

use gpui_kit::base::{h_flex, v_flex, IndexPath};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::table::{DataTable, TableState};
use gpui_kit::component::{ActiveTheme as _, Root, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::kube_bridge::KubeBridge;
use crate::pods_table::{PodRow, PodsDelegate};

type NamesSelect = SelectState<Vec<String>>;

/// Which namespace to open on, out of the ones the cluster has.
///
/// `SRELENS_GPUI_NAMESPACE` wins when it names one of them: a development
/// override so a screenshot or a check can land on a namespace with something
/// in it without driving the picker first. Then `default`, then the first.
/// A context's own declared namespace is the rule this should learn next;
/// it needs the chosen `ContextDto` kept around, which the first increment
/// does not yet do.
fn initial_namespace(names: &[String]) -> usize {
    if let Ok(wanted) = std::env::var("SRELENS_GPUI_NAMESPACE") {
        if let Some(ix) = names.iter().position(|n| *n == wanted) {
            return ix;
        }
    }
    names.iter().position(|n| n == "default").unwrap_or(0)
}

pub struct Workspace {
    contexts: Entity<NamesSelect>,
    namespaces: Entity<NamesSelect>,
    table: Entity<TableState<PodsDelegate>>,
    /// What is being looked at. `None` until the pickers have something.
    context: Option<String>,
    namespace: Option<String>,
    /// Whether any load is in flight; drives the Refresh button's state.
    busy: bool,
    /// The last failure, in the backend's words, until the next load starts.
    error: Option<String>,
    /// Stamped onto every load; a result carrying an older stamp is dropped.
    revision: u64,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let contexts = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
        let namespaces = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
        let table = cx.new(|cx| {
            TableState::new(PodsDelegate::default(), window, cx)
                .row_selectable(true)
                .col_resizable(true)
                .sortable(true)
        });

        let subscriptions = vec![
            cx.subscribe_in(&contexts, window, |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(name)) = event {
                    this.choose_context(name.clone(), window, cx);
                }
            }),
            cx.subscribe_in(&namespaces, window, |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(name)) = event {
                    this.choose_namespace(name.clone(), window, cx);
                }
            }),
        ];

        let mut this = Self {
            contexts,
            namespaces,
            table,
            context: None,
            namespace: None,
            busy: false,
            error: None,
            revision: 0,
            _subscriptions: subscriptions,
        };
        this.load_contexts(window, cx);
        this
    }

    /// A new load begins: everything older is now stale.
    fn begin(&mut self, cx: &mut Context<Self>) -> u64 {
        self.revision += 1;
        self.busy = true;
        self.error = None;
        cx.notify();
        self.revision
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = Some(message);
        cx.notify();
    }

    fn load_contexts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self.begin(cx);
        let work = cx.global::<KubeBridge>().contexts();
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            this.update_in(cx, |this, window, cx| {
                if revision != this.revision {
                    return;
                }
                match result {
                    Ok(list) => {
                        let names: Vec<String> = list.iter().map(|c| c.name.clone()).collect();
                        // The kubeconfig's current context first, else the first listed.
                        let pick = list.iter().position(|c| c.is_current).unwrap_or(0);
                        let chosen = names.get(pick).cloned();
                        this.contexts.update(cx, |select, cx| {
                            select.set_items(names, window, cx);
                            select.set_selected_index(
                                chosen.as_ref().map(|_| IndexPath::new(pick)),
                                window,
                                cx,
                            );
                        });
                        match chosen {
                            Some(name) => this.choose_context(name, window, cx),
                            None => this
                                .fail("No contexts in any kubeconfig on this machine".into(), cx),
                        }
                    }
                    Err(message) => this.fail(message, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn choose_context(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        // Setting the picker's index from a load reports back as a choice, and
        // `load_contexts` also calls this directly, so the same name arrives
        // twice. Choosing what is already chosen is not a change — unless the
        // last load of it failed, in which case choosing it again is the retry.
        if self.context.as_deref() == Some(name.as_str()) && self.error.is_none() {
            return;
        }
        self.context = Some(name);
        self.namespace = None;
        self.load_namespaces(window, cx);
    }

    fn load_namespaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context.clone() else {
            return;
        };
        let revision = self.begin(cx);
        let work = cx.global::<KubeBridge>().namespaces(context);
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            this.update_in(cx, |this, window, cx| {
                if revision != this.revision {
                    return;
                }
                match result {
                    Ok(names) => {
                        let pick = initial_namespace(&names);
                        let chosen = names.get(pick).cloned();
                        this.namespaces.update(cx, |select, cx| {
                            select.set_items(names, window, cx);
                            select.set_selected_index(
                                chosen.as_ref().map(|_| IndexPath::new(pick)),
                                window,
                                cx,
                            );
                        });
                        match chosen {
                            Some(name) => this.choose_namespace(name, window, cx),
                            None => this.fail("This cluster has no namespaces".into(), cx),
                        }
                    }
                    Err(message) => this.fail(message, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn choose_namespace(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.namespace.as_deref() == Some(name.as_str()) && self.error.is_none() {
            return;
        }
        self.namespace = Some(name);
        self.load_pods(window, cx);
    }

    fn load_pods(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(context), Some(namespace)) = (self.context.clone(), self.namespace.clone())
        else {
            return;
        };
        let revision = self.begin(cx);
        let work = cx.global::<KubeBridge>().pods(context, namespace);
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            this.update_in(cx, |this, _, cx| {
                if revision != this.revision {
                    return;
                }
                match result {
                    Ok(pods) => {
                        let rows: Vec<PodRow> = pods.into_iter().map(PodRow::from).collect();
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_rows(rows);
                            table.refresh(cx);
                        });
                        this.busy = false;
                        cx.notify();
                    }
                    Err(message) => this.fail(message, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Refresh re-reads the pods of the current scope. It does not re-read
    /// the pickers: a context that vanished mid-session is a rarer event than
    /// a pod that did, and it has its own remedy (restart) for now.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_pods(window, cx);
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.table.read(cx).delegate();
        let summary: SharedString = match (&self.namespace, self.busy) {
            (_, true) => "Loading…".into(),
            (Some(_), false) if delegate.is_empty() => "No pods".into(),
            (Some(_), false) if delegate.len() == 1 => "1 pod".into(),
            (Some(_), false) => format!("{} pods", delegate.len()).into(),
            (None, false) => "".into(),
        };
        h_flex()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Label::new("Pods").text_sm())
            // Named for assistive technology as well as for the eye. Without
            // `accessibility_label` these reached Windows UI Automation as two
            // ComboBoxes with empty names — measured with AccessKit's bridge,
            // which is what GPUI exposes on Windows.
            .child(
                Select::new(&self.contexts)
                    .placeholder("Context")
                    .accessibility_label("Context")
                    .small()
                    .w_64(),
            )
            .child(
                Select::new(&self.namespaces)
                    .placeholder("Namespace")
                    .accessibility_label("Namespace")
                    .small()
                    .w_56(),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(summary),
            )
            .child(
                Button::new("refresh")
                    .outline()
                    .small()
                    .label("Refresh")
                    .loading(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_toolbar(cx))
            .when_some(self.error.clone(), |this, message| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .child(Alert::error("load-error", message)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).small()),
            )
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
