//! The operator example apps (#1099): each manifest under `examples/extensions/` that a
//! row names is held to that row, and to the rules the desktop applies at install.
//! The directory scans elsewhere prove a manifest parses; this proves it is the app it
//! says it is, so a kind or an action cannot quietly drop out of one.
use super::tests::fake_core;
use super::*;
use std::collections::BTreeSet;

struct App {
    file: &'static str,
    id: &'static str,
    /// The upstream tag whose CRDs the manifest was written against.
    release: &'static str,
    /// Every `group/Kind` the app reads.
    kinds: &'static [&'static str],
    actions: &'static [Action],
}

struct Action {
    name: &'static str,
    target: &'static str,
    /// The annotation key or `/spec` pointer the action writes.
    writes: &'static str,
    /// Whether it declares both `availableWhen` and `preconditions`.
    gated: bool,
}

const fn action(
    name: &'static str,
    target: &'static str,
    writes: &'static str,
    gated: bool,
) -> Action {
    Action { name, target, writes, gated }
}

const PAUSED: &str = "autoscaling.keda.sh/paused";
const PAUSED_REPLICAS: &str = "autoscaling.keda.sh/paused-replicas";

const APPS: &[App] = &[
    App {
        file: "keda.json",
        id: "org.srelens.keda",
        release: "v2.21.1",
        kinds: &[
            "keda.sh/ClusterTriggerAuthentication",
            "keda.sh/ScaledJob",
            "keda.sh/ScaledObject",
            "keda.sh/TriggerAuthentication",
        ],
        actions: &[
            action("pause", "k8s.annotate", PAUSED, true),
            action("scale-to-zero", "k8s.annotate", PAUSED_REPLICAS, true),
            action("resume", "k8s.mergePatch", PAUSED, false),
            action("pause-job", "k8s.annotate", PAUSED, true),
            action("resume-job", "k8s.mergePatch", PAUSED, false),
        ],
    },
    App {
        file: "external-secrets.json",
        id: "org.srelens.external-secrets",
        release: "helm-chart-2.15.0",
        kinds: &[
            "external-secrets.io/ClusterExternalSecret",
            "external-secrets.io/ClusterSecretStore",
            "external-secrets.io/ExternalSecret",
            "external-secrets.io/PushSecret",
            "external-secrets.io/SecretStore",
        ],
        actions: &[action("force-sync", "k8s.annotate", "external-secrets.io/force-sync", false)],
    },
    // Karpenter v1.15.1; the AWS and Azure node classes from their providers' v1.15.2,
    // and the GCE one from karpenter-provider-gcp v0.8.2.
    App {
        file: "karpenter.json",
        id: "org.srelens.karpenter",
        release: "v1.15.1",
        kinds: &[
            "karpenter.azure.com/AKSNodeClass",
            "karpenter.k8s.aws/EC2NodeClass",
            "karpenter.k8s.gcp/GCENodeClass",
            "karpenter.sh/NodeClaim",
            "karpenter.sh/NodeOverlay",
            "karpenter.sh/NodePool",
        ],
        actions: &[],
    },
    App {
        file: "kamaji.json",
        id: "org.srelens.kamaji",
        release: "26.11.4-edge",
        kinds: &[
            "kamaji.clastix.io/DataStore",
            "kamaji.clastix.io/KubeconfigGenerator",
            "kamaji.clastix.io/TenantControlPlane",
        ],
        actions: &[],
    },
    App {
        file: "velero.json",
        id: "org.srelens.velero",
        release: "v1.18.4",
        kinds: &[
            "velero.io/Backup",
            "velero.io/BackupRepository",
            "velero.io/BackupStorageLocation",
            "velero.io/DataDownload",
            "velero.io/DataUpload",
            "velero.io/PodVolumeBackup",
            "velero.io/PodVolumeRestore",
            "velero.io/Restore",
            "velero.io/Schedule",
            "velero.io/VolumeSnapshotLocation",
        ],
        actions: &[
            action("pause-schedule", "k8s.setFields", "/spec/paused", true),
            action("resume-schedule", "k8s.setFields", "/spec/paused", true),
        ],
    },
    // CloudNativePG v1.30.1; ObjectStore from the Barman Cloud plugin's v0.16.0.
    App {
        file: "cloudnative-pg.json",
        id: "org.srelens.cloudnative-pg",
        release: "v1.30.1",
        kinds: &[
            "barmancloud.cnpg.io/ObjectStore",
            "postgresql.cnpg.io/Backup",
            "postgresql.cnpg.io/Cluster",
            "postgresql.cnpg.io/ClusterImageCatalog",
            "postgresql.cnpg.io/Database",
            "postgresql.cnpg.io/DatabaseRole",
            "postgresql.cnpg.io/FailoverQuorum",
            "postgresql.cnpg.io/ImageCatalog",
            "postgresql.cnpg.io/Pooler",
            "postgresql.cnpg.io/Publication",
            "postgresql.cnpg.io/ScheduledBackup",
            "postgresql.cnpg.io/Subscription",
        ],
        actions: &[
            action("restart", "k8s.annotate", "kubectl.kubernetes.io/restartedAt", false),
            action("reload", "k8s.annotate", "cnpg.io/reloadedAt", false),
            // The two that stop something are merge patches, for the high-impact warning.
            action("hibernate", "k8s.mergePatch", "cnpg.io/hibernation", true),
            action("wake", "k8s.mergePatch", "cnpg.io/hibernation", true),
            action("pause-pooler", "k8s.mergePatch", "/spec/pgbouncer/paused", true),
            action("resume-pooler", "k8s.mergePatch", "/spec/pgbouncer/paused", true),
            action("start-maintenance", "k8s.setFields", "/spec/nodeMaintenanceWindow/inProgress", true),
            action("end-maintenance", "k8s.setFields", "/spec/nodeMaintenanceWindow/inProgress", true),
            action("suspend-backups", "k8s.setFields", "/spec/suspend", true),
            action("resume-backups", "k8s.setFields", "/spec/suspend", true),
        ],
    },
    App {
        file: "sveltos.json",
        id: "org.srelens.sveltos",
        release: "v1.17.0",
        kinds: &[
            "config.projectsveltos.io/ClusterConfiguration",
            "config.projectsveltos.io/ClusterProfile",
            "config.projectsveltos.io/ClusterReport",
            "config.projectsveltos.io/ClusterSummary",
            "config.projectsveltos.io/Profile",
            "lib.projectsveltos.io/Classifier",
            "lib.projectsveltos.io/EventSource",
            "lib.projectsveltos.io/EventTrigger",
            "lib.projectsveltos.io/SveltosCluster",
        ],
        actions: &[
            action("pause-cluster", "k8s.setFields", "/spec/paused", true),
            action("resume-cluster", "k8s.setFields", "/spec/paused", true),
        ],
    },
    App {
        file: "kubeswift.json",
        id: "org.srelens.kubeswift",
        release: "v0.25.1",
        kinds: &[
            "fleet.kubeswift.io/Cluster",
            "gpu.kubeswift.io/SwiftGPUNode",
            "gpu.kubeswift.io/SwiftGPUProfile",
            "image.kubeswift.io/SwiftImage",
            "kernel.kubeswift.io/SwiftKernel",
            "migration.kubeswift.io/SwiftMigration",
            "sandbox.kubeswift.io/SwiftArtifactExportPolicy",
            "sandbox.kubeswift.io/SwiftArtifactPlacement",
            "sandbox.kubeswift.io/SwiftArtifactVerificationPolicy",
            "sandbox.kubeswift.io/SwiftSandbox",
            "sandbox.kubeswift.io/SwiftSandboxPool",
            "seed.kubeswift.io/SwiftSeedProfile",
            "snapshot.kubeswift.io/SwiftRestore",
            "snapshot.kubeswift.io/SwiftSnapshot",
            "snapshot.kubeswift.io/SwiftSnapshotSchedule",
            "storage.kubeswift.io/SwiftClusterStorageLocation",
            "storage.kubeswift.io/SwiftStorageLocation",
            "swift.kubeswift.io/SwiftGuest",
            "swift.kubeswift.io/SwiftGuestClass",
            "swift.kubeswift.io/SwiftGuestPool",
            "swift.kubeswift.io/SwiftVolumeAttachment",
        ],
        // Stop is left out: upstream also deletes the launcher pod, which an app cannot.
        actions: &[action("start-guest", "k8s.setFields", "/spec/runPolicy", true)],
    },
];

