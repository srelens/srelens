//! Prompt files: markdown with a YAML front-matter header. One format for the
//! built-ins embedded below and for user files under `<config>/mcp/prompts`.

use serde::Deserialize;

/// Which variant of a prompt this file is. A prompt is identified by
/// (name, mode), so one name can have both a targeted and a discover file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The caller named the object to triage.
    #[default]
    Targeted,
    /// No object given: the prompt opens by finding candidates.
    Discover,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Targeted => "targeted",
            Mode::Discover => "discover",
        }
    }
}

/// One declared argument. `target: true` marks the argument whose presence
/// selects the targeted mode — the target is `pod` for one flow and `node` or
/// `service` for another, so it has to be declared rather than inferred.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArgSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub target: bool,
    /// Substituted when the caller omits an optional argument, so no `{{token}}`
    /// can survive rendering into the agent's instructions.
    #[serde(default)]
    pub default: Option<String>,
}

/// The YAML header. Unknown fields are ignored (serde's default), which is what
/// makes a file written for a newer srelens still load on an older one.
#[derive(Debug, Deserialize)]
struct FrontMatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    priority: i64,
    #[serde(default)]
    arguments: Vec<ArgSpec>,
}

/// A parsed prompt file. `source` is the filename, used for tie-breaks and for
/// telling the user which file was rejected.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptFile {
    pub name: String,
    pub description: String,
    pub mode: Mode,
    pub priority: i64,
    pub arguments: Vec<ArgSpec>,
    pub body: String,
    pub source: String,
}

/// Split `---\n<yaml>\n---\n<body>` and parse the header.
pub fn parse_prompt_file(source: &str, text: &str) -> Result<PromptFile, String> {
    // Normalize line endings: convert CRLF to LF so the parser works on
    // Windows-authored files without misleading error messages.
    let text = text.replace("\r\n", "\n");

    let rest = text
        .strip_prefix("---\n")
        .ok_or("missing front matter: the file must start with `---`")?;
    let (yaml, body) = rest
        .split_once("\n---\n")
        .ok_or("unterminated front matter: no closing `---` line")?;
    let fm: FrontMatter =
        serde_yaml::from_str(yaml).map_err(|e| format!("invalid front matter: {e}"))?;
    if fm.name.trim().is_empty() {
        return Err("front matter `name` must not be empty".to_string());
    }
    Ok(PromptFile {
        name: fm.name.trim().to_string(),
        description: fm.description.unwrap_or_default(),
        mode: fm.mode,
        priority: fm.priority,
        arguments: fm.arguments,
        body: body.to_string(),
        source: source.to_string(),
    })
}

/// Every distinct `{{token}}` in `body`, in order of first appearance.
pub fn placeholders(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let token = after[..end].trim().to_string();
        if !token.is_empty() && !out.contains(&token) {
            out.push(token);
        }
        rest = &after[end + 2..];
    }
    out
}

/// Reject a file that could not render safely.
pub fn validate(file: &PromptFile) -> Result<(), String> {
    if file.body.trim().is_empty() {
        return Err("prompt body is empty".to_string());
    }
    // Check for empty placeholders that would leak into the agent instructions
    let mut rest = file.body.as_str();
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let token = after[..end].trim();
        if token.is_empty() {
            return Err("body contains empty placeholder (e.g. {{}} or {{ }}), which would leak literally into the agent instructions".to_string());
        }
        rest = &after[end + 2..];
    }
    let declared: Vec<&str> = file.arguments.iter().map(|a| a.name.as_str()).collect();
    let undeclared: Vec<String> = placeholders(&file.body)
        .into_iter()
        .filter(|t| !declared.contains(&t.as_str()))
        .collect();
    if !undeclared.is_empty() {
        return Err(format!(
            "body uses undeclared argument(s): {}",
            undeclared.join(", ")
        ));
    }
    Ok(())
}

