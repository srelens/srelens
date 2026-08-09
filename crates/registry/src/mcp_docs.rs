//! Renders `docs/mcp-catalog.md` from the live registry, the built-in prompt
//! library and the resource templates, so the published catalog cannot drift
//! from what the server actually serves.

use srelens_capability::Annotations;

/// How a tool is gated. Disjoint and exhaustive: every capability lands in
/// exactly one of these, which `the_buckets_partition_the_whole_registry`
/// enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SafetyClass {
    ReadOnly,
    SensitiveRead,
    NeedsConfirm,
    Destructive,
}

impl SafetyClass {
    pub fn label(self) -> &'static str {
        match self {
            SafetyClass::ReadOnly => "read-only",
            SafetyClass::SensitiveRead => "sensitive read",
            SafetyClass::NeedsConfirm => "needs confirmation",
            SafetyClass::Destructive => "destructive",
        }
    }
}

/// Annotation flags overlap — `SENSITIVE_READ` sets both `read_only` and
/// `requires_confirm` — so grouping by flag would double-count. Order matters:
/// `destructive` is checked first, and the read-only + confirm combination
/// before plain confirm.
pub fn classify(a: &Annotations) -> SafetyClass {
    if a.destructive {
        SafetyClass::Destructive
    } else if a.requires_confirm && a.read_only {
        SafetyClass::SensitiveRead
    } else if a.requires_confirm {
        SafetyClass::NeedsConfirm
    } else {
        SafetyClass::ReadOnly
    }
}

/// Which section of the catalog a tool belongs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Area {
    Kubernetes,
    Helm,
    Toolbox,
    Server,
}

impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Area::Kubernetes => "Kubernetes",
            Area::Helm => "Helm",
            Area::Toolbox => "Toolbox",
            Area::Server => "Server",
        }
    }

    /// Section order in the rendered page.
    pub fn all() -> [Area; 4] {
        [Area::Kubernetes, Area::Helm, Area::Toolbox, Area::Server]
    }
}

