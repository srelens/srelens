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
    /// Set only by `builtins()`, never by parsing front matter — `FrontMatter`
    /// has no `builtin` field, so a user file cannot claim this status for
    /// itself by writing `builtin: true` in its own header. This is what lets
    /// `resolve` give built-ins the win on an equal-priority tie without a
    /// user being able to spoof that outcome.
    pub builtin: bool,
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
        // Always false here: `FrontMatter` (the struct serde deserializes
        // into) has no `builtin` field at all, so there is no YAML key a
        // user file could set to become true. Only `builtins()` below ever
        // flips this.
        builtin: false,
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
    // Check for empty or unterminated placeholders that would leak into the
    // agent instructions. An unclosed `{{` must be rejected here, not merely
    // tolerated: `placeholders` stops scanning at it (so it never gets
    // declared/undeclared-checked below) while `render` copies everything
    // from that point on verbatim, so a body like `Check {{context` with no
    // closing `}}` would otherwise validate cleanly and still ship malformed
    // template text straight into the agent's instructions.
    let mut rest = file.body.as_str();
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(
                "body contains an unterminated `{{` with no matching `}}`, which would \
                 leak literally into the agent instructions"
                    .to_string(),
            );
        };
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

/// Pick one winner per (name, mode): highest priority, then built-in status,
/// then filename order. Ties are reported because a silent choice between two
/// files is a trap.
///
/// Overriding a shipped prompt must be an EXPLICIT, declared act — that is the
/// whole point of the numeric `priority` field. So on an equal-priority tie a
/// built-in beats a user file regardless of filename: a user file that
/// declares nothing must never silently replace a built-in triage flow whose
/// body an AI agent is about to follow as instructions. A user-vs-user tie
/// still breaks by filename order, unchanged.
pub fn resolve(mut candidates: Vec<PromptFile>) -> (Vec<PromptFile>, Vec<LoadIssue>) {
    let mut issues = Vec::new();
    candidates.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.mode.cmp(&b.mode))
            .then(b.priority.cmp(&a.priority)) // higher priority first
            .then(b.builtin.cmp(&a.builtin)) // built-in first on a tie
            .then(a.source.cmp(&b.source))
    });
    let mut kept: Vec<PromptFile> = Vec::new();
    for candidate in candidates {
        match kept.last() {
            Some(winner) if winner.name == candidate.name && winner.mode == candidate.mode => {
                if winner.priority == candidate.priority {
                    let problem = if winner.builtin && !candidate.builtin {
                        format!(
                            "`{}` ({}) in `{}` did not take effect: it ties the built-in \
                             prompt's priority ({}), and a built-in wins that tie; declare \
                             a higher `priority` in `{}` to override it",
                            candidate.name,
                            candidate.mode.as_str(),
                            candidate.source,
                            candidate.priority,
                            candidate.source
                        )
                    } else {
                        format!(
                            "`{}` ({}) is defined by both `{}` and `{}`; \
                             they have the same priority {}; `{}` wins by filename order",
                            candidate.name,
                            candidate.mode.as_str(),
                            winner.source,
                            candidate.source,
                            candidate.priority,
                            winner.source
                        )
                    };
                    issues.push(LoadIssue {
                        file: candidate.source.clone(),
                        problem,
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
            Ok(mut f) => {
                f.builtin = true;
                files.push(f);
            }
            Err(problem) => issues.push(LoadIssue {
                file: (*source).to_string(),
                problem,
            }),
        }
    }
    (files, issues)
}

/// Prompt text becomes instructions in an agent's context, so both the number of
/// files and their size are bounded — unbounded content here is a footgun, not
/// a feature.
pub const MAX_USER_PROMPT_FILES: usize = 100;
pub const MAX_USER_PROMPT_BYTES: u64 = 64 * 1024;

/// Read `*.md` from `dir`. Never fails as a whole: each file's problem is
/// recorded and the remaining files still load. A missing directory is silent —
/// a user who never created one has nothing wrong with their setup. Any OTHER
/// `read_dir` error (permission denied, or `dir` existing as a regular file)
/// is a real I/O failure, not an absent directory, and must not be
/// indistinguishable from "the user has no prompts" — it is recorded as a
/// directory-level issue instead.
pub fn load_dir(dir: &std::path::Path) -> (Vec<PromptFile>, Vec<LoadIssue>) {
    let mut files = Vec::new();
    let mut issues = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (files, issues),
        Err(e) => {
            issues.push(LoadIssue {
                // Not a filename: same sentinel used for the file-count cap
                // below, so the user's local path never leaks into a field
                // the Settings UI renders as a filename.
                file: "<prompts directory>".to_string(),
                problem: format!("could not read prompts directory: {e}"),
            });
            return (files, issues);
        }
    };

    // Sorted so the priority tie-break in `resolve` is deterministic and so the
    // file-count cap keeps a stable subset rather than whatever the OS listed first.
    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .collect();
    paths.sort();

    if paths.len() > MAX_USER_PROMPT_FILES {
        issues.push(LoadIssue {
            // Not a filename: the cap is a property of the directory, not of
            // any one file. Using the real path would leak the user's local
            // filesystem layout into a field the Settings UI renders as a
            // filename (see `LoadIssue::file`'s doc comment).
            file: "<prompts directory>".to_string(),
            problem: format!(
                "{} prompt files found; only the first {MAX_USER_PROMPT_FILES} \
                 (by filename) were loaded",
                paths.len()
            ),
        });
        paths.truncate(MAX_USER_PROMPT_FILES);
    }

    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut fail = |problem: String| {
            issues.push(LoadIssue { file: name.clone(), problem });
        };

        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > MAX_USER_PROMPT_BYTES => {
                fail(format!(
                    "file is {} bytes; the limit is {} KB",
                    meta.len(),
                    MAX_USER_PROMPT_BYTES / 1024
                ));
                continue;
            }
            Err(e) => {
                fail(format!("could not read: {e}"));
                continue;
            }
            _ => {}
        }

        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                fail(format!("could not read: {e}"));
                continue;
            }
        };

        match parse_prompt_file(&name, &text).and_then(|f| validate(&f).map(|()| f)) {
            Ok(f) => files.push(f),
            Err(problem) => fail(problem),
        }
    }

    (files, issues)
}

