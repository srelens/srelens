use crossterm::event::KeyCode;
use srectl::app::ActiveView;
use srelens_kube::lineage::{LineageNode, LineageRelation, NodeOutcome};

use crate::common::{self, ch, key, press};

/// Build a synthetic 3-tier hierarchy: Deployment -> ReplicaSet -> Pod -> Container
fn mock_deployment_tree() -> LineageNode {
    let mut root = LineageNode::new(
        "Deployment",
        "web-app",
        Some("prod".into()),
        LineageRelation::Owner,
    )
    .with_api_version("apps/v1")
    .with_uid("uid-deploy-1")
    .with_status(Some("Ready: 3/3".into()), Some("Up-to-date: 3".into()))
    .with_outcome(NodeOutcome::Healthy);

    let mut rs = LineageNode::new(
        "ReplicaSet",
        "web-app-7b8c9d",
        Some("prod".into()),
        LineageRelation::Owner,
    )
    .with_api_version("apps/v1")
    .with_uid("uid-rs-1")
    .with_status(Some("Ready: 3/3".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    let mut pod = LineageNode::new(
        "Pod",
        "web-app-7b8c9d-x1",
        Some("prod".into()),
        LineageRelation::Target,
    )
    .with_api_version("v1")
    .with_uid("uid-pod-1")
    .with_status(Some("Running".into()), Some("Ready: 1/1".into()))
    .with_outcome(NodeOutcome::Healthy);

    let container = LineageNode::new(
        "Container",
        "web",
        Some("prod".into()),
        LineageRelation::Child,
    )
    .with_status(Some("Running".into()), Some("restarts: 0".into()))
    .with_outcome(NodeOutcome::Healthy);

    pod.children.push(container);
    rs.children.push(pod);
    root.children.push(rs);
    root
}

/// Build a synthetic CRD hierarchy: VirtualMachine -> VirtualMachineInstance -> Pod
fn mock_crd_tree() -> LineageNode {
    let mut vm = LineageNode::new(
        "VirtualMachine",
        "vm-workload",
        Some("default".into()),
        LineageRelation::Owner,
    )
    .with_api_version("kubevirt.io/v1")
    .with_uid("uid-vm-1")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    let mut vmi = LineageNode::new(
        "VirtualMachineInstance",
        "vm-workload",
        Some("default".into()),
        LineageRelation::Target,
    )
    .with_api_version("kubevirt.io/v1")
    .with_uid("uid-vmi-1")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    let pod = LineageNode::new(
        "Pod",
        "virt-launcher-vm-workload-abc",
        Some("default".into()),
        LineageRelation::Child,
    )
    .with_api_version("v1")
    .with_uid("uid-pod-launcher")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    vmi.children.push(pod);
    vm.children.push(vmi);
    vm
}

/// Build a synthetic CRD hierarchy with VirtualMachine as the Target root
fn mock_vm_crd_tree() -> LineageNode {
    let mut vm = LineageNode::new(
        "VirtualMachine",
        "vm-workload",
        Some("default".into()),
        LineageRelation::Target,
    )
    .with_api_version("kubevirt.io/v1")
    .with_uid("uid-vm-1")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    let mut vmi = LineageNode::new(
        "VirtualMachineInstance",
        "vm-workload",
        Some("default".into()),
        LineageRelation::Child,
    )
    .with_api_version("kubevirt.io/v1")
    .with_uid("uid-vmi-1")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    let pod = LineageNode::new(
        "Pod",
        "virt-launcher-vm-workload-abc",
        Some("default".into()),
        LineageRelation::Child,
    )
    .with_api_version("v1")
    .with_uid("uid-pod-launcher")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    vmi.children.push(pod);
    vm.children.push(vmi);
    vm
}

// ---------------------------------------------------------------------------
// D1: Input Disorder & Interleaving (The "Messy Human")
// Human opens tree for a Pod, navigates up to parent Deployment with 'k' / Up arrow,
// then presses 't' to re-root the tree onto the Deployment.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d1_in_tree_reroot_navigates_to_selected_ancestor() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // 1. Open tree for a Pod
    app.open_resource_tree(
        "Pod".into(),
        "web-app-7b8c9d-x1".into(),
        Some("prod".into()),
        None,
    );
    app.handle_lineage_result("Pod", "web-app-7b8c9d-x1", Ok(mock_deployment_tree()));

    // Target Pod is selected by default (index 2: Deployment [0], ReplicaSet [1], Pod [2], Container [3])
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "Pod");
    assert_eq!(tree.selected_node().unwrap().name, "web-app-7b8c9d-x1");

    // 2. Navigate up with 'k' twice to reach Deployment
    press(&mut app, ch('k')).await;
    press(&mut app, ch('k')).await;

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "Deployment");
    assert_eq!(tree.selected_node().unwrap().name, "web-app");

    // 3. Press 't' to re-root on the highlighted Deployment
    press(&mut app, ch('t')).await;

    let ActiveView::Tree(ref new_tree) = app.active_view else {
        panic!("expected re-rooted ActiveView::Tree");
    };
    assert_eq!(new_tree.root_kind, "Deployment");
    assert_eq!(new_tree.root_name, "web-app");
    assert_eq!(new_tree.api_version.as_deref(), Some("apps/v1"));
    assert!(new_tree.is_loading, "new tree must be in loading state");
}