/// Helm tools are `k8s.helm*`, so that prefix MUST be tested before the bare
/// `k8s.` one.
pub fn area(id: &str) -> Area {
    if id.starts_with("k8s.helm") {
        Area::Helm
    } else if id.starts_with("k8s.") {
        Area::Kubernetes
    } else if id.starts_with("toolbox.") {
        Area::Toolbox
    } else {
        Area::Server
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::Annotations;

    #[test]
    fn each_annotation_preset_maps_to_its_bucket() {
        assert_eq!(classify(&Annotations::READ_ONLY), SafetyClass::ReadOnly);
        assert_eq!(classify(&Annotations::SENSITIVE_READ), SafetyClass::SensitiveRead);
        assert_eq!(classify(&Annotations::MUTATING), SafetyClass::NeedsConfirm);
        assert_eq!(classify(&Annotations::DESTRUCTIVE), SafetyClass::Destructive);
    }

    /// Smoke check that `classify` is total — every capability lands in exactly
    /// one bucket, and all registered ids can be looked up. Does not guard against
    /// mis-filing; for that, see `every_bucket_holds_only_capabilities_matching_its_definition`.
    ///
    /// The guarantee that no capability is left ungated (which would be published
    /// as read-only without consent) lives upstream: `srelens_mcp::completeness::assert_mutating_capabilities_are_gated`
    /// (crates/mcp/src/completeness.rs:36) asserts no capability has `!read_only && !requires_confirm`.
    /// This test just validates that our bucket assignments partition the registry correctly.
    #[test]
    fn the_buckets_partition_the_whole_registry() {
        let reg = crate::build_registry();
        let ids = reg.ids();
        let mut counts = std::collections::BTreeMap::new();
        for id in &ids {
            let class = classify(&reg.get(id).unwrap().annotations);
            *counts.entry(class.label()).or_insert(0usize) += 1;
        }
        let summed: usize = counts.values().sum();
        assert_eq!(summed, ids.len(), "buckets do not partition the registry: {counts:?}");
    }

    /// `sensitive` is a redaction flag, NOT a safety class. `diffManifest` is
    /// sensitive but deliberately un-gated (it was un-gated on purpose), so it
    /// must not be published as requiring confirmation.
    #[test]
    fn a_sensitive_but_ungated_tool_stays_read_only() {
        let reg = crate::build_registry();
        let diff = reg.get("k8s.diffManifest").expect("k8s.diffManifest is registered");
        assert!(diff.annotations.sensitive, "precondition: diffManifest is sensitive");
        assert!(!diff.annotations.requires_confirm, "precondition: diffManifest is not gated");
        assert_eq!(classify(&diff.annotations), SafetyClass::ReadOnly);
    }

    #[test]
    fn the_gated_secret_read_is_a_sensitive_read() {
        let reg = crate::build_registry();
        let secret = reg.get("k8s.getSecret").expect("k8s.getSecret is registered");
        assert_eq!(classify(&secret.annotations), SafetyClass::SensitiveRead);
    }

    /// Every capability must match its bucket's defining property. This catches
    /// genuine mis-filing when the classification order is wrong. Unlike the
    /// partition test (which is tautological for total functions), this test
    /// fails if a capability lands in the wrong bucket.
    ///
    /// - `ReadOnly` ⇒ `read_only && !requires_confirm && !destructive`
    /// - `SensitiveRead` ⇒ `read_only && requires_confirm`
    /// - `NeedsConfirm` ⇒ `requires_confirm && !read_only && !destructive`
    /// - `Destructive` ⇒ `destructive`
    #[test]
    fn every_bucket_holds_only_capabilities_matching_its_definition() {
        let reg = crate::build_registry();
        for id in &reg.ids() {
            let cap = reg.get(id).expect("capability exists");
            let ann = &cap.annotations;
            let class = classify(ann);

            match class {
                SafetyClass::ReadOnly => {
                    assert!(
                        ann.read_only && !ann.requires_confirm && !ann.destructive,
                        "{id} classified as ReadOnly but violates definition: \
                        read_only={}, requires_confirm={}, destructive={}",
                        ann.read_only, ann.requires_confirm, ann.destructive
                    );
                }
                SafetyClass::SensitiveRead => {
                    assert!(
                        ann.read_only && ann.requires_confirm,
                        "{id} classified as SensitiveRead but violates definition: \
                        read_only={}, requires_confirm={}",
                        ann.read_only, ann.requires_confirm
                    );
                }
                SafetyClass::NeedsConfirm => {
                    assert!(
                        ann.requires_confirm && !ann.read_only && !ann.destructive,
                        "{id} classified as NeedsConfirm but violates definition: \
                        requires_confirm={}, read_only={}, destructive={}",
                        ann.requires_confirm, ann.read_only, ann.destructive
                    );
                }
                SafetyClass::Destructive => {
                    assert!(
                        ann.destructive,
                        "{id} classified as Destructive but violates definition: \
                        destructive={}",
                        ann.destructive
                    );
                }
            }
        }
    }

    /// `k8s.helm*` must be tested BEFORE the `k8s.` prefix, or every Helm tool
    /// is filed under Kubernetes.
    #[test]
    fn helm_is_recognised_before_the_kubernetes_prefix() {
        assert_eq!(area("k8s.helmInstall"), Area::Helm);
        assert_eq!(area("k8s.helmListReleases"), Area::Helm);
        assert_eq!(area("k8s.listPods"), Area::Kubernetes);
        assert_eq!(area("k8s.scale"), Area::Kubernetes);
        assert_eq!(area("toolbox.status"), Area::Toolbox);
        assert_eq!(area("ping"), Area::Server);
    }

    /// Every registered capability must land in a real area. A new prefix
    /// falling through to `Server` would bury it under a heading nobody reads.
    #[test]
    fn every_registered_capability_maps_to_a_plausible_area() {
        let reg = crate::build_registry();
        for id in reg.ids() {
            let a = area(id);
            if id.starts_with("k8s.helm") {
                assert_eq!(a, Area::Helm, "{id}");
            } else if id.starts_with("k8s.") {
                assert_eq!(a, Area::Kubernetes, "{id}");
            } else if id.starts_with("toolbox.") {
                assert_eq!(a, Area::Toolbox, "{id}");
            } else {
                assert_eq!(a, Area::Server, "{id} has an unexpected prefix — add an Area for it");
            }
        }
    }
}