/// One prompt as a client sees it: modes are an internal detail, so both
/// variants of a name collapse into a single entry whose arguments are the
/// union across modes.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptSpec {
    pub name: String,
    pub description: String,
    pub arguments: Vec<ArgSpec>,
}

/// A rendered prompt, ready to become an MCP `prompts/get` result.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub description: String,
    pub text: String,
}

/// The built-ins plus any user files, resolved by priority.
pub struct PromptLibrary {
    user_dir: Option<std::path::PathBuf>,
}

impl PromptLibrary {
    pub fn new(user_dir: Option<std::path::PathBuf>) -> Self {
        Self { user_dir }
    }

    /// Re-read on every call so editing a file takes effect without restarting
    /// srelens. A handful of small files, and callers are infrequent.
    fn all(&self) -> (Vec<PromptFile>, Vec<LoadIssue>) {
        let (mut files, mut issues) = builtins();
        if let Some(dir) = &self.user_dir {
            let (user_files, user_issues) = load_dir(dir);
            files.extend(user_files);
            issues.extend(user_issues);
        }
        let (kept, resolve_issues) = resolve(files);
        issues.extend(resolve_issues);
        (kept, issues)
    }

    pub fn issues(&self) -> Vec<LoadIssue> {
        self.all().1
    }

    pub fn list(&self) -> Vec<PromptSpec> {
        let (files, _) = self.all();
        let mut specs: Vec<PromptSpec> = Vec::new();
        // `files` is sorted by (name, mode) out of `resolve`, and Targeted sorts
        // before Discover, so the targeted variant is seen first and supplies the
        // description.
        for file in &files {
            if specs.iter().any(|s| s.name == file.name) {
                continue;
            }

            // Every resolved variant of this prompt name (targeted and/or
            // discover — there are at most these two, one per `Mode`).
            let variants: Vec<&PromptFile> = files.iter().filter(|f| f.name == file.name).collect();

            let description = variants
                .iter()
                .map(|v| v.description.clone())
                .find(|d| !d.is_empty())
                .unwrap_or_default();

            // `required` is serialized to the client, so its union must be safe:
            // an argument is required in the merged spec only if EVERY variant
            // of this prompt requires it. A variant that does not declare the
            // argument AT ALL does not require it — it simply has no opinion —
            // so a `target: true, required: true` argument declared only by the
            // targeted file must not be advertised as always required: that
            // would force every caller to always supply it, making discover
            // mode unreachable for this prompt. `default` is not serialized, so
            // taking the first-seen variant's value for it is unobservable.
            // `description` IS serialized, and first-seen non-empty (targeted,
            // since it sorts first) deliberately wins: a reasonable choice of
            // one merged form field's text.
            let mut arguments: Vec<ArgSpec> = Vec::new();
            for variant in &variants {
                for arg in &variant.arguments {
                    if arguments.iter().any(|a| a.name == arg.name) {
                        continue;
                    }
                    let required = variants.iter().all(|v| {
                        v.arguments
                            .iter()
                            .find(|a| a.name == arg.name)
                            .is_some_and(|a| a.required)
                    });
                    let mut merged = arg.clone();
                    merged.required = required;
                    arguments.push(merged);
                }
            }

            specs.push(PromptSpec {
                name: file.name.clone(),
                description,
                arguments,
            });
        }
        specs
    }