// ---------------------------------------------------------------------------
// D2: Ambiguity & Collisions
// A Custom Resource Definition whose kind is identical to a built-in or has a custom group
// preserves its exact `crd_api_version` when opened via 't' from table.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d2_crd_preserves_custom_api_version_on_open() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // Open resource tree with explicit custom api_version
    app.open_resource_tree(
        "VirtualMachine".into(),
        "vm-workload".into(),
        Some("default".into()),
        Some("kubevirt.io/v1".into()),
    );

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.root_kind, "VirtualMachine");
    assert_eq!(tree.root_name, "vm-workload");
    assert_eq!(tree.api_version.as_deref(), Some("kubevirt.io/v1"));
}

// ---------------------------------------------------------------------------
// D3: Temporal & Asynchrony
// A late-arriving LineageResult for an old/cancelled request is ignored and does not
// overwrite or corrupt the currently active tree view.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d3_stale_lineage_results_are_ignored() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // Open tree for target "worker-2"
    app.open_resource_tree("Pod".into(), "worker-2".into(), Some("prod".into()), None);

    // Stale result for "worker-1" arrives
    app.handle_lineage_result("Pod", "worker-1", Ok(mock_deployment_tree()));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert!(tree.is_loading, "tree must still be loading for worker-2");
    assert_eq!(tree.root_name, "worker-2");
    assert!(
        tree.nodes.is_empty(),
        "nodes must not be populated by stale worker-1 result"
    );

    // Correct result arrives for worker-2
    let mut worker_2_tree = mock_deployment_tree();
    worker_2_tree.name = "worker-2".into();
    app.handle_lineage_result("Pod", "worker-2", Ok(worker_2_tree));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert!(!tree.is_loading, "tree must no longer be loading");
    assert_eq!(tree.root_name, "worker-2");
    assert!(!tree.nodes.is_empty(), "nodes must now be populated");
}

// ---------------------------------------------------------------------------
// D4: Boundary & Scale Extremes
// An orphaned child whose parent was deleted (404 Not Found) renders without crash,
// showing the missing parent with [NOT FOUND] outcome.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d4_orphaned_deleted_parent_renders_not_found_badge() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    app.open_resource_tree(
        "VirtualMachineInstance".into(),
        "vmi-orphaned".into(),
        Some("default".into()),
        Some("kubevirt.io/v1".into()),
    );

    let mut parent = LineageNode::new(
        "VirtualMachine",
        "vm-deleted",
        Some("default".into()),
        LineageRelation::Owner,
    )
    .with_api_version("kubevirt.io/v1")
    .with_uid("uid-vm-deleted")
    .with_outcome(NodeOutcome::NotFound);

    let child = LineageNode::new(
        "VirtualMachineInstance",
        "vmi-orphaned",
        Some("default".into()),
        LineageRelation::Target,
    )
    .with_api_version("kubevirt.io/v1")
    .with_uid("uid-vmi-orphaned")
    .with_status(Some("Running".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    parent.children.push(child);

    app.handle_lineage_result("VirtualMachineInstance", "vmi-orphaned", Ok(parent));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.nodes.len(), 2);
    assert_eq!(tree.nodes[0].name, "vm-deleted");
    assert_eq!(tree.nodes[0].fetch_outcome, Some(NodeOutcome::NotFound));
    assert_eq!(tree.nodes[1].name, "vmi-orphaned");
    assert_eq!(tree.nodes[1].fetch_outcome, Some(NodeOutcome::Healthy));

    let text = tree.tree_as_text();
    assert!(text.contains("VirtualMachine/vm-deleted [OWNER] [NOT FOUND] [status: -]"));
}