/// Substitute `{{name}}` for every declared argument. Uses a single forward pass
/// to avoid both padded-placeholder misses ({{ pod }} not matching {{pod}}) and
/// chained re-substitution (supplied values containing other placeholders).
///
/// For each `{{...}}` span in the body:
/// - TRIM the token between the braces
/// - If a declared argument matches: append its resolved value (supplied → default → "")
///   and continue scanning AFTER the `}}`
/// - If no match or empty token: append the original span verbatim (braces included)
/// - If no closing `}}`: append the remainder verbatim and stop
///
/// This guarantees supplied values are treated as opaque literals and the output
/// is not affected by argument declaration order.
pub fn render(
    file: &PromptFile,
    supplied: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::new();
    let mut body = file.body.as_str();

    while let Some(start) = body.find("{{") {
        // Copy everything before the placeholder verbatim
        out.push_str(&body[..start]);

        let after_braces = &body[start + 2..];
        if let Some(end) = after_braces.find("}}") {
            let token = after_braces[..end].trim();

            // Look up the token in declared arguments
            let found = file.arguments.iter().find(|spec| spec.name == token);

            if let Some(spec) = found {
                // Append the resolved value: supplied → default → ""
                let value = supplied
                    .get(&spec.name)
                    .cloned()
                    .or_else(|| spec.default.clone())
                    .unwrap_or_default();
                out.push_str(&value);
            } else {
                // Token is undeclared or empty: append the original span verbatim
                out.push_str("{{");
                out.push_str(&after_braces[..end]);
                out.push_str("}}");
            }

            // Continue scanning after the closing braces
            body = &after_braces[end + 2..];
        } else {
            // No closing `}}`: append `{{` and continue from after it
            out.push_str("{{");
            body = after_braces;
        }
    }

    // Append any remaining text
    out.push_str(body);
    out
}

/// A file that could not be loaded, or was loaded with a caveat. Surfaced in
/// Settings → MCP: a silently-skipped file is a miserable authoring experience.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LoadIssue {
    /// Filename, not a full path — the path is the user's own config dir.
    pub file: String,
    pub problem: String,
}

/// Pick one winner per (name, mode): highest priority, then filename order.
/// Ties are reported because a silent choice between two files is a trap.
pub fn resolve(mut candidates: Vec<PromptFile>) -> (Vec<PromptFile>, Vec<LoadIssue>) {
    let mut issues = Vec::new();
    candidates.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.mode.cmp(&b.mode))
            .then(b.priority.cmp(&a.priority)) // higher priority first
            .then(a.source.cmp(&b.source))
    });
    let mut kept: Vec<PromptFile> = Vec::new();
    for candidate in candidates {
        match kept.last() {
            Some(winner) if winner.name == candidate.name && winner.mode == candidate.mode => {
                if winner.priority == candidate.priority {
                    issues.push(LoadIssue {
                        file: candidate.source.clone(),
                        problem: format!(
                            "`{}` ({}) is defined by both `{}` and `{}`; \
                             they have the same priority {}; `{}` wins by filename order",
                            candidate.name,
                            candidate.mode.as_str(),
                            winner.source,
                            candidate.source,
                            candidate.priority,
                            winner.source
                        ),
                    });
                }
                // Lower priority, or the loser of a tie: discarded.
            }
            _ => kept.push(candidate),
        }
    }
    (kept, issues)
}

/// The prompts srelens ships. Embedded, so they need no filesystem at runtime.
const BUILTIN_FILES: &[(&str, &str)] = &[
    (
        "pod-crashloop.targeted.md",
        include_str!("prompts/pod-crashloop.targeted.md"),
    ),
    (
        "pod-crashloop.discover.md",
        include_str!("prompts/pod-crashloop.discover.md"),
    ),
    (
        "pod-pending.targeted.md",
        include_str!("prompts/pod-pending.targeted.md"),
    ),
    (
        "pod-pending.discover.md",
        include_str!("prompts/pod-pending.discover.md"),
    ),
    (
        "node-pressure.targeted.md",
        include_str!("prompts/node-pressure.targeted.md"),
    ),
    (
        "node-pressure.discover.md",
        include_str!("prompts/node-pressure.discover.md"),
    ),
    (
        "service-no-endpoints.targeted.md",
        include_str!("prompts/service-no-endpoints.targeted.md"),
    ),
    (
        "service-no-endpoints.discover.md",
        include_str!("prompts/service-no-endpoints.discover.md"),
    ),
];

