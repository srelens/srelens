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

/// Which catalog section a capability id belongs under.
///
/// The Helm test runs first and is deliberately two-pronged, because the
/// registry names Helm capabilities two ways: helm as the verb prefix
/// (`k8s.helmInstall`) and helm as a camelCase word inside the name
/// (`k8s.getHelmRelease`, `k8s.listHelmReleases`). Matching only the prefix
/// published those two release readers under Kubernetes.
///
/// Two exclusions keep the broader match honest:
/// - Both prongs require an uppercase letter after `helm`, so a hypothetical
///   `k8s.helmet` stays under Kubernetes.
/// - Both are scoped to `k8s.` ids, so `toolbox.installHelm` stays under
///   Toolbox — it installs the helm *binary*, which is Toolbox work, not a
///   Helm release operation.
pub fn area(id: &str) -> Area {
    let helm_prefixed =
        id.strip_prefix("k8s.helm").is_some_and(|rest| rest.starts_with(char::is_uppercase));
    let helm_word = id.starts_with("k8s.") && id.contains("Helm");
    if helm_prefixed || helm_word {
        Area::Helm
    } else if id.starts_with("k8s.") {
        Area::Kubernetes
    } else if id.starts_with("toolbox.") {
        Area::Toolbox
    } else {
        Area::Server
    }
}

/// Safety-class order within an area: safest first, so a reader scanning a
/// section meets the harmless tools before the dangerous ones.
const SAFETY_ORDER: [SafetyClass; 4] = [
    SafetyClass::ReadOnly,
    SafetyClass::SensitiveRead,
    SafetyClass::NeedsConfirm,
    SafetyClass::Destructive,
];

/// Built-in prompts only. A user's own prompt directory is machine-local, so
/// including it would make the published catalog depend on who generated it.
pub fn render_prompts() -> String {
    let lib = srelens_mcp::prompts::PromptLibrary::new(None);
    let specs = lib.list();
    let mut out = format!("## Prompts ({})\n\n", specs.len());
    out.push_str("| Prompt | Description | Arguments |\n| --- | --- | --- |\n");
    for spec in specs {
        let args: Vec<String> = spec
            .arguments
            .iter()
            .map(|a| {
                if a.required {
                    format!("{} (required)", a.name)
                } else {
                    a.name.clone()
                }
            })
            .collect();
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            spec.name,
            spec.description,
            args.join(", ")
        ));
    }
    out.push('\n');
    out
}


/// The addressable resource surface: the fixed entries a client gets from
/// `resources/list`, then the URI templates from `resources/templates/list`.
pub fn render_resources() -> String {
    let fixed = srelens_mcp::resources::fixed_resources();
    let templates = srelens_mcp::resources::templates();
    let mut out = format!(
        "## Resources ({} fixed, {} templates)\n\n",
        fixed.len(),
        templates.len()
    );

    out.push_str("`resources/list` returns only these two:\n\n| URI | Description |\n| --- | --- |\n");
    for r in &fixed {
        // These are internally-produced values, so a missing key is a bug in this repo.
        let uri = r["uri"]
            .as_str()
            .expect("fixed resource missing 'uri' key");
        let description = r["description"]
            .as_str()
            .expect("fixed resource missing 'description' key");
        out.push_str(&format!("| `{}` | {} |\n", uri, description));
    }

    out.push_str(
        "\nObject addressing is discovered through `resources/templates/list`:\n\n\
         | URI template | Description |\n| --- | --- |\n",
    );
    for t in &templates {
        // These are internally-produced values, so a missing key is a bug in this repo.
        let uri_template = t["uriTemplate"]
            .as_str()
            .expect("template missing 'uriTemplate' key");
        let description = t["description"]
            .as_str()
            .expect("template missing 'description' key");
        out.push_str(&format!("| `{}` | {} |\n", uri_template, description));
    }
    out.push('\n');
    out
}

/// srelens's own half of every client config. Generated from these constants so
/// a renamed flag is caught by `every_flag_the_configs_name_exists_in_the_cli`
/// rather than silently breaking the documented setup path.
pub const FLAG_STDIO: &str = "--mcp-stdio";
pub const FLAG_HTTP: &str = "--mcp-http";
pub const FLAG_ALLOW_DESTRUCTIVE: &str = "--mcp-allow-destructive";
pub const FLAG_ALLOW_SENSITIVE_READS: &str = "--mcp-allow-sensitive-reads";
/// The token never goes in argv — that would leak it into `ps`.
pub const TOKEN_ENV: &str = "SRELENS_MCP_TOKEN";

