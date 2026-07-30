//! The capability catalog: a stable, serializable projection of the registry
//! (id + the annotation flags the frontend palette audit needs). Emitted to a
//! committed JSON so a Vitest test can cross-check the palette without linking
//! Rust. Kept in sync by a test in `lib.rs`.
use serde::Serialize;
use srelens_capability::Registry;

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: String,
    pub read_only: bool,
    pub destructive: bool,
}

/// Project a registry into a sorted catalog for the frontend bridge.
pub fn catalog_of(reg: &Registry) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = reg
        .entries()
        .map(|c| CatalogEntry {
            id: c.id.clone(),
            read_only: c.annotations.read_only,
            destructive: c.annotations.destructive,
        })
        .collect();
    out.sort();
    out
}