    /// Render `name` for `supplied`. The error string is the JSON-RPC -32602
    /// message, so it must be actionable on its own.
    pub fn get(
        &self,
        name: &str,
        supplied: &std::collections::BTreeMap<String, String>,
    ) -> Result<Rendered, String> {
        let (files, _) = self.all();
        let variants: Vec<&PromptFile> = files.iter().filter(|f| f.name == name).collect();
        if variants.is_empty() {
            return Err(format!("unknown prompt `{name}`"));
        }

        // Targeted iff every target-marked argument was supplied with a
        // non-empty value. The marker is what makes this unambiguous — the
        // target is `pod` for one flow and `node` or `service` for another.
        // A present-but-blank value (`""`, or whitespace-only from an unfilled
        // form field) is not a target: it must fall back to discover mode
        // rather than render instructions that name an empty resource.
        let targets: Vec<&str> = variants
            .iter()
            .flat_map(|f| f.arguments.iter())
            .filter(|a| a.target)
            .map(|a| a.name.as_str())
            .collect();

        // With no target-marked argument declared, there is no discover-vs-
        // targeted distinction a caller could even express — a prompt with a
        // single variant and no `target: true` argument (the simplest legal
        // prompt file) must render that one variant rather than being forced
        // into Discover and failing because no Discover file exists. Prefer
        // Targeted (matching `Mode`'s own default) when both happen to be
        // present with no target declared; otherwise render whichever one
        // variant the file actually ships.
        let wanted = if targets.is_empty() {
            variants
                .iter()
                .find(|f| f.mode == Mode::Targeted)
                .or_else(|| variants.iter().find(|f| f.mode == Mode::Discover))
                .map(|f| f.mode)
                .unwrap_or_default()
        } else {
            let all_targets_supplied = targets
                .iter()
                .all(|t| supplied.get(*t).is_some_and(|v| !v.trim().is_empty()));
            if all_targets_supplied {
                Mode::Targeted
            } else {
                Mode::Discover
            }
        };

        let chosen = variants.iter().find(|f| f.mode == wanted).ok_or_else(|| {
            if targets.is_empty() {
                format!("`{name}` has no {} variant", wanted.as_str())
            } else {
                format!(
                    "`{name}` has no {} variant; supply the `{}` argument",
                    wanted.as_str(),
                    targets.join(", ")
                )
            }
        })?;

        // Required arguments are checked only against the variant actually
        // being rendered: `list()` ANDs `required` across modes (an argument
        // required in one mode but optional in another is advertised as NOT
        // required), so `get()` must honor that by validating only `chosen`'s
        // own arguments — otherwise a bare call that `list()` says is legal
        // (omitting an argument that's required only in the targeted variant)
        // would still be rejected here, making discover mode unreachable. A
        // present-but-blank value (`""`, or whitespace-only) does not satisfy
        // a required argument either — same predicate as the target check
        // above, so e.g. a blank `context` can't sneak through and render
        // instructions with an empty, multi-context-hazard value.
        for spec in &chosen.arguments {
            if spec.required
                && !supplied.get(&spec.name).is_some_and(|v| !v.trim().is_empty())
            {
                return Err(format!("`{name}` requires the `{}` argument", spec.name));
            }
        }

        Ok(Rendered {
            description: chosen.description.clone(),
            text: render(chosen, supplied),
        })
    }
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
            builtin: false,
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
            builtin: false,
        }
    }

    /// Same as `named`, but marked as a built-in — for pinning `resolve`'s
    /// provenance-aware tie-break.
    fn named_builtin(name: &str, mode: Mode, priority: i64, source: &str) -> PromptFile {
        let mut f = named(name, mode, priority, source);
        f.builtin = true;
        f
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("srelens-prompts-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &std::path::Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    const GOOD: &str = "---\nname: mine\npriority: 5\narguments:\n  - { name: context, required: true }\n---\nCheck `{{context}}`.\n";

    #[test]
    fn load_dir_reads_markdown_files() {
        let dir = temp_dir("read");
        write(&dir, "mine.md", GOOD);
        let (files, issues) = load_dir(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "mine");
        assert_eq!(files[0].priority, 5);
        assert_eq!(files[0].source, "mine.md", "source is the filename");
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn load_dir_ignores_non_markdown_files() {
        let dir = temp_dir("ext");
        write(&dir, "mine.md", GOOD);
        write(&dir, "notes.txt", "not a prompt");
        write(&dir, "README", "not a prompt");
        let (files, issues) = load_dir(&dir);
        assert_eq!(files.len(), 1);
        assert!(issues.is_empty(), "a non-.md file is not an error, got: {issues:?}");
    }

    /// macOS and Windows filesystems treat `.md` and `.MD` as the same
    /// extension, so a case-sensitive filter would skip a same-file rename
    /// while presenting it as an unrelated non-prompt file.
    #[test]
    fn load_dir_matches_the_extension_case_insensitively() {
        let dir = temp_dir("case");
        write(&dir, "UPPER.MD", &GOOD.replace("name: mine", "name: upper"));
        write(&dir, "Mixed.Md", &GOOD.replace("name: mine", "name: mixed"));
        let (files, issues) = load_dir(&dir);
        assert_eq!(files.len(), 2, "both uppercase and mixed-case .md must load, got: {files:?}");
        assert!(files.iter().any(|f| f.name == "upper"));
        assert!(files.iter().any(|f| f.name == "mixed"));
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    /// A directory the user never created is not a problem to report.
    #[test]
    fn load_dir_of_a_missing_directory_is_silent() {
        let (files, issues) = load_dir(&std::env::temp_dir().join("srelens-no-such-dir-xyz"));
        assert!(files.is_empty());
        assert!(issues.is_empty());
    }

    /// A real I/O failure (here: `dir` is actually a regular file, so
    /// `read_dir` fails with `NotADirectory`/`ENOTDIR`, not `NotFound`) must
    /// not be silently indistinguishable from "the user has no prompts" —
    /// unlike a missing directory, this is a genuine problem to surface.
    #[test]
    fn load_dir_of_a_path_that_is_a_regular_file_reports_one_issue() {
        let parent = std::env::temp_dir()
            .join(format!("srelens-prompts-notadir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        let file_path = parent.join("this-is-a-file-not-a-dir");
        std::fs::write(&file_path, "not a directory").unwrap();

        let (files, issues) = load_dir(&file_path);
        assert!(files.is_empty(), "a regular file has no prompt files in it");
        assert_eq!(issues.len(), 1, "got: {issues:?}");
        assert_eq!(
            issues[0].file, "<prompts directory>",
            "must not leak the real path into the issue, got: {}",
            issues[0].file
        );
        assert!(
            !issues[0].problem.contains(file_path.to_string_lossy().as_ref()),
            "the local path must not leak into the issue text, got: {}",
            issues[0].problem
        );
    }

    /// The load-bearing behaviour: one bad file must not take the others down.
    #[test]
    fn load_dir_skips_a_bad_file_and_keeps_the_rest() {
        let dir = temp_dir("bad");
        write(&dir, "good.md", GOOD);
        write(&dir, "broken.md", "no front matter here\n");
        write(&dir, "typo.md", "---\nname: t\n---\nuses {{undeclared}}\n");
        let (files, issues) = load_dir(&dir);
        assert_eq!(files.len(), 1, "the good file must still load");
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|i| i.file == "broken.md"));
        assert!(issues.iter().any(|i| i.file == "typo.md" && i.problem.contains("undeclared")));
    }

    #[test]
    fn load_dir_skips_a_file_over_the_size_cap() {
        let dir = temp_dir("size");
        let padding = "x".repeat(MAX_USER_PROMPT_BYTES as usize + 1);
        write(&dir, "huge.md", &format!("---\nname: h\n---\n{padding}\n"));
        write(&dir, "good.md", GOOD);
        let (files, issues) = load_dir(&dir);
        assert_eq!(files.len(), 1, "only the small file loads");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, "huge.md");
        assert!(issues[0].problem.contains("64"), "the cap must be stated, got: {}", issues[0].problem);
    }

    #[test]
    fn load_dir_stops_at_the_file_count_cap() {
        let dir = temp_dir("count");
        for i in 0..(MAX_USER_PROMPT_FILES + 5) {
            write(&dir, &format!("p{i:04}.md"), &GOOD.replace("name: mine", &format!("name: p{i}")));
        }
        let (files, issues) = load_dir(&dir);
        assert_eq!(files.len(), MAX_USER_PROMPT_FILES);
        assert_eq!(
            files.iter().map(|f| f.source.as_str()).max(),
            Some("p0099.md"),
            "the kept files must be the first MAX_USER_PROMPT_FILES by filename"
        );
        assert_eq!(issues.len(), 1, "exactly one cap issue, got: {issues:?}");
        assert!(
            issues[0].problem.contains("100"),
            "the cap must be reported, got: {issues:?}"
        );
        assert_eq!(
            issues[0].file, "<prompts directory>",
            "LoadIssue.file must never be an absolute path; got: {}",
            issues[0].file
        );
        assert!(
            !issues[0].file.contains(dir.to_string_lossy().as_ref()),
            "the local temp dir path must not leak into the issue, got: {}",
            issues[0].file
        );
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

    /// Case 1 of the provenance tie-break: builtin@0 vs user@0 -> the builtin
    /// wins even though "mine.md" would have sorted first alphabetically, and
    /// the warning names the user's file and tells them what to do about it.
    #[test]
    fn resolve_prefers_a_builtin_over_a_user_file_on_an_equal_priority_tie() {
        let (kept, issues) = resolve(vec![
            named("pod-crashloop", Mode::Targeted, 0, "mine.md"),
            named_builtin("pod-crashloop", Mode::Targeted, 0, "pod-crashloop.targeted.md"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "pod-crashloop.targeted.md", "the built-in must win the tie");
        assert_eq!(issues.len(), 1, "the tie must be reported");
        assert_eq!(issues[0].file, "mine.md", "the issue must name the file that lost");
        assert!(
            issues[0].problem.contains("mine.md"),
            "got: {}",
            issues[0].problem
        );
        assert!(
            issues[0].problem.to_lowercase().contains("priority"),
            "the warning must say what to do about it (declare a higher priority), got: {}",
            issues[0].problem
        );
    }

    /// Case 2: builtin@0 vs user@10 -> a declared, higher priority is a clean
    /// override, with no warning at all.
    #[test]
    fn resolve_lets_a_higher_priority_user_file_cleanly_override_a_builtin() {
        let (kept, issues) = resolve(vec![
            named_builtin("pod-crashloop", Mode::Targeted, 0, "pod-crashloop.targeted.md"),
            named("pod-crashloop", Mode::Targeted, 10, "mine.md"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "mine.md", "the higher priority must win");
        assert!(issues.is_empty(), "a clean override must not warn");
    }

    /// Case 3: user@5 vs user@5 -> unchanged behaviour, filename order breaks
    /// the tie (this is `resolve_breaks_a_priority_tie_by_filename_and_reports_it`
    /// above in spirit; restated here to pin it as one of the four provenance
    /// cases explicitly).
    #[test]
    fn resolve_breaks_a_tie_between_two_user_files_by_filename() {
        let (kept, issues) = resolve(vec![
            named("x", Mode::Targeted, 5, "b.md"),
            named("x", Mode::Targeted, 5, "a.md"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "a.md", "filename order still breaks a user-vs-user tie");
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0].problem.contains("filename order"),
            "a user-vs-user tie keeps the old wording, got: {}",
            issues[0].problem
        );
    }

    /// Case 4: `builtin` is not a front-matter field, so a user cannot forge
    /// built-in status for their own file by writing `builtin: true` in it.
    #[test]
    fn a_user_file_cannot_claim_builtin_status_via_front_matter() {
        let text = "---\nname: x\nbuiltin: true\n---\nbody\n";
        let f = parse_prompt_file("x.md", text).unwrap();
        assert!(!f.builtin, "parse_prompt_file must never set builtin: true");
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

    /// Round-3 finding: `Check {{context` with no closing `}}` used to pass
    /// `validate` cleanly — `placeholders` stops scanning at the unclosed
    /// delimiter (so it never reaches the undeclared-argument check) while
    /// `render` copies the remainder verbatim, so the literal `{{` reached
    /// agent-facing output with no load issue reported anywhere.
    #[test]
    fn validate_rejects_an_unterminated_placeholder() {
        let e = validate(&file_with("Check {{context on the node", &["context"])).unwrap_err();
        assert!(
            e.contains("unterminated") && e.contains("{{"),
            "the message must say the placeholder is unterminated, got: {e}"
        );
    }

    #[test]
    fn validate_accepts_a_body_with_multiple_balanced_placeholders() {
        assert!(validate(&file_with(
            "Check {{context}} then {{namespace}} then {{context}} again",
            &["context", "namespace"]
        ))
        .is_ok());
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
        assert!(
            files.iter().all(|f| f.builtin),
            "builtins() must mark every file it returns as builtin: true"
        );
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

    /// The only tools a built-in prompt body may reference: enough to diagnose
    /// the four documented flows, and nothing that mutates cluster state. This
    /// is a positive list rather than a denylist of known-bad tools — a
    /// denylist is only ever as complete as the last audit, whereas this list
    /// catches a mutating tool AND a typo'd tool name, in CI, with no cluster
    /// required. Adding a genuinely read-only tool a new prompt needs means
    /// adding it here deliberately.
    const ALLOWED_TOOLS: &[&str] = &[
        "k8s.getObject",
        "k8s.listEndpointSlices",
        "k8s.listEvents",
        "k8s.listIngresses",
        "k8s.listLimitRanges",
        "k8s.listNodes",
        "k8s.listPersistentVolumeClaims",
        "k8s.listPods",
        "k8s.listResourceQuotas",
        "k8s.listServices",
        "k8s.listStorageClasses",
        "k8s.nodeMetrics",
        "k8s.podLogs",
        "k8s.podMetrics",
        "k8s.podsForPvc",
        "k8s.podsForSelector",
    ];

    /// Every `k8s.*` / `toolbox.*` token mentioned in `body`, with surrounding
    /// markdown punctuation (backticks, commas, trailing sentence periods,
    /// parens, colons) trimmed off. Same scan-and-trim idea as `placeholders`,
    /// just hunting tool-name tokens instead of `{{...}}` ones.
    fn tool_tokens(body: &str) -> Vec<String> {
        // Trimming '.' along with everything else is safe here: the '.' inside
        // a real token (`k8s.getObject`) sits between two alphanumeric runs, so
        // it is never at either edge and survives the trim. Only an edge '.' —
        // a sentence-ending period glued to the token — gets stripped.
        body.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
            .filter(|w| w.starts_with("k8s.") || w.starts_with("toolbox."))
            .map(str::to_string)
            .collect()
    }

    /// Built-ins diagnose and recommend; they never drive a mutation, and every
    /// tool they name must actually exist. This replaced a denylist
    /// (`no_builtin_instructs_a_mutating_tool_call`) that only caught tools
    /// someone remembered to list — it missed `toolbox.installKubectl` and
    /// several `k8s.*` mutators entirely. The allowlist below is strictly
    /// stronger: it also catches a typo'd tool name that would otherwise reach
    /// users unnoticed until the ignored, cluster-requiring integration test
    /// happened to be run.
    #[test]
    fn every_referenced_tool_is_on_the_read_only_allowlist() {
        let (files, _) = builtins();
        for f in &files {
            for tool in tool_tokens(&f.body) {
                assert!(
                    ALLOWED_TOOLS.contains(&tool.as_str()),
                    "{} references `{tool}`, which is not in ALLOWED_TOOLS; it is \
                     either a mutating tool (built-ins must not instruct one) or a \
                     typo. If it is genuinely a new read-only tool this prompt \
                     needs, add it to ALLOWED_TOOLS deliberately.",
                    f.source
                );
            }
        }
    }

    /// Every argument-KEY token a body writes for a tool call, e.g. the
    /// `context` in `` `context: {{context}}` ``, the `tail_lines` in
    /// `` `tail_lines: 200` ``, the `objectKind` in `` `objectKind: Pod` ``.
    /// Bodies consistently write a call's arguments as backtick-quoted
    /// `key: value` pairs, so scanning for that shape (backtick, an
    /// identifier, a colon) extracts exactly the keys without needing to
    /// parse which tool a given step is calling — the prose isn't structured
    /// enough for that, and this doesn't need it: a key either belongs to
    /// *some* allowlisted capability's input schema or it doesn't. A bare
    /// field reference with no colon, like `` `node` `` (a PodSummary output
    /// field, not an argument), is deliberately NOT matched.
    fn argument_key_tokens(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = body;
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('`') else { break };
            let inside = &after[..end];
            if let Some(colon) = inside.find(':') {
                let key = inside[..colon].trim();
                let is_identifier = !key.is_empty()
                    && key
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if is_identifier {
                    out.push(key.to_string());
                }
            }
            rest = &after[end + 1..];
        }
        out
    }

    /// The guard for the class of bug that let `tailLines` (camelCase; the
    /// real field is `tail_lines`) and the missing/invented keys in FIX 5/6
    /// ship: tool NAMES are checked against `ALLOWED_TOOLS` above, but until
    /// now argument KEYS were only hand-checked. This builds the real
    /// registry (`srelens-registry` is a dev-dependency of this crate for
    /// exactly this — see the comment in `Cargo.toml`) and asserts every key
    /// a built-in body writes is a real `properties` field of at least one
    /// allowlisted capability's `input_schema`. It does not, and cannot from
    /// the prose alone, check that a key belongs to the SPECIFIC tool named
    /// in that step — only that it belongs to the allowlisted set somewhere,
    /// which is what actually catches an invented or misspelled key.
    ///
    /// This is still worth keeping alongside the exact, per-call guard below
    /// (`every_call_supplies_every_required_field_of_its_capability`): that
    /// one only checks that a call's REQUIRED fields are present, so it has
    /// nothing to say about a bogus EXTRA key on an otherwise-complete call
    /// line (e.g. a stray `tailLines` next to a correct `tail_lines`) or
    /// about a mistyped key on a call line this test's stricter grammar
    /// doesn't parse as a call at all. This one catches those, at the cost of
    /// not knowing which specific tool a key belongs to.
    ///
    /// Deterministic and cluster-free: `build_registry` only constructs a
    /// lazy client cache and registers capability closures (no connection
    /// attempt), and `input_schema` comes from `schemars::schema_for!` at
    /// capability-construction time — see the doc comment on the sibling
    /// `mcp_prompts_name_only_real_capabilities` e2e test, which relies on
    /// the same guarantee.
    #[test]
    fn every_argument_key_in_a_builtin_body_is_a_real_capability_input_field() {
        let registry = srelens_registry::build_registry();
        let mut legal_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for cap in registry.entries() {
            if !ALLOWED_TOOLS.contains(&cap.id.as_str()) {
                continue;
            }
            if let Some(props) = cap.input_schema.get("properties").and_then(|p| p.as_object()) {
                legal_fields.extend(props.keys().cloned());
            }
        }
        assert!(
            !legal_fields.is_empty(),
            "sanity check failed: no fields collected from ALLOWED_TOOLS; \
             did every allowlisted capability's input schema come back empty?"
        );

        let (files, _) = builtins();
        for f in &files {
            for key in argument_key_tokens(&f.body) {
                assert!(
                    legal_fields.contains(&key),
                    "{} writes argument key `{key}` in a tool-call, which is not a \
                     field of any ALLOWED_TOOLS capability's input schema. This is \
                     either a typo (e.g. `tailLines` vs `tail_lines`) or a key that \
                     belongs to a tool not on ALLOWED_TOOLS — check the step's tool \
                     name and the real input struct in `crates/kube/src`.",
                    f.source
                );
            }
        }
    }

    /// Every canonical call site in `body`: `(tool, keys)` for each
    /// `` Call `k8s.tool` with `k1: v1`, `k2: v2`. `` sentence, where `keys`
    /// is every backtick `key:` token written between "with" and the period
    /// that ends the sentence (found outside any backtick span, so a period
    /// inside a value like `{{context}}` or `<candidate>` can't terminate it
    /// early). A bare mention like "`k8s.listNodes` does not return any of
    /// this" has no "Call" immediately before the backtick and no "with"
    /// immediately after it, so it is not matched — that is deliberate: it's
    /// what lets an informational mention skip this guard entirely without
    /// special-casing it, per the canonical-form rules the built-in bodies
    /// are held to.
    fn call_sites(body: &str) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        let mut rest = body;
        while let Some(call_at) = rest.find("Call `") {
            let after_call = &rest[call_at + "Call `".len()..];
            let Some(tick_end) = after_call.find('`') else {
                break;
            };
            let tool = after_call[..tick_end].to_string();
            let after_tool = &after_call[tick_end + 1..];
            let Some(with_rest) = after_tool.trim_start().strip_prefix("with") else {
                // Not a call in the canonical shape (e.g. "Call `k8s.X`
                // directly" with no "with" clause) — keep scanning past it
                // rather than treating it as a call with zero keys.
                rest = after_tool;
                continue;
            };

            // The call's key list runs from here to the first '.' that is
            // outside a backtick span.
            let mut in_tick = false;
            let mut end = with_rest.len();
            for (i, c) in with_rest.char_indices() {
                match c {
                    '`' => in_tick = !in_tick,
                    '.' if !in_tick => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            let keys = argument_key_tokens(&with_rest[..end]);
            out.push((tool, keys));
            rest = &with_rest[end..];
        }
        out
    }

    /// The guard PART 1 of the round-2 review asked for: `every_argument_key_
    /// in_a_builtin_body_is_a_real_capability_input_field` above can catch an
    /// INVENTED key (one that belongs to no allowlisted capability at all),
    /// but it has no notion of which tool a key belongs to, so it cannot
    /// catch a call that's simply missing one of ITS tool's required fields —
    /// that key might well be a real field of some *other* allowlisted
    /// capability. This is what let `getObject` omit `name`/`namespace` and
    /// `podLogs` omit `pod`/`namespace` ship across two review rounds.
    ///
    /// This test is exact instead: for every canonical `` Call `k8s.X` with
    /// ... `` sentence (see `call_sites`), it looks up the REAL capability
    /// `k8s.X` in the registry, reads its `input_schema`'s `required` array
    /// (schemars emits this from schema_for!, listing every field that is
    /// neither `Option<_>` nor `#[serde(default)]`), and asserts every one of
    /// those fields appears as a backtick `key:` token on that same call.
    #[test]
    fn every_call_supplies_every_required_field_of_its_capability() {
        let registry = srelens_registry::build_registry();
        let (files, _) = builtins();

        let mut total_calls = 0usize;
        for f in &files {
            let sites = call_sites(&f.body);
            total_calls += sites.len();
            for (tool, keys) in sites {
                let cap = registry.get(&tool).unwrap_or_else(|| {
                    panic!(
                        "{}: `Call \\`{tool}\\` with ...` names a tool that does not exist \
                         in the real capability registry — typo?",
                        f.source
                    )
                });
                let required: Vec<&str> = cap
                    .input_schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                let missing: Vec<&str> = required
                    .iter()
                    .copied()
                    .filter(|r| !keys.iter().any(|k| k == r))
                    .collect();
                assert!(
                    missing.is_empty(),
                    "{}: `Call \\`{tool}\\` with ...` is missing required field(s) {:?} \
                     (supplied keys: {:?})",
                    f.source,
                    missing,
                    keys
                );
            }
        }
        assert!(
            total_calls >= 30,
            "sanity check failed: only found {total_calls} canonical call sites across all \
             built-ins — did `call_sites`'s parsing regress, or did a rewrite stop using the \
             canonical `Call \\`k8s.X\\` with ...` form?"
        );
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

    #[test]
    fn list_collapses_modes_to_one_entry_per_name() {
        let lib = PromptLibrary::new(None);
        let specs = lib.list();
        assert_eq!(specs.len(), 4, "four flows, not eight files");
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pod-crashloop"));
        assert!(names.contains(&"service-no-endpoints"));
    }

    #[test]
    fn list_unions_arguments_across_modes() {
        let lib = PromptLibrary::new(None);
        let spec = lib.list().into_iter().find(|s| s.name == "pod-crashloop").unwrap();
        let names: Vec<&str> = spec.arguments.iter().map(|a| a.name.as_str()).collect();
        // `pod` is declared only by the targeted file, `namespace` by both —
        // the client's form needs all of them.
        assert!(names.contains(&"context"));
        assert!(names.contains(&"namespace"));
        assert!(names.contains(&"pod"));
        assert!(!spec.description.is_empty());
    }

    /// `required` is serialized to the client, so its union across modes must
    /// not accidentally force an argument that discover mode leaves optional:
    /// that would make discover mode unreachable for the prompt. An argument
    /// is required in the merged spec only if every mode declaring it does.
    #[test]
    fn list_reports_an_argument_as_required_only_if_every_mode_requires_it() {
        let dir = temp_dir("required-union");
        write(
            &dir,
            "mixed.targeted.md",
            "---\nname: mixed\nmode: targeted\narguments:\n  - { name: context, required: true }\n  - { name: pod, target: true, required: true }\n  - { name: urgency, required: true }\n---\nTriage `{{pod}}` on `{{context}}`.\n",
        );
        write(
            &dir,
            "mixed.discover.md",
            "---\nname: mixed\nmode: discover\narguments:\n  - { name: context, required: true }\n  - { name: pod, required: false }\n---\nFind candidates on `{{context}}`, maybe `{{pod}}`.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let spec = lib.list().into_iter().find(|s| s.name == "mixed").unwrap();
        let pod = spec.arguments.iter().find(|a| a.name == "pod").unwrap();
        assert!(
            !pod.required,
            "pod is optional in discover mode, so the merged spec must not require it"
        );
        let context = spec.arguments.iter().find(|a| a.name == "context").unwrap();
        assert!(context.required, "context is required in both modes");

        // The other half of the contract: `get()` must actually accept what
        // `list()` advertised as legal. Before the fix, `get()` validated
        // `pod` (required only in the targeted variant) across every variant,
        // so this exact bare call was rejected with `-32602` even though
        // `list()` says `pod` is optional — discover mode was unreachable.
        let bare = lib
            .get("mixed", &args(&[("context", "kind")]))
            .expect("a bare call list() advertises as legal must succeed");
        assert!(
            bare.text.contains("Find candidates"),
            "the discover body must be the one rendered, got: {}",
            bare.text
        );

        // A targeted call must still enforce the targeted variant's OWN
        // required arguments — `urgency` here, which the discover variant
        // does not declare at all, so it can only ever be checked against
        // `chosen`, never unioned across variants.
        let err = lib
            .get("mixed", &args(&[("context", "kind"), ("pod", "web-0")]))
            .unwrap_err();
        assert!(
            err.contains("urgency"),
            "the targeted variant's own required argument must still be enforced, got: {err}"
        );
    }

    /// The specific shape of the round-2 regression: the discover variant
    /// does not merely mark `pod` `required: false` (covered above) — it
    /// omits the argument from its front matter entirely, which is the
    /// common case (a discover file has no target to declare at all). Before
    /// the fix, the `None` branch of the merge pushed `pod` with its
    /// originally-declared `required: true` from the targeted file and
    /// nothing ever relaxed it, so `list()` advertised `pod` as always
    /// required and a conforming client could never invoke discover mode.
    #[test]
    fn list_does_not_require_an_argument_a_variant_omits_entirely() {
        let dir = temp_dir("omitted-argument");
        write(
            &dir,
            "solo.targeted.md",
            "---\nname: solo\nmode: targeted\narguments:\n  - { name: context, required: true }\n  - { name: pod, target: true, required: true }\n---\nTriage `{{pod}}` on `{{context}}`.\n",
        );
        write(
            &dir,
            "solo.discover.md",
            "---\nname: solo\nmode: discover\narguments:\n  - { name: context, required: true }\n---\nFind candidates on `{{context}}`.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let spec = lib.list().into_iter().find(|s| s.name == "solo").unwrap();
        let pod = spec.arguments.iter().find(|a| a.name == "pod").unwrap();
        assert!(
            !pod.required,
            "discover doesn't declare `pod` at all, so the merged spec must not require it"
        );

        let out = lib
            .get("solo", &args(&[("context", "kind")]))
            .expect("a bare call with only `context` must succeed and render discover mode");
        assert!(
            out.text.contains("Find candidates"),
            "the discover body must be the one rendered, got: {}",
            out.text
        );
    }

    #[test]
    fn get_uses_the_targeted_mode_when_the_target_is_supplied() {
        let lib = PromptLibrary::new(None);
        let out = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "web-0")]))
            .unwrap();
        assert!(out.text.contains("web-0"), "the pod name must be substituted");
        assert!(out.text.contains("previous"), "targeted triage reads previous logs");
        assert!(!out.text.contains("{{"), "no placeholder may survive");
    }

    #[test]
    fn get_uses_the_discover_mode_when_the_target_is_omitted() {
        let lib = PromptLibrary::new(None);
        let out = lib.get("pod-crashloop", &args(&[("context", "kind")])).unwrap();
        assert!(out.text.contains("k8s.listPods"), "discovery starts by listing pods");
        assert!(!out.text.contains("{{"));
    }

    /// A present-but-empty target (an unfilled form field submitted as `""`)
    /// is not a target: it must fall back to discover rather than rendering
    /// instructions that name an empty resource.
    #[test]
    fn get_uses_the_discover_mode_when_the_target_is_an_empty_string() {
        let lib = PromptLibrary::new(None);
        let out = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "")]))
            .unwrap();
        assert!(out.text.contains("k8s.listPods"), "discovery starts by listing pods");
        assert!(!out.text.contains("{{"));
    }

    /// Same as above, for whitespace-only values.
    #[test]
    fn get_uses_the_discover_mode_when_the_target_is_whitespace_only() {
        let lib = PromptLibrary::new(None);
        let out = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "   ")]))
            .unwrap();
        assert!(out.text.contains("k8s.listPods"), "discovery starts by listing pods");
        assert!(!out.text.contains("{{"));
    }

    /// The simplest legal prompt file: one variant, `mode: targeted`, and no
    /// `target: true` argument at all. Before the fix, `targets` was empty,
    /// `all_targets_supplied` was false, `wanted` was forced to `Discover`,
    /// and `get()` failed every single call with a message naming no
    /// argument (`supply the `` argument`) because there was nothing to name.
    #[test]
    fn get_renders_a_single_targeted_variant_with_no_target_argument() {
        let dir = temp_dir("single-targeted");
        write(
            &dir,
            "solo.md",
            "---\nname: solo\nmode: targeted\narguments:\n  - { name: context, required: true }\n---\nCheck `{{context}}` end to end.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let out = lib.get("solo", &args(&[("context", "kind")])).unwrap();
        assert!(out.text.contains("Check `kind` end to end."), "got: {}", out.text);
    }

    /// Same, for the `mode: discover` variant of a no-target prompt.
    #[test]
    fn get_renders_a_single_discover_variant_with_no_target_argument() {
        let dir = temp_dir("single-discover");
        write(
            &dir,
            "solo.md",
            "---\nname: solo\nmode: discover\narguments:\n  - { name: context, required: true }\n---\nSurvey `{{context}}` broadly.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let out = lib.get("solo", &args(&[("context", "kind")])).unwrap();
        assert!(out.text.contains("Survey `kind` broadly."), "got: {}", out.text);
    }

    /// The two-mode, target-declared behaviour must be unchanged by the
    /// no-target fix above: supplying the target still selects Targeted, and
    /// omitting it still selects Discover.
    #[test]
    fn get_still_uses_targeted_or_discover_based_on_the_target_for_a_two_mode_prompt() {
        let lib = PromptLibrary::new(None);
        let targeted = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "web-0")]))
            .unwrap();
        assert!(targeted.text.contains("web-0"), "target supplied must pick targeted");

        let discover = lib.get("pod-crashloop", &args(&[("context", "kind")])).unwrap();
        assert!(
            discover.text.contains("k8s.listPods"),
            "target omitted must still pick discover"
        );
    }

    #[test]
    fn get_rejects_a_blank_required_argument() {
        let lib = PromptLibrary::new(None);
        let e = lib
            .get("pod-crashloop", &args(&[("context", ""), ("pod", "web-0")]))
            .unwrap_err();
        assert!(e.contains("context"), "an empty required value must be treated as missing, got: {e}");
    }

    #[test]
    fn get_rejects_a_whitespace_only_required_argument() {
        let lib = PromptLibrary::new(None);
        let e = lib
            .get("pod-crashloop", &args(&[("context", "   "), ("pod", "web-0")]))
            .unwrap_err();
        assert!(
            e.contains("context"),
            "a whitespace-only required value must be treated as missing, got: {e}"
        );
    }

    #[test]
    fn get_accepts_a_normal_required_argument() {
        let lib = PromptLibrary::new(None);
        let out = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "web-0")]))
            .unwrap();
        assert!(out.text.contains("kind"));
    }

    #[test]
    fn get_rejects_an_unknown_prompt() {
        let lib = PromptLibrary::new(None);
        let e = lib.get("nope", &args(&[("context", "kind")])).unwrap_err();
        assert!(e.contains("nope"), "got: {e}");
    }

    #[test]
    fn get_rejects_a_missing_required_argument() {
        let lib = PromptLibrary::new(None);
        let e = lib.get("pod-crashloop", &args(&[])).unwrap_err();
        assert!(e.contains("context"), "the message must name the argument, got: {e}");
    }

    #[test]
    fn a_user_file_with_higher_priority_overrides_a_builtin() {
        let dir = temp_dir("override");
        write(
            &dir,
            "mine.md",
            "---\nname: pod-crashloop\ndescription: Mine\nmode: targeted\npriority: 10\narguments:\n  - { name: context, required: true }\n  - { name: pod, target: true }\n---\nMy own triage for `{{pod}}` on `{{context}}`.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let out = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "web-0")]))
            .unwrap();
        assert!(out.text.contains("My own triage"), "the override must win");
        // The discover mode was not overridden, so it is inherited.
        let disc = lib.get("pod-crashloop", &args(&[("context", "kind")])).unwrap();
        assert!(disc.text.contains("k8s.listPods"));
    }

    #[test]
    fn a_user_file_at_default_priority_does_not_override_a_builtin() {
        let dir = temp_dir("nooverride");
        write(
            &dir,
            "mine.md",
            "---\nname: pod-crashloop\nmode: targeted\narguments:\n  - { name: context, required: true }\n  - { name: pod, target: true }\n---\nMy own triage for `{{pod}}` on `{{context}}`.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let out = lib
            .get("pod-crashloop", &args(&[("context", "kind"), ("pod", "web-0")]))
            .unwrap();
        assert!(!out.text.contains("My own triage"), "priority 0 must not shadow a built-in");
        assert!(lib.issues().iter().any(|i| i.file == "mine.md"), "the tie must be reported");
    }

    #[test]
    fn issues_from_a_bad_user_file_are_exposed() {
        let dir = temp_dir("issues");
        write(&dir, "broken.md", "not a prompt\n");
        let lib = PromptLibrary::new(Some(dir));
        assert!(lib.issues().iter().any(|i| i.file == "broken.md"));
        assert_eq!(lib.list().len(), 4, "the built-ins still work");
    }

    #[test]
    fn get_rejects_a_prompt_whose_needed_mode_is_absent() {
        let dir = temp_dir("nomode");
        write(
            &dir,
            "only.md",
            "---\nname: only-targeted\nmode: targeted\narguments:\n  - { name: context, required: true }\n  - { name: pod, target: true }\n---\nTriage `{{pod}}` on `{{context}}`.\n",
        );
        let lib = PromptLibrary::new(Some(dir));
        let e = lib.get("only-targeted", &args(&[("context", "kind")])).unwrap_err();
        assert!(e.contains("pod"), "must name the argument that would select a mode, got: {e}");
    }
}