/// Shared helpers the tests need. No `#[cfg(test)]` of its own: the whole
/// `mcp_docs` module is already gated on it in `lib.rs`, since `lib.rs`'s
/// doc-scan tests use `json_blocks` too and every caller of this module is a
/// test.
pub mod tests_support {
    /// Extract the body of every ```json fenced block.
    ///
    /// Panics if a ```json fence is opened but never closed, because an
    /// unterminated block in hand-written prose is an error that should be
    /// caught during testing, not silently skipped. This matters in
    /// particular for the doc-scan validation over `docs/MCP.md`.
    pub fn json_blocks(md: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut current: Option<String> = None;
        let mut fence_start_line = 0;
        for (line_num, line) in md.lines().enumerate() {
            let trimmed = line.trim_start();
            match (&mut current, trimmed) {
                (None, "```json") => {
                    current = Some(String::new());
                    fence_start_line = line_num;
                }
                (Some(buf), l) if l.starts_with("```") => {
                    out.push(std::mem::take(buf));
                    current = None;
                }
                (Some(buf), _) => {
                    buf.push_str(line);
                    buf.push('\n');
                }
                _ => {}
            }
        }
        if current.is_some() {
            let first_line = md.lines().nth(fence_start_line).unwrap_or("(unknown)");
            panic!(
                "json_blocks: ```json fence opened at line {} but never closed: {}",
                fence_start_line + 1,
                first_line
            );
        }
        out
    }
}

/// Paste-ready client configs.
///
/// srelens's side is generated from the constants above. The surrounding
/// envelope (`mcpServers`, and each client's key names) is a per-client
/// template, because that shape belongs to the client and changes outside this
/// repo — generating it cannot make it correct, only consistent.
pub fn render_client_configs() -> String {
    let mut out = String::from("## Client configuration\n\n");

    out.push_str(
        "### Claude Desktop\n\nAdd to `claude_desktop_config.json`:\n\n```json\n{\n  \
         \"mcpServers\": {\n    \"srelens\": {\n      \"command\": \"srelens\",\n      \
         \"args\": [\"",
    );
    out.push_str(FLAG_STDIO);
    out.push_str("\"]\n    }\n  }\n}\n```\n\n");

    out.push_str("### Claude Code\n\n```bash\nclaude mcp add srelens -- srelens ");
    out.push_str(FLAG_STDIO);
    out.push_str("\n```\n\n");

    out.push_str(
        "### Generic stdio client\n\nSpawn the binary and speak newline-delimited \
         JSON-RPC on stdin/stdout:\n\n```json\n{\n  \"command\": \"srelens\",\n  \
         \"args\": [\"",
    );
    out.push_str(FLAG_STDIO);
    out.push_str("\"]\n}\n```\n\n");

    out.push_str(&format!(
        "### Headless consent\n\nBoth transports refuse gated tools by default. To \
         pre-authorise them for an unattended session, add `{FLAG_ALLOW_DESTRUCTIVE}` \
         (mutations) or `{FLAG_ALLOW_SENSITIVE_READS}` (secret reads).\n\n"
    ));

    out.push_str(&format!(
        "### HTTP transport\n\n`{FLAG_HTTP} <addr>` starts a **separate, headless** MCP \
         server process — it does not attach to an already-running GUI, and will fail to \
         bind if the GUI's own Settings → MCP toggle already holds the port. (To share the \
         GUI's process and its in-app confirm dialog instead, use Settings → MCP → Run the \
         MCP server in the running desktop app.) Either way, point an HTTP-capable client \
         at the address with the bearer token as `Authorization: Bearer <token>` — read \
         from `{TOKEN_ENV}` for the headless process, or from Settings → MCP for the \
         in-app one; never from argv:\n\n"
    ));
    out.push_str(
        "```json\n{\n  \"mcpServers\": {\n    \"srelens\": {\n      \
         \"url\": \"http://127.0.0.1:8765/mcp\",\n      \
         \"headers\": { \"Authorization\": \"Bearer <token>\" }\n    }\n  }\n}\n```\n\n",
    );

    out
}