// ---------------------------------------------------------------------------
// D5: State Transitions & Re-Entry
// Pressing Esc on ActiveView::Tree returns to the previous view on the nav stack.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d5_esc_returns_to_previous_view_on_nav_stack() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // Verify initial view is Table
    assert!(matches!(app.active_view, ActiveView::Table(_)));

    // Open tree view
    app.open_resource_tree("Pod".into(), "web-app".into(), Some("prod".into()), None);
    assert!(matches!(app.active_view, ActiveView::Tree(_)));

    // Press Esc to exit tree view
    press(&mut app, key(KeyCode::Esc)).await;

    // Must return to Table view
    assert!(matches!(app.active_view, ActiveView::Table(_)));
}

// ---------------------------------------------------------------------------
// D6: Feedback & Truthfulness ("Say What You Know")
// 403 Forbidden, 404 Not Found, and TimedOut are never collapsed into absence
// or rendered as generic "missing". Each renders its truthful badge.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d6_say_what_you_know_distinct_outcome_badges() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    app.open_resource_tree(
        "Pod".into(),
        "test-pod".into(),
        Some("default".into()),
        None,
    );

    let mut root = LineageNode::new(
        "Parent403",
        "p1",
        Some("default".into()),
        LineageRelation::Owner,
    )
    .with_outcome(NodeOutcome::Forbidden);

    let mut middle = LineageNode::new(
        "ParentTimeout",
        "p2",
        Some("default".into()),
        LineageRelation::Owner,
    )
    .with_outcome(NodeOutcome::TimedOut);

    let target = LineageNode::new(
        "Pod",
        "test-pod",
        Some("default".into()),
        LineageRelation::Target,
    )
    .with_outcome(NodeOutcome::Healthy);

    middle.children.push(target);
    root.children.push(middle);

    app.handle_lineage_result("Pod", "test-pod", Ok(root));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };

    let text = tree.tree_as_text();
    assert!(
        text.contains("Parent403/p1 [OWNER] [FORBIDDEN] [status: -]"),
        "Forbidden must be explicitly named"
    );
    assert!(
        text.contains("ParentTimeout/p2 [OWNER] [TIMED OUT] [status: -]"),
        "TimedOut must be explicitly named"
    );
}

// ---------------------------------------------------------------------------
// D7: Visual & Perceptual Reality
// Render the terminal buffer at 80x24 standard geometry and assert:
// - Title header displays "Resource Relationship Tree: VirtualMachine/vm-workload default"
// - Tree branch glyphs are visible
// - Status and relationship badges are visible
// - Help bar contains "<t> Re-root"
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d7_rendered_buffer_reality_on_80x24_terminal() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    app.open_resource_tree(
        "VirtualMachine".into(),
        "vm-workload".into(),
        Some("default".into()),
        Some("kubevirt.io/v1".into()),
    );
    app.handle_lineage_result("VirtualMachine", "vm-workload", Ok(mock_crd_tree()));

    // Render terminal buffer at 80 columns by 24 rows
    let rendered = common::render_app(&mut app, 80, 24);

    // Verify Title & Kind
    assert!(
        rendered.contains("Resource Relationship Tree: VirtualMachine/vm-workload default"),
        "Title header must name the target resource and namespace"
    );

    // Verify Branch Glyphs & Hierarchy Nodes
    assert!(
        rendered.contains("VirtualMachine/vm-workload"),
        "Root VM must be visible"
    );
    assert!(
        rendered.contains("VirtualMachineInstance/vm-workload"),
        "Child VMI must be visible"
    );
    assert!(
        rendered.contains("Pod/virt-launcher-vm-workload-abc"),
        "Grandchild launcher Pod must be visible"
    );

    // Verify Badges
    assert!(
        rendered.contains("[OWNER]"),
        "[OWNER] badge must be rendered"
    );
    assert!(
        rendered.contains("[TARGET]"),
        "[TARGET] badge must be rendered"
    );
    assert!(
        rendered.contains("[CHILD]"),
        "[CHILD] badge must be rendered"
    );
    assert!(
        rendered.contains("[● Running]"),
        "Running status badge must be rendered"
    );

    // Verify Footer Help Bar
    assert!(
        rendered.contains("<t> Re-root"),
        "Footer help bar must expose <t> Re-root action"
    );
    assert!(
        rendered.contains("<Enter> Jump"),
        "Footer help bar must expose <Enter> Jump action"
    );
}