/// Parse and validate the embedded built-ins. Returns issues rather than
/// panicking so a bad build degrades to "that prompt is missing" instead of
/// taking the process down — `every_builtin_parses_and_validates` is what keeps
/// the issue list empty in practice.
pub fn builtins() -> (Vec<PromptFile>, Vec<LoadIssue>) {
    let mut files = Vec::new();
    let mut issues = Vec::new();
    for (source, text) in BUILTIN_FILES {
        match parse_prompt_file(source, text).and_then(|f| validate(&f).map(|()| f)) {
            Ok(f) => files.push(f),
            Err(problem) => issues.push(LoadIssue {
                file: (*source).to_string(),
                problem,
            }),
        }
    }
    (files, issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: pod-crashloop\ndescription: Triage a restarting pod\nmode: targeted\npriority: 0\narguments:\n  - { name: context, required: true }\n  - { name: pod, target: true }\n---\nTriage `{{pod}}` on `{{context}}`.\n";


    #[test]
    fn parses_front_matter_and_body() {
        let f = parse_prompt_file("pod-crashloop.targeted.md", SAMPLE).unwrap();
        assert_eq!(f.name, "pod-crashloop");
        assert_eq!(f.description, "Triage a restarting pod");
        assert_eq!(f.mode, Mode::Targeted);
        assert_eq!(f.priority, 0);
        assert_eq!(f.source, "pod-crashloop.targeted.md");
        assert_eq!(f.arguments.len(), 2);
        assert!(f.arguments[0].required, "context must parse as required");
        assert!(f.arguments[1].target, "pod must parse as the target argument");
        assert!(!f.arguments[1].required, "target arguments are optional");
        assert_eq!(f.body.trim(), "Triage `{{pod}}` on `{{context}}`.");
    }

    #[test]
    fn mode_defaults_to_targeted_and_priority_to_zero() {
        let text = "---\nname: x\n---\nbody\n";
        let f = parse_prompt_file("x.md", text).unwrap();
        assert_eq!(f.mode, Mode::Targeted);
        assert_eq!(f.priority, 0);
        assert_eq!(f.description, "", "a missing description is empty, not an error");
        assert!(f.arguments.is_empty());
    }

    #[test]
    fn rejects_a_file_with_no_front_matter() {
        let e = parse_prompt_file("x.md", "just a body\n").unwrap_err();
        assert!(e.contains("front matter"), "got: {e}");
    }

    #[test]
    fn rejects_unterminated_front_matter() {
        let e = parse_prompt_file("x.md", "---\nname: x\n").unwrap_err();
        assert!(e.contains("unterminated"), "got: {e}");
    }

    #[test]
    fn rejects_invalid_yaml() {
        let e = parse_prompt_file("x.md", "---\nname: [unclosed\n---\nbody\n").unwrap_err();
        assert!(e.contains("invalid front matter"), "got: {e}");
    }

    #[test]
    fn rejects_an_empty_name() {
        let e = parse_prompt_file("x.md", "---\nname: \"  \"\n---\nbody\n").unwrap_err();
        assert!(e.contains("name"), "got: {e}");
    }

    #[test]
    fn unknown_front_matter_fields_are_ignored() {
        // Forward compatibility: a file written for a newer srelens must still
        // load on an older one rather than disappearing.
        let text = "---\nname: x\nfutureField: 3\n---\nbody\n";
        let f = parse_prompt_file("x.md", text).unwrap();
        assert_eq!(f.name, "x");
    }

    #[test]
    fn parses_crlf_line_endings() {
        // Windows files with CRLF line endings should parse successfully.
        let text = "---\r\nname: x\r\n---\r\nbody\r\n";
        let f = parse_prompt_file("x.md", text).unwrap();
        assert_eq!(f.name, "x");
        assert_eq!(f.body, "body\n", "body should not retain stray \\r");
    }

    #[test]
    fn parses_mixed_line_endings() {
        // Some editors mix LF and CRLF; should still parse.
        let text = "---\r\nname: y\nmode: discover\r\n---\nbody line\r\n";
        let f = parse_prompt_file("y.md", text).unwrap();
        assert_eq!(f.name, "y");
        assert_eq!(f.mode, Mode::Discover);
        assert_eq!(f.body, "body line\n", "body should be normalized");
    }

    fn file_with(body: &str, args: &[&str]) -> PromptFile {
        PromptFile {
            name: "t".into(),
            description: String::new(),
            mode: Mode::Targeted,
            priority: 0,
            arguments: args
                .iter()
                .map(|n| ArgSpec {
                    name: (*n).into(),
                    description: None,
                    required: false,
                    target: false,
                    default: None,
                })
                .collect(),
            body: body.into(),
            source: "t.md".into(),
        }
    }

    fn args(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn named(name: &str, mode: Mode, priority: i64, source: &str) -> PromptFile {
        PromptFile {
            name: name.into(),
            description: String::new(),
            mode,
            priority,
            arguments: Vec::new(),
            body: "body".into(),
            source: source.into(),
        }
    }

    #[test]
    fn resolve_keeps_the_highest_priority_for_a_name_and_mode() {
        let (kept, issues) = resolve(vec![
            named("pod-crashloop", Mode::Targeted, 0, "builtin.md"),
            named("pod-crashloop", Mode::Targeted, 10, "mine.md"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "mine.md", "a higher priority must win");
        assert!(issues.is_empty(), "a clean override is not an issue");
    }

    #[test]
    fn resolve_keeps_both_modes_of_one_name() {
        let (kept, _) = resolve(vec![
            named("pod-crashloop", Mode::Targeted, 0, "a.md"),
            named("pod-crashloop", Mode::Discover, 0, "b.md"),
        ]);
        assert_eq!(kept.len(), 2, "(name, mode) is the identity, not name alone");
    }

    #[test]
    fn resolve_overrides_only_the_mode_it_collides_with() {
        let (kept, _) = resolve(vec![
            named("pod-crashloop", Mode::Targeted, 0, "builtin-t.md"),
            named("pod-crashloop", Mode::Discover, 0, "builtin-d.md"),
            named("pod-crashloop", Mode::Discover, 5, "mine-d.md"),
        ]);
        assert_eq!(kept.len(), 2);
        let targeted = kept.iter().find(|f| f.mode == Mode::Targeted).unwrap();
        let discover = kept.iter().find(|f| f.mode == Mode::Discover).unwrap();
        assert_eq!(targeted.source, "builtin-t.md", "the untouched mode is inherited");
        assert_eq!(discover.source, "mine-d.md");
    }

    #[test]
    fn resolve_breaks_a_priority_tie_by_filename_and_reports_it() {
        let (kept, issues) = resolve(vec![
            named("x", Mode::Targeted, 3, "b.md"),
            named("x", Mode::Targeted, 3, "a.md"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "a.md", "filename order breaks the tie");
        assert_eq!(issues.len(), 1, "a silent tie-break would be a trap");
        assert!(issues[0].problem.contains("a.md"), "got: {}", issues[0].problem);
        assert!(issues[0].problem.contains("b.md"), "got: {}", issues[0].problem);
    }

    #[test]
    fn render_substitutes_supplied_values() {
        let f = file_with("pod {{pod}} in {{namespace}}", &["pod", "namespace"]);
        let out = render(&f, &args(&[("pod", "web-0"), ("namespace", "prod")]));
        assert_eq!(out, "pod web-0 in prod");
    }

    /// No `{{token}}` may survive: an omitted optional argument falls back to
    /// its declared default, and to an empty string if it has none.
    #[test]
    fn render_falls_back_to_the_declared_default() {
        let mut f = file_with("ns {{namespace}} pod {{pod}}", &["namespace", "pod"]);
        f.arguments[0].default = Some("default".into());
        let out = render(&f, &args(&[("pod", "web-0")]));
        assert_eq!(out, "ns default pod web-0");
        assert!(!out.contains("{{"), "no placeholder may survive rendering");
    }

    #[test]
    fn render_leaves_no_placeholder_when_an_argument_has_no_default() {
        let f = file_with("ns [{{namespace}}]", &["namespace"]);
        let out = render(&f, &args(&[]));
        assert_eq!(out, "ns []");
        assert!(!out.contains("{{"));
    }

    #[test]
    fn render_replaces_every_occurrence() {
        let f = file_with("{{pod}} and {{pod}}", &["pod"]);
        assert_eq!(render(&f, &args(&[("pod", "web-0")])), "web-0 and web-0");
    }

    #[test]
    fn render_handles_padded_placeholders() {
        let f = file_with("ns [{{ pod }}]", &["pod"]);
        assert!(validate(&f).is_ok());
        let out = render(&f, &args(&[("pod", "web-0")]));
        assert_eq!(out, "ns [web-0]");
    }

    #[test]
    fn render_preserves_supplied_values_as_opaque_literals() {
        let f = file_with("{{pod}} {{namespace}}", &["pod", "namespace"]);
        let out = render(&f, &args(&[("pod", "{{namespace}}"), ("namespace", "prod")]));
        assert_eq!(out, "{{namespace}} prod", "pod's value is opaque text, not re-substituted");
    }

    #[test]
    fn render_is_order_independent() {
        let f1 = file_with("{{pod}} {{namespace}}", &["pod", "namespace"]);
        let out1 = render(&f1, &args(&[("pod", "{{namespace}}"), ("namespace", "prod")]));

        let f2 = file_with("{{pod}} {{namespace}}", &["namespace", "pod"]);
        let out2 = render(&f2, &args(&[("pod", "{{namespace}}"), ("namespace", "prod")]));

        assert_eq!(out1, out2, "output should not depend on argument declaration order");
    }

    #[test]
    fn render_handles_prefix_collision_correctly() {
        let f = file_with("{{pod}} {{pod_name}}", &["pod", "pod_name"]);
        let out = render(&f, &args(&[("pod", "web-0"), ("pod_name", "web")]));
        assert_eq!(out, "web-0 web");
    }

    #[test]
    fn render_preserves_unclosed_placeholder_verbatim() {
        let f = file_with("a {{unclosed", &[]);
        let out = render(&f, &args(&[]));
        assert_eq!(out, "a {{unclosed");
    }

    #[test]
    fn render_preserves_undeclared_placeholder_verbatim() {
        let f = file_with("{{pod}} {{undeclared}}", &["pod"]);
        let out = render(&f, &args(&[("pod", "web-0")]));
        assert_eq!(out, "web-0 {{undeclared}}", "undeclared placeholders remain as-is");
    }

    #[test]
    fn placeholders_are_found_in_order_without_duplicates() {
        let found = placeholders("a {{one}} b {{two}} c {{one}}");
        assert_eq!(found, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn placeholders_tolerate_whitespace_and_ignore_unclosed() {
        assert_eq!(placeholders("{{ spaced }}"), vec!["spaced".to_string()]);
        assert_eq!(placeholders("{{unclosed"), Vec::<String>::new());
        assert_eq!(placeholders("no tokens"), Vec::<String>::new());
    }

    #[test]
    fn validate_accepts_a_body_using_only_declared_arguments() {
        assert!(validate(&file_with("hi {{context}}", &["context"])).is_ok());
    }

    /// The important one: rendering an undeclared token would ship literal
    /// `{{foo}}` to the agent AS INSTRUCTIONS.
    #[test]
    fn validate_rejects_an_undeclared_placeholder() {
        let e = validate(&file_with("hi {{typo}}", &["context"])).unwrap_err();
        assert!(e.contains("typo"), "the message must name the offender, got: {e}");
    }

    #[test]
    fn validate_rejects_an_empty_body() {
        let e = validate(&file_with("   \n", &["context"])).unwrap_err();
        assert!(e.contains("empty"), "got: {e}");
    }

    #[test]
    fn validate_rejects_an_empty_placeholder() {
        let e = validate(&file_with("please check {{}}", &["context"])).unwrap_err();
        assert!(e.contains("empty placeholder"), "the message must indicate an empty placeholder, got: {e}");
    }

    #[test]
    fn validate_rejects_whitespace_only_placeholders() {
        let e = validate(&file_with("check {{ }}", &["context"])).unwrap_err();
        assert!(e.contains("empty placeholder"), "the message must indicate an empty placeholder, got: {e}");
    }

    /// Every embedded built-in must parse and validate. This is the build-time
    /// guard standing in for the compile-time checks we lost by moving argument
    /// specs into markdown — a malformed built-in fails the build rather than
    /// shipping and being skipped at runtime.
    #[test]
    fn every_builtin_parses_and_validates() {
        let (files, issues) = builtins();
        assert!(issues.is_empty(), "built-ins must be clean, got: {issues:?}");
        assert_eq!(files.len(), 8, "four flows x targeted/discover");
    }

    #[test]
    fn builtins_cover_the_four_documented_flows() {
        let (files, _) = builtins();
        for name in ["pod-crashloop", "pod-pending", "node-pressure", "service-no-endpoints"] {
            let modes: Vec<Mode> = files.iter().filter(|f| f.name == name).map(|f| f.mode).collect();
            assert!(modes.contains(&Mode::Targeted), "{name} needs a targeted variant");
            assert!(modes.contains(&Mode::Discover), "{name} needs a discover variant");
        }
    }

    #[test]
    fn every_builtin_requires_context_and_ships_at_priority_zero() {
        let (files, _) = builtins();
        for f in &files {
            assert_eq!(f.priority, 0, "{} must ship at priority 0", f.source);
            let ctx = f.arguments.iter().find(|a| a.name == "context");
            let ctx = ctx.unwrap_or_else(|| panic!("{} must declare context", f.source));
            assert!(ctx.required, "{}: context must be required", f.source);
        }
    }

    /// Built-ins diagnose and recommend; they never drive a mutation. Guarding
    /// by name means adding a fifth flow that says "call k8s.deletePod" fails
    /// the build instead of quietly shipping.
    #[test]
    fn no_builtin_instructs_a_mutating_tool_call() {
        let (files, _) = builtins();
        let mutating = [
            "k8s.deletePod", "k8s.deleteResource", "k8s.evictPod", "k8s.drainNode",
            "k8s.cordonNode", "k8s.scale", "k8s.rolloutRestart", "k8s.applyManifest",
            "k8s.updateConfigData", "k8s.helmInstall", "k8s.helmUpgrade",
            "k8s.helmUninstall", "k8s.helmRollback", "k8s.getSecret",
        ];
        for f in &files {
            for tool in mutating {
                assert!(
                    !f.body.contains(tool),
                    "{} names the mutating tool {tool}; built-ins must recommend a \
                     kubectl command instead",
                    f.source
                );
            }
        }
    }

    #[test]
    fn every_declared_argument_is_used_by_some_mode() {
        // The other direction of the drift guard: `validate` catches a body
        // using an undeclared argument; this catches a declared argument that no
        // body of that prompt uses, which is dead metadata in the client's form.
        let (files, _) = builtins();
        for name in ["pod-crashloop", "pod-pending", "node-pressure", "service-no-endpoints"] {
            let group: Vec<&PromptFile> = files.iter().filter(|f| f.name == name).collect();
            let used: Vec<String> =
                group.iter().flat_map(|f| placeholders(&f.body)).collect();
            for f in &group {
                for spec in &f.arguments {
                    assert!(
                        used.contains(&spec.name),
                        "{} declares `{}` but no {name} body uses it",
                        f.source,
                        spec.name
                    );
                }
            }
        }
    }
}