/// Render every tool as `### <Area> — <safety> (<count>)` sections, each a
/// table sorted by id. Empty combinations are skipped rather than rendered as
/// an empty table.
pub fn render_tools(reg: &srelens_capability::Registry) -> String {
    let mut out = String::new();
    out.push_str(&format!("## Tools ({})\n\n", reg.ids().len()));
    out.push_str(
        "Argument schemas are not reproduced here — call `tools/list` for those, \
         which cannot go stale.\n\n",
    );

    for a in Area::all() {
        for safety in SAFETY_ORDER {
            let mut rows: Vec<(&str, &str)> = reg
                .ids()
                .into_iter()
                .filter(|id| area(id) == a)
                .filter_map(|id| reg.get(id).map(|cap| (id, cap)))
                .filter(|(_, cap)| classify(&cap.annotations) == safety)
                .map(|(id, cap)| (id, cap.summary.as_str()))
                .collect();
            if rows.is_empty() {
                continue;
            }
            // `Registry::ids()` yields sorted order today (BTreeMap), but this sort is
            // defensive: if the type ever changes to HashMap or if filtering scrambles order,
            // the sort becomes load-bearing. Without it, the published catalog would be
            // silently out of order.
            rows.sort_unstable_by_key(|(id, _)| *id);

            out.push_str(&format!(
                "### {} — {} ({})\n\n| Tool | Summary |\n| --- | --- |\n",
                a.label(),
                safety.label(),
                rows.len()
            ));
            for (id, summary) in rows {
                out.push_str(&format!("| `{id}` | {summary} |\n"));
            }
            out.push('\n');
        }
    }
    out
}