#[tokio::test]
async fn journey_d1_container_reroot_shows_toast_without_corrupting_state() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // Open standard deployment tree
    app.open_resource_tree(
        "Pod".into(),
        "web-app-7b8c9d-x1".into(),
        Some("prod".into()),
        None,
    );
    app.handle_lineage_result("Pod", "web-app-7b8c9d-x1", Ok(mock_deployment_tree()));

    // Target Pod is at index 2. Move down with 'j' to index 3 (Container/nginx)
    press(&mut app, ch('j')).await;

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "Container");

    // Press 't' to attempt re-rooting on a container
    press(&mut app, ch('t')).await;

    // Verify toast is set with truthful warning
    assert!(
        app.toast.as_ref().map_or(false, |(msg, _, _)| msg
            .contains("Cannot re-root tree on a container")),
        "Must show toast explaining that containers cannot be re-rooted"
    );

    // Verify active view is still Tree for Pod and not corrupted
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("ActiveView must remain Tree");
    };
    assert_eq!(tree.root_kind, "Pod");
    assert_eq!(tree.root_name, "web-app-7b8c9d-x1");
}

// ---------------------------------------------------------------------------
// D1: Input Disorder & Interleaving (CRD Finger Chaos)
// Human re-roots from child CRD to parent CRD, then back down to child CRD,
// verifying api_version is strictly preserved and view remains intact.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d1_crd_reroot_child_to_parent_and_back() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // 1. Open tree for VirtualMachineInstance
    app.open_resource_tree(
        "VirtualMachineInstance".into(),
        "vm-workload".into(),
        Some("default".into()),
        Some("kubevirt.io/v1".into()),
    );
    app.handle_lineage_result("VirtualMachineInstance", "vm-workload", Ok(mock_crd_tree()));

    // Target VMI is selected (VirtualMachine [0], VirtualMachineInstance [1], Pod [2])
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "VirtualMachineInstance");
    assert_eq!(
        tree.selected_node().unwrap().api_version.as_deref(),
        Some("kubevirt.io/v1")
    );

    // 2. Navigate up with 'k' to parent VirtualMachine
    press(&mut app, ch('k')).await;
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "VirtualMachine");

    // 3. Re-root on parent VirtualMachine with 't'
    press(&mut app, ch('t')).await;
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.root_kind, "VirtualMachine");
    assert_eq!(tree.root_name, "vm-workload");
    assert_eq!(tree.api_version.as_deref(), Some("kubevirt.io/v1"));
    assert!(tree.is_loading);

    // Provide lineage result for parent VirtualMachine
    app.handle_lineage_result("VirtualMachine", "vm-workload", Ok(mock_vm_crd_tree()));

    // 4. Navigate down with 'j' back to child VirtualMachineInstance
    press(&mut app, ch('j')).await;
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "VirtualMachineInstance");

    // 5. Re-root on child VirtualMachineInstance with 't'
    press(&mut app, ch('t')).await;
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.root_kind, "VirtualMachineInstance");
    assert_eq!(tree.root_name, "vm-workload");
    assert_eq!(tree.api_version.as_deref(), Some("kubevirt.io/v1"));
}

