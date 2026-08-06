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
}
