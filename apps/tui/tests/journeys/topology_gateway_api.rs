//! Opening Gateway API nodes from the topology (#1089).
//!
//! A kind name is not an identity: an implementation may ship its own
//! `Gateway` CRD in another group. Enter, `d` and `y` on a Gateway API node
//! must read the `gateway.networking.k8s.io` object, never the same-named one.
//!
//! Dimensions: D1 (lane hopping before the key), D2 (colliding `Gateway`
//! kinds, the other group discovered first), D6 (with no cluster, the view
//! says the read failed rather than showing an empty manifest), D7 (rendered
//! buffer). D3–D5 do not apply: the key opens a view synchronously from a
//! graph already on screen, with no context switch or late arrival involved.

use crossterm::event::KeyCode;
use srectl::app::ActiveView;
use srectl::commands::CrdMeta;
use srectl::views::topology_view::TopologyViewState;
use srelens_kube::topology::{Health, Lane, TopologyGraphOut, TopologyNode};

use crate::common::{self, ch, key, press, render_app};

const G: &str = "gateway.networking.k8s.io";

fn crd(group: &str, version: &str, kind: &str) -> CrdMeta {
    CrdMeta {
        crd_name: format!("{}s.{group}", kind.to_lowercase()),
        group: group.to_string(),
        version: version.to_string(),
        kind: kind.to_string(),
        plural: format!("{}s", kind.to_lowercase()),
        singular: kind.to_lowercase(),
        namespaced: true,
        short_names: vec![],
        printer_columns: vec![],
        created_at: None,
    }
}

fn node(kind: &str, name: &str, lane: Lane) -> TopologyNode {
    TopologyNode {
        id: format!("{kind}/shop/{name}"),
        kind: kind.to_string(),
        name: name.to_string(),
        namespace: "shop".to_string(),
        lane,
        detail: String::new(),
        ready: None,
        desired: None,
        health: Health::Ok,
    }
}

/// An app on the topology: a Gateway, an HTTPRoute and an Ingress, with an
/// `acme.io` Gateway CRD discovered ahead of the Gateway API's own.
/// Holds the settings guard itself: callers keep it for the whole test.
async fn topology_app() -> (common::env::SettingsGuard, srectl::app::App) {
    let settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.crds = vec![
        crd("acme.io", "v1", "Gateway"),
        crd(G, "v1beta1", "Gateway"),
        crd(G, "v1beta1", "HTTPRoute"),
    ];
    let mut topo = TopologyViewState::new(vec!["shop".to_string()]);
    topo.set_graph(TopologyGraphOut {
        nodes: vec![
            node("Gateway", "edge", Lane::Gateway),
            node("HTTPRoute", "store", Lane::Route),
            node("Ingress", "legacy", Lane::Route),
        ],
        edges: vec![],
        probe: None,
        unread: vec![],
    });
    app.active_view = ActiveView::Topology(topo);
    (settings, app)
}

fn pinned(app: &srectl::app::App) -> (&'static str, Option<String>) {
    match &app.active_view {
        ActiveView::Yaml(y) => ("yaml", y.pinned_api_version.clone()),
        ActiveView::Describe(d) => ("describe", d.pinned_api_version.clone()),
        _ => panic!("expected a YAML or describe view"),
    }
}

#[tokio::test]
async fn journey_d2_gateway_api_nodes_open_in_their_own_group() {
    let (_settings, mut app) = topology_app().await;
    let want = Some(format!("{G}/v1beta1"));

    // D1: hop lanes and back before asking, as a hand would.
    press(&mut app, key(KeyCode::Tab)).await;
    press(&mut app, key(KeyCode::BackTab)).await;
    press(&mut app, ch('y')).await;
    assert_eq!(pinned(&app), ("yaml", want.clone()));
    // D6 + D7: no cluster here, so the view names the failed read, on screen.
    let screen = render_app(&mut app, 120, 30);
    assert!(screen.contains("YAML: Gateway/edge"), "{screen}");
    let ActiveView::Yaml(ref y) = app.active_view else {
        unreachable!()
    };
    assert!(
        y.load_error.is_some(),
        "a failed read must not look like a manifest"
    );

    press(&mut app, key(KeyCode::Esc)).await;
    press(&mut app, ch('d')).await;
    assert_eq!(pinned(&app), ("describe", want.clone()));

    press(&mut app, key(KeyCode::Esc)).await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(pinned(&app), ("describe", want.clone()));

    // The HTTPRoute, first in the routes lane, by every key.
    press(&mut app, key(KeyCode::Esc)).await;
    press(&mut app, key(KeyCode::Tab)).await;
    for k in [ch('y'), ch('d'), key(KeyCode::Enter)] {
        press(&mut app, k).await;
        assert_eq!(pinned(&app).1, want, "HTTPRoute by {k:?}");
        press(&mut app, key(KeyCode::Esc)).await;
        assert!(
            matches!(app.active_view, ActiveView::Topology(_)),
            "Esc returns to the topology"
        );
    }
}

#[tokio::test]
async fn journey_d2_an_ingress_beside_them_keeps_the_built_in_lookup() {
    let (_settings, mut app) = topology_app().await;
    press(&mut app, key(KeyCode::Tab)).await;
    press(&mut app, ch('j')).await;
    press(&mut app, ch('y')).await;
    let ActiveView::Yaml(ref y) = app.active_view else {
        panic!("expected YAML")
    };
    assert_eq!(
        (y.resource_kind.as_str(), y.pinned_api_version.as_deref()),
        ("Ingress", None)
    );
}