// ---------------------------------------------------------------------------
// D1: Input Disorder & Interleaving (CRD Logs Error Shielding)
// Human presses 'l' (Logs) on a non-pod CRD node; asserts a truthful warning toast
// without panicking, corrupting state, or exiting Tree view.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d1_crd_logs_attempt_warns_truthfully_without_crashing() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    app.open_resource_tree(
        "VirtualMachine".into(),
        "vm-workload".into(),
        Some("default".into()),
        Some("kubevirt.io/v1".into()),
    );
    app.handle_lineage_result("VirtualMachine", "vm-workload", Ok(mock_crd_tree()));

    // Move to root VirtualMachine
    press(&mut app, ch('g')).await;
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.selected_node().unwrap().kind, "VirtualMachine");

    // Press 'l' on VirtualMachine CRD
    press(&mut app, ch('l')).await;

    assert!(
        app.toast.as_ref().map_or(false, |(msg, _, _)| msg
            .contains("Logs only available for Pods (selected VirtualMachine)")),
        "Must warn truthfully that logs are only for Pods"
    );

    // Tree view must remain intact
    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree intact");
    };
    assert_eq!(tree.root_kind, "VirtualMachine");
    assert_eq!(tree.nodes.len(), 3);
}

// ---------------------------------------------------------------------------
// D2: Ambiguity & Collisions (CRD Colliding with Built-in Kind)
// A Custom Resource whose kind is "Deployment" with group "acme.io/v1" preserves
// its custom api_version in tree view, rendered buffers, and navigation.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d2_crd_colliding_with_builtin_kind_preserves_custom_group() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    let mut custom_deploy = LineageNode::new(
        "Deployment",
        "custom-agent",
        Some("custom-ns".into()),
        LineageRelation::Owner,
    )
    .with_api_version("acme.io/v1")
    .with_uid("uid-acme-deploy")
    .with_status(Some("Synced".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    let child_custom_ds = LineageNode::new(
        "DaemonSet",
        "custom-ds",
        Some("custom-ns".into()),
        LineageRelation::Target,
    )
    .with_api_version("acme.io/v1")
    .with_uid("uid-acme-ds")
    .with_status(Some("Ready".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    custom_deploy.children.push(child_custom_ds);

    app.open_resource_tree(
        "Deployment".into(),
        "custom-agent".into(),
        Some("custom-ns".into()),
        Some("acme.io/v1".into()),
    );
    app.handle_lineage_result("Deployment", "custom-agent", Ok(custom_deploy));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };

    assert_eq!(tree.root_kind, "Deployment");
    assert_eq!(tree.api_version.as_deref(), Some("acme.io/v1"));
    assert_eq!(tree.nodes[0].api_version.as_deref(), Some("acme.io/v1"));
    assert_eq!(tree.nodes[1].api_version.as_deref(), Some("acme.io/v1"));

    let rendered = common::render_app(&mut app, 100, 24);
    assert!(rendered.contains("Deployment/custom-agent custom-ns"));
    assert!(rendered.contains("DaemonSet/custom-ds"));
}

// ---------------------------------------------------------------------------
// D4: Boundary & Scale Extremes (Nested Same-Kind CRD Traversal)
// ArgoCD nested Applications: Root Application -> Child Application -> Deployment
// Verifies same-kind CRDs at different hierarchy levels resolve and display properly.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d4_nested_same_kind_crd_traversal() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    let mut root_app = LineageNode::new(
        "Application",
        "root-infra",
        Some("argocd".into()),
        LineageRelation::Owner,
    )
    .with_api_version("argoproj.io/v1alpha1")
    .with_uid("uid-app-root")
    .with_status(Some("Synced".into()), Some("Healthy".into()))
    .with_outcome(NodeOutcome::Healthy);

    let mut sub_app = LineageNode::new(
        "Application",
        "sub-cluster-apps",
        Some("argocd".into()),
        LineageRelation::Target,
    )
    .with_api_version("argoproj.io/v1alpha1")
    .with_uid("uid-app-sub")
    .with_status(Some("Synced".into()), Some("Healthy".into()))
    .with_outcome(NodeOutcome::Healthy);

    let deploy = LineageNode::new(
        "Deployment",
        "frontend-proxy",
        Some("production".into()),
        LineageRelation::Child,
    )
    .with_api_version("apps/v1")
    .with_uid("uid-deploy-proxy")
    .with_status(Some("Ready: 2/2".into()), None)
    .with_outcome(NodeOutcome::Healthy);

    sub_app.children.push(deploy);
    root_app.children.push(sub_app);

    app.open_resource_tree(
        "Application".into(),
        "sub-cluster-apps".into(),
        Some("argocd".into()),
        Some("argoproj.io/v1alpha1".into()),
    );
    app.handle_lineage_result("Application", "sub-cluster-apps", Ok(root_app));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };

    assert_eq!(tree.nodes.len(), 3);
    assert_eq!(tree.nodes[0].kind, "Application");
    assert_eq!(tree.nodes[0].name, "root-infra");
    assert_eq!(tree.nodes[0].relationship, LineageRelation::Owner);

    assert_eq!(tree.nodes[1].kind, "Application");
    assert_eq!(tree.nodes[1].name, "sub-cluster-apps");
    assert_eq!(tree.nodes[1].relationship, LineageRelation::Target);

    assert_eq!(tree.nodes[2].kind, "Deployment");
    assert_eq!(tree.nodes[2].name, "frontend-proxy");
    assert_eq!(tree.nodes[2].relationship, LineageRelation::Child);

    assert_eq!(tree.selected_node().unwrap().name, "sub-cluster-apps");
}

