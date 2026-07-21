#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub requires_confirm: bool,
    /// Reads or exposes sensitive material (e.g. Secret values). Lets an MCP
    /// consent layer gate these separately from ordinary reads.
    pub sensitive: bool,
}

impl Annotations {
    pub const READ_ONLY: Self =
        Self { read_only: true, destructive: false, requires_confirm: false, sensitive: false };
    pub const DESTRUCTIVE: Self =
        Self { read_only: false, destructive: true, requires_confirm: true, sensitive: false };
    /// A change that isn't destructive but still needs user consent (e.g.
    /// installing a tool): confirm-gated, not flagged destructive.
    pub const MUTATING: Self =
        Self { read_only: false, destructive: false, requires_confirm: true, sensitive: false };
    /// A read that returns sensitive material — gateable by consent policy.
    pub const SENSITIVE_READ: Self =
        Self { read_only: true, destructive: false, requires_confirm: false, sensitive: true };
}