fn source(file: &str) -> String {
    let path = format!("{}/../../examples/extensions/{file}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Whether an action's arguments write `target`: an annotation key, a `setFields`
/// pointer, or a path a merge patch reaches.
fn writes(arguments: &Value, target: &str) -> bool {
    if arguments["key"] == target || arguments["fields"].get(target).is_some() {
        return true;
    }
    let patch = &arguments["patch"];
    match target.strip_prefix('/') {
        Some(pointer) => patch.pointer(&format!("/{pointer}")).is_some(),
        None => patch["metadata"]["annotations"].get(target).is_some(),
    }
}

#[test]
fn every_example_app_installs_and_declares_what_its_row_says() {
    for app in APPS {
        let file = app.file;
        let raw: Value = serde_json::from_str(&source(file)).unwrap();
        assert_eq!(raw["id"], app.id, "{file}");
        assert_eq!(raw["srelensApiVersion"], "^0.5", "{file}");
        assert_eq!(raw["version"], "0.1.0", "{file}");
        assert!(!app.release.is_empty(), "{file}: no upstream release recorded");

        let kinds: BTreeSet<String> = raw["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|binding| binding["target"] == "k8s.listCustomResource")
            .map(|binding| {
                let arguments = &binding["arguments"];
                format!(
                    "{}/{}",
                    arguments["group"].as_str().unwrap(),
                    arguments["kind"].as_str().unwrap()
                )
            })
            .collect();
        let expected: BTreeSet<String> = app.kinds.iter().map(|kind| kind.to_string()).collect();
        assert_eq!(kinds, expected, "{file}: kinds");

        let declared = raw["actions"].as_array().cloned().unwrap_or_default();
        let names: BTreeSet<&str> = declared
            .iter()
            .map(|action| action["name"].as_str().unwrap())
            .collect();
        let rows: BTreeSet<&str> = app.actions.iter().map(|action| action.name).collect();
        assert_eq!(names, rows, "{file}: actions");
        for row in app.actions {
            let name = row.name;
            let action = declared.iter().find(|action| action["name"] == name).unwrap();
            assert_eq!(action["target"], row.target, "{file}: {name}");
            assert!(writes(&action["arguments"], row.writes), "{file}: {name} writes {}", row.writes);
            let gated = action.get("availableWhen").is_some() && action.get("preconditions").is_some();
            assert_eq!(gated, row.gated, "{file}: {name} gating");
        }

        // An overview keeps a live stream open per page it lists, and an app may hold
        // eight (srelens-streams). Eleven pages were a valid dashboard that stopped
        // updating as it opened; six were seen updating with one other view open.
        for page in raw["contributions"]["pages"].as_array().unwrap() {
            let listed = page["dashboard"]["pages"].as_array().map_or(0, Vec::len);
            assert!(listed <= 6, "{file}: the {} dashboard lists {listed} pages", page["id"]);
        }

        let manifest = Manifest::parse(&raw.to_string()).unwrap_or_else(|error| panic!("{file}: {error}"));
        if let Err(errors) = validate_app_for_tests(&manifest, fake_core()) {
            panic!("{file}: {errors:?}");
        }
    }
}