// ---------------------------------------------------------------------------
// D6: Feedback & Truthfulness (CRD Child Scan 403 Forbidden)
// Non-native child scan returning 403 Forbidden renders truthful [FORBIDDEN] badge,
// rather than being hidden or collapsed into an empty list.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d6_crd_child_scan_forbidden_renders_truthful_badge() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    let mut root_crd = LineageNode::new("Tenant", "tenant-finance", None, LineageRelation::Target)
        .with_api_version("multitenancy.example.com/v1")
        .with_uid("uid-tenant-1")
        .with_outcome(NodeOutcome::Healthy);

    let child_forbidden = LineageNode::new(
        "TenantWorkspace",
        "ws-finance-prod",
        Some("tenant-finance".into()),
        LineageRelation::Child,
    )
    .with_api_version("multitenancy.example.com/v1")
    .with_uid("uid-ws-1")
    .with_outcome(NodeOutcome::Forbidden);

    root_crd.children.push(child_forbidden);

    app.open_resource_tree(
        "Tenant".into(),
        "tenant-finance".into(),
        None,
        Some("multitenancy.example.com/v1".into()),
    );
    app.handle_lineage_result("Tenant", "tenant-finance", Ok(root_crd));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };

    assert_eq!(tree.nodes.len(), 2);
    assert_eq!(tree.nodes[1].fetch_outcome, Some(NodeOutcome::Forbidden));

    let text = tree.tree_as_text();
    assert!(
        text.contains("TenantWorkspace/ws-finance-prod [CHILD] [FORBIDDEN]"),
        "Forbidden child scan must render truthful [FORBIDDEN] outcome badge"
    );
}

// ---------------------------------------------------------------------------
// D5: State Transitions & Re-Entry (CRD Describe -> Esc -> Tree -> Esc -> Table)
// Opening Describe view on a CRD node and pressing Esc safely unwinds the nav stack
// back to the intact Tree view, and a second Esc returns to Table view.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn journey_d5_crd_describe_navigation_preserves_tree_nav_stack() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    app.open_resource_tree(
        "VirtualMachine".into(),
        "vm-workload".into(),
        Some("default".into()),
        Some("kubevirt.io/v1".into()),
    );
    app.handle_lineage_result("VirtualMachine", "vm-workload", Ok(mock_crd_tree()));

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected ActiveView::Tree");
    };
    assert_eq!(tree.root_kind, "VirtualMachine");

    // Press 'd' to open Describe view for selected CRD node
    press(&mut app, ch('d')).await;

    // View is now Describe
    let ActiveView::Describe(ref desc) = app.active_view else {
        panic!("expected ActiveView::Describe");
    };
    assert_eq!(desc.resource_kind, "VirtualMachineInstance");
    assert_eq!(desc.resource_name, "vm-workload");
    assert_eq!(desc.pinned_api_version.as_deref(), Some("kubevirt.io/v1"));

    // Press 'Esc' to pop back from Describe to Tree view
    press(&mut app, key(KeyCode::Esc)).await;

    let ActiveView::Tree(ref tree) = app.active_view else {
        panic!("expected restored ActiveView::Tree");
    };
    assert_eq!(tree.root_kind, "VirtualMachine");
    assert_eq!(tree.nodes.len(), 3);

    // Press 'Esc' again to pop back from Tree to Table view
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
}