/// The whole generated page.
pub fn render_catalog() -> String {
    let mut out = String::from(
        "<!-- GENERATED FILE — do not edit by hand.\n     \
         Regenerate with: UPDATE_CATALOG=1 cargo test -p srelens-registry -->\n\n\
         # srelens MCP catalog\n\n\
         Everything this server exposes over MCP, generated from the live \
         registry so it cannot drift. Written for someone wiring an agent to \
         srelens; the narrative reference is [MCP.md](MCP.md).\n\n",
    );
    out.push_str(&render_tools(&crate::build_registry()));
    out.push_str(&render_prompts());
    out.push_str(&render_resources());
    out.push_str(&render_client_configs());
    out
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
            // Toolbox is checked first: `toolbox.installHelm` installs the helm
            // binary, which is Toolbox work, not a Helm release operation.
            //
            // The helm arm is deliberately case-insensitive rather than
            // mirroring `area`'s own prefix logic. An earlier version asserted
            // that anything `k8s.`-prefixed but not `k8s.helm`-prefixed was
            // Kubernetes — which ratified the bug that filed
            // `k8s.getHelmRelease` and `k8s.listHelmReleases` under Kubernetes
            // instead of catching it. Expectations here come from what the name
            // means, not from how `area` happens to parse it.
            if id.starts_with("toolbox.") {
                assert_eq!(a, Area::Toolbox, "{id}");
            } else if id.to_ascii_lowercase().contains("helm") {
                assert_eq!(a, Area::Helm, "{id} names helm, so it belongs under Helm");
            } else if id.starts_with("k8s.") {
                assert_eq!(a, Area::Kubernetes, "{id}");
            } else if id.starts_with("toolbox.") {
                assert_eq!(a, Area::Toolbox, "{id}");
            } else {
                assert_eq!(a, Area::Server, "{id} has an unexpected prefix — add an Area for it");
            }
        }
    }

    /// Helm tools follow a strict naming convention: `k8s.helm` followed by an
    /// uppercase verb like `Install`, `Upgrade`, `Rollback`. This test uses
    /// synthetic ids to ensure boundary checking prevents false positives like
    /// a hypothetical `k8s.helmet` from being misfiled.
    #[test]
    fn helm_boundary_prevents_false_matches() {
        // Synthetic edge cases that must NOT match as Helm
        assert_eq!(area("k8s.helmet"), Area::Kubernetes, "lowercase after k8s.helm");
        assert_eq!(area("k8s.helm"), Area::Kubernetes, "bare k8s.helm with no verb");

        // Real Helm tools that MUST match, in both naming styles the registry
        // actually uses: helm as the verb prefix, and helm as a camelCase word.
        assert_eq!(area("k8s.helmInstall"), Area::Helm);
        assert_eq!(area("k8s.getHelmRelease"), Area::Helm, "helm as a camelCase word");
        assert_eq!(area("k8s.listHelmReleases"), Area::Helm, "helm as a camelCase word");

        // And the camelCase match must not reach outside `k8s.`: installing the
        // helm binary is Toolbox work.
        assert_eq!(area("toolbox.installHelm"), Area::Toolbox, "toolbox owns binary installs");
    }

    /// Adding a new `Area` variant should be caught by the exhaustive match here,
    /// forcing acknowledgment of the new area. However, the `all()` array is
    /// hand-maintained, so a variant added to both the match and the seed but
    /// forgotten in the array would still slip through. This test catches the
    /// first part of that problem (compiler forces acknowledgment), but not the
    /// second (hand-maintenance of `all()`).
    #[test]
    fn all_area_variants_appear_in_the_canonical_order() {
        for probe in [Area::Kubernetes, Area::Helm, Area::Toolbox, Area::Server] {
            match probe {
                Area::Kubernetes | Area::Helm | Area::Toolbox | Area::Server => {}
            }
            assert!(
                Area::all().contains(&probe),
                "{probe:?} is missing from Area::all()"
            );
        }
    }

    #[test]
    fn tool_tables_are_grouped_by_area_and_safety_with_counts() {
        let reg = crate::build_registry();
        let md = render_tools(&reg);
        assert!(md.contains("### Kubernetes — read-only ("), "got:\n{md}");
        assert!(md.contains("### Kubernetes — destructive ("), "got:\n{md}");
        assert!(md.contains("### Helm — "), "got:\n{md}");
        assert!(md.contains("### Toolbox — "), "got:\n{md}");
        // A known row, with its summary, proving the table carries real data.
        assert!(md.contains("| `k8s.listPods` |"), "got:\n{md}");
    }

    /// Every tool appears exactly once across all sections. Renders the id in
    /// backticks inside a table cell, so counting that exact pattern counts rows.
    #[test]
    fn every_tool_appears_exactly_once() {
        let reg = crate::build_registry();
        let md = render_tools(&reg);
        for id in reg.ids() {
            let cell = format!("| `{id}` |");
            assert_eq!(md.matches(&cell).count(), 1, "{id} should appear exactly once");
        }
    }

    /// `diffManifest` is sensitive but un-gated, so it must render under
    /// read-only — never under a confirmation heading.
    #[test]
    fn the_sensitive_but_ungated_tool_renders_under_read_only() {
        let reg = crate::build_registry();
        let md = render_tools(&reg);
        let ro = md
            .split("### ")
            .find(|s| s.starts_with("Kubernetes — read-only"))
            .expect("a Kubernetes read-only section exists");
        assert!(ro.contains("| `k8s.diffManifest` |"), "diffManifest belongs in read-only");
    }

    /// Proves the renderer **preserves** the input's sorted order across filtering and
    /// grouping — a future change that scrambles rows (HashMap intermediate, broken
    /// sort, reordering filter) would fail this. Does not prove the sort call itself
    /// is reachable: input arrives pre-sorted from `Registry::ids()` (BTreeMap), so
    /// the sort is defensive rather than essential today.
    #[test]
    fn rows_within_a_section_are_sorted_by_id() {
        let reg = crate::build_registry();
        let md = render_tools(&reg);
        for section_text in md.split("### ").skip(1) {
            // Skip the header line; remaining lines are either table separator or rows.
            let ids: Vec<&str> = section_text
                .lines()
                .skip(2) // Skip heading and separator
                .filter_map(|l| l.strip_prefix("| `"))
                .filter_map(|l| l.split('`').next())
                .collect();
            if ids.is_empty() {
                continue;
            }
            let mut sorted = ids.clone();
            sorted.sort_unstable();
            let section_name = section_text.lines().next().unwrap_or("(unknown)");
            assert_eq!(
                ids, sorted,
                "Section '### {}' rows must be sorted by id, got: {:?}",
                section_name, ids
            );
        }
    }

    /// Backticks are allowed if balanced (even count); a single unmatched backtick
    /// opens a code span that never closes and corrupts rendering. Pipes and newlines
    /// break the table row structure itself.
    fn has_unmatched_backticks(s: &str) -> bool {
        !s.matches('`').count().is_multiple_of(2)
    }

    /// Tool summaries are interpolated into Markdown table cells. A summary
    /// containing `|`, newline, or unmatched backticks would corrupt the row or break
    /// the pattern that `every_tool_appears_exactly_once` counts on. This test fails
    /// at the source (in the registry) rather than producing silently broken markdown.
    #[test]
    fn no_summary_contains_table_or_markdown_delimiters() {
        let reg = crate::build_registry();
        for id in reg.ids() {
            let cap = reg.get(id).expect("capability exists");
            let summary = &cap.summary;
            assert!(
                !summary.contains('|'),
                "{id}: summary contains pipe: {summary:?}"
            );
            assert!(
                !has_unmatched_backticks(summary),
                "{id}: summary contains unmatched backtick: {summary:?}"
            );
            assert!(
                !summary.contains('\n'),
                "{id}: summary contains newline: {summary:?}"
            );
        }
    }

    #[test]
    fn prompts_render_with_required_arguments_marked() {
        let lib = srelens_mcp::prompts::PromptLibrary::new(None);
        let specs = lib.list();
        let md = render_prompts();
        // Every built-in prompt must appear with its full row format.
        for spec in &specs {
            let args_str: Vec<String> = spec
                .arguments
                .iter()
                .map(|a| {
                    if a.required {
                        format!("{} (required)", a.name)
                    } else {
                        a.name.clone()
                    }
                })
                .collect();
            let expected_row = format!(
                "| `{}` | {} | {} |",
                spec.name,
                spec.description,
                args_str.join(", ")
            );
            assert!(
                md.contains(&expected_row),
                "prompt {} missing expected row format from:\n{md}",
                spec.name
            );
        }
        // Specifically verify context is marked as required on all prompts.
        assert!(md.contains("context (required)"), "got:\n{md}");
    }

    #[test]
    fn resources_render_both_fixed_entries_and_every_template() {
        let fixed = srelens_mcp::resources::fixed_resources();
        let templates = srelens_mcp::resources::templates();
        let md = render_resources();

        // Every fixed resource must appear with its full row format.
        for r in &fixed {
            let uri = r["uri"].as_str().expect("fixed resource has 'uri' key");
            let description = r["description"]
                .as_str()
                .expect("fixed resource has 'description' key");
            let expected_row = format!("| `{}` | {} |", uri, description);
            assert!(
                md.contains(&expected_row),
                "fixed resource {} missing expected row format from:\n{md}",
                uri
            );
        }

        // Every template must appear with its full row format.
        for t in &templates {
            let uri_template = t["uriTemplate"]
                .as_str()
                .expect("template has 'uriTemplate' key");
            let description = t["description"]
                .as_str()
                .expect("template has 'description' key");
            let expected_row = format!("| `{}` | {} |", uri_template, description);
            assert!(
                md.contains(&expected_row),
                "template {} missing expected row format from:\n{md}",
                uri_template
            );
        }
    }

    /// Prompt descriptions and argument names are interpolated into Markdown
    /// table cells. A description containing `|`, unmatched backticks, or newline
    /// would corrupt the row or break the rendering. Resource names and descriptions
    /// face the same risk. Balanced backticks (`` `-` ``) are allowed; a single
    /// unmatched backtick opens a code span that never closes. This test fails at
    /// the source (in the MCP definitions) rather than producing silently broken markdown.
    #[test]
    fn no_prompt_or_resource_text_contains_table_or_markdown_delimiters() {
        // Check prompt descriptions and argument names.
        let lib = srelens_mcp::prompts::PromptLibrary::new(None);
        for spec in lib.list() {
            assert!(
                !spec.description.contains('|'),
                "prompt {}: description contains pipe: {:?}",
                spec.name,
                spec.description
            );
            assert!(
                !has_unmatched_backticks(&spec.description),
                "prompt {}: description contains unmatched backtick: {:?}",
                spec.name,
                spec.description
            );
            assert!(
                !spec.description.contains('\n'),
                "prompt {}: description contains newline: {:?}",
                spec.name,
                spec.description
            );
            for arg in &spec.arguments {
                assert!(
                    !arg.name.contains('|'),
                    "prompt {} argument {}: name contains pipe: {:?}",
                    spec.name,
                    arg.name,
                    arg.name
                );
                assert!(
                    !has_unmatched_backticks(&arg.name),
                    "prompt {} argument {}: name contains unmatched backtick: {:?}",
                    spec.name,
                    arg.name,
                    arg.name
                );
                assert!(
                    !arg.name.contains('\n'),
                    "prompt {} argument {}: name contains newline: {:?}",
                    spec.name,
                    arg.name,
                    arg.name
                );
            }
        }

        // Check fixed resource names and descriptions.
        for r in srelens_mcp::resources::fixed_resources() {
            let uri = r["uri"].as_str().expect("has 'uri' key");
            let name = r["name"].as_str().expect("has 'name' key");
            let description = r["description"].as_str().expect("has 'description' key");
            assert!(
                !name.contains('|'),
                "fixed resource {}: name contains pipe: {:?}",
                uri,
                name
            );
            assert!(
                !has_unmatched_backticks(name),
                "fixed resource {}: name contains unmatched backtick: {:?}",
                uri,
                name
            );
            assert!(
                !name.contains('\n'),
                "fixed resource {}: name contains newline: {:?}",
                uri,
                name
            );
            assert!(
                !description.contains('|'),
                "fixed resource {}: description contains pipe: {:?}",
                uri,
                description
            );
            assert!(
                !has_unmatched_backticks(description),
                "fixed resource {}: description contains unmatched backtick: {:?}",
                uri,
                description
            );
            assert!(
                !description.contains('\n'),
                "fixed resource {}: description contains newline: {:?}",
                uri,
                description
            );
        }

        // Check template names and descriptions.
        for t in srelens_mcp::resources::templates() {
            let uri_template = t["uriTemplate"].as_str().expect("has 'uriTemplate' key");
            let name = t["name"].as_str().expect("has 'name' key");
            let description = t["description"].as_str().expect("has 'description' key");
            assert!(
                !name.contains('|'),
                "template {}: name contains pipe: {:?}",
                uri_template,
                name
            );
            assert!(
                !has_unmatched_backticks(name),
                "template {}: name contains unmatched backtick: {:?}",
                uri_template,
                name
            );
            assert!(
                !name.contains('\n'),
                "template {}: name contains newline: {:?}",
                uri_template,
                name
            );
            assert!(
                !description.contains('|'),
                "template {}: description contains pipe: {:?}",
                uri_template,
                description
            );
            assert!(
                !has_unmatched_backticks(description),
                "template {}: description contains unmatched backtick: {:?}",
                uri_template,
                description
            );
            assert!(
                !description.contains('\n'),
                "template {}: description contains newline: {:?}",
                uri_template,
                description
            );
        }
    }

    #[test]
    fn client_configs_name_the_real_flags_and_token_env_var() {
        let md = render_client_configs();
        // Check that all five constants appear. These assertions guard section
        // presence (e.g., the entire "Headless consent" section could vanish
        // without triggering failure elsewhere), not flag correctness — the
        // `main.rs` cross-check below is what guards that flags are correct.
        assert!(md.contains(FLAG_STDIO), "FLAG_STDIO missing:\n{md}");
        assert!(md.contains(FLAG_HTTP), "FLAG_HTTP missing:\n{md}");
        assert!(md.contains(FLAG_ALLOW_DESTRUCTIVE), "FLAG_ALLOW_DESTRUCTIVE missing:\n{md}");
        assert!(md.contains(FLAG_ALLOW_SENSITIVE_READS), "FLAG_ALLOW_SENSITIVE_READS missing:\n{md}");
        assert!(md.contains(TOKEN_ENV), "TOKEN_ENV missing:\n{md}");
        // Also verify the sections that carry these constants exist.
        assert!(md.contains("### Headless consent"), "Headless consent section missing:\n{md}");
        assert!(md.contains("### HTTP transport"), "HTTP transport section missing:\n{md}");
        // Verify client names are present.
        assert!(md.contains("Claude Desktop"), "Claude Desktop name missing:\n{md}");
        assert!(md.contains("Claude Code"), "Claude Code name missing:\n{md}");
    }

    /// Every fenced json block in the generated configs must parse — a
    /// paste-ready config that is not valid JSON is worse than none.
    #[test]
    fn every_generated_json_block_parses() {
        let md = render_client_configs();
        let blocks = crate::mcp_docs::tests_support::json_blocks(&md);
        assert!(!blocks.is_empty(), "expected at least one json block:\n{md}");
        for b in blocks {
            serde_json::from_str::<serde_json::Value>(&b)
                .unwrap_or_else(|e| panic!("generated config is not valid JSON: {e}\n{b}"));
        }
    }

    /// srelens's own half of every config must be flags the binary actually
    /// accepts. `main.rs` matches flags as string literals, so the literal
    /// appearing there is the check available without invoking the binary.
    #[test]
    fn every_flag_the_configs_name_exists_in_the_cli() {
        let main_rs = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/desktop/src-tauri/src/main.rs"
        ))
        .expect("main.rs is readable");
        for flag in [FLAG_STDIO, FLAG_HTTP, FLAG_ALLOW_DESTRUCTIVE, FLAG_ALLOW_SENSITIVE_READS] {
            assert!(
                main_rs.contains(&format!("\"{flag}\"")),
                "{flag} is documented but the CLI does not accept it"
            );
        }
        assert!(
            main_rs.contains(&format!("\"{TOKEN_ENV}\"")),
            "{TOKEN_ENV} is documented but main.rs does not read it"
        );
    }

    #[test]
    #[should_panic(expected = "json_blocks: ```json fence opened at line")]
    fn json_blocks_panics_on_unterminated_fence() {
        crate::mcp_docs::tests_support::json_blocks("```json\n{\"a\": 1}\n");
    }

    #[test]
    fn json_blocks_only_matches_exactly_json_not_json5_or_jsonc() {
        let md = "```json\n{\"a\": 1}\n```\n```json5\n{a: 1}\n```\n```jsonc\n{\"a\": 1}\n```";
        let blocks = crate::mcp_docs::tests_support::json_blocks(md);
        assert_eq!(blocks.len(), 1, "should only match ```json, not json5 or jsonc");
        assert!(blocks[0].contains("\"a\""), "should contain the json block content");
    }

    /// `classify` and `McpServer::consent_kind` (`crates/mcp/src/lib.rs`) both
    /// decide safety from the same `Annotations`, but check the flags in
    /// different orders: `classify` tests `destructive` first, `consent_kind`
    /// tests `read_only` first. If a capability were ever annotated with both
    /// `read_only` and `destructive` set, the two would disagree — the
    /// published catalog would tell an agent to pass `--mcp-allow-destructive`
    /// while the runtime gate actually demanded `--mcp-allow-sensitive-reads`
    /// (or vice versa). No such capability exists today, but nothing else
    /// guards against one being added — `every_bucket_holds_only_capabilities_
    /// matching_its_definition` only asserts `ann.destructive` for the
    /// `Destructive` arm, so it would not catch this. This test makes the
    /// safety table in MCP.md mechanically true rather than merely true today.
    #[test]
    fn classify_and_consent_kind_agree_on_the_flag_a_capability_needs() {
        let reg = crate::build_registry();
        let server = srelens_mcp::McpServer::new(std::sync::Arc::new(crate::build_registry()));
        for id in reg.ids() {
            let cap = reg.get(id).expect("capability exists");
            let published_flag = match classify(&cap.annotations) {
                SafetyClass::ReadOnly => None,
                SafetyClass::SensitiveRead => {
                    Some(srelens_mcp::policy::ConsentKind::SensitiveRead.flag())
                }
                SafetyClass::NeedsConfirm | SafetyClass::Destructive => {
                    Some(srelens_mcp::policy::ConsentKind::Destructive.flag())
                }
            };
            let runtime_flag = server.consent_kind(id).map(|k| k.flag());
            assert_eq!(
                published_flag, runtime_flag,
                "{id}: classify() implies flag {published_flag:?} but \
                 McpServer::consent_kind implies {runtime_flag:?} — the published \
                 safety class and the runtime gate disagree about which flag \
                 this capability needs"
            );
        }
    }

    #[test]
    fn the_page_opens_with_a_do_not_edit_header_and_carries_every_section() {
        let md = render_catalog();
        assert!(md.starts_with("<!-- GENERATED FILE"), "got:\n{}", &md[..200.min(md.len())]);
        assert!(md.contains("UPDATE_CATALOG=1"), "the header must name the fix command");
        assert!(md.contains("## Tools ("));
        assert!(md.contains("## Prompts ("));
        assert!(md.contains("## Resources ("));
        assert!(md.contains("## Client configuration"));
        assert!(md.ends_with('\n'), "must end with a newline");
    }
}
