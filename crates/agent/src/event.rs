//! The transport-agnostic event stream every agent adapter normalizes to.

use serde::{Deserialize, Serialize};

/// Outcome of a tool call, as the drawer shows it on a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Ok,
    Error,
    Denied,
}

/// Stable prefix the srelens MCP server puts on the text of a consent-denied
/// tool result (`srelens_mcp::stdio::DENIED_PREFIX` — same value; neither
/// crate depends on the other, and the desktop crate pins the two equal).
/// CLI transports strip the MCP `_meta` denial marker, so this text is the
/// only signal that survives an agent CLI's transcript.
pub const DENIED_PREFIX: &str = "consent denied: ";

/// What a tool's own result says, in a few words, for the transcript's row
/// (#385). Only what the text supports: a count where the result is one list,
/// an object's kind and name, or the first line of plain text or of an error.
/// Any other structured result gets no summary rather than a JSON fragment.
pub fn summarize_result(text: &str, is_error: bool) -> Option<String> {
    let first_line = || {
        text.lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| bound(&mask_credentials(l)))
    };
    if is_error {
        return first_line();
    }
    match serde_json::from_str::<serde_json::Value>(text.trim()) {
        Ok(serde_json::Value::Array(items)) => Some(bound(&format!("{} items", items.len()))),
        Ok(serde_json::Value::Object(map)) => {
            let arrays: Vec<(&String, usize)> = map
                .iter()
                .filter_map(|(k, v)| v.as_array().map(|a| (k, a.len())))
                .collect();
            // One list, counted once even when a known second view of it sits
            // beside it (`VIEW_FIELDS`, PR #806 review). Arrays that merely
            // share a length are not one list, and say nothing.
            let (views, lists): (Vec<_>, Vec<_>) = arrays
                .iter()
                .partition(|(k, _)| VIEW_FIELDS.contains(&k.as_str()));
            let counted = match (lists.as_slice(), views.as_slice()) {
                ([list], views) if views.iter().all(|(_, len)| *len == list.1) => Some(*list),
                ([], [only]) => Some(*only),
                _ => None,
            };
            if let Some((field, n)) = counted {
                let noun = if n == 1 {
                    singular(field)
                } else {
                    field.to_string()
                };
                return Some(bound(&format!("{n} {noun}")));
            }
            let kind = map.get("kind").and_then(|k| k.as_str());
            let name = map
                .get("metadata")
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str());
            match (kind, name) {
                (Some(kind), Some(name)) => Some(bound(&format!("{kind} {name}"))),
                _ => None,
            }
        }
        Ok(_) => None,
        Err(_) => first_line(),
    }
}

/// Control characters stripped, then cut to 80 characters with `…`.
fn bound(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= 80 {
        clean
    } else {
        let mut cut: String = clean.chars().take(79).collect();
        cut.push('…');
        cut
    }
}

/// Array fields that are a second view of the list beside them, not a list of
/// their own: `k8s.listNamespaces` returns its namespaces as names and again
/// as `summaries` rows.
const VIEW_FIELDS: &[&str] = &["summaries"];

const REDACTED: &str = "[redacted]";

/// Words that, as the key of a `key=value` or `key: value`, name a credential.
/// Matched inside the key, so `DB_PASSWORD` and `x-api-key` count.
const CREDENTIAL_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "access_key",
    "private_key",
    "credential",
    "authorization",
];

/// Credentials in well-known shapes, masked in a line of free text before it
/// becomes a summary (PR #806 review): a summary is written into the saved
/// conversation, and a first line can be a shell's `env` or a log line. Best
/// effort by design — it catches the shapes credentials usually take (a
/// credential-named key's value, a `Bearer`/`Basic` value, GitHub/OpenAI/Slack
/// /AWS token prefixes, a JWT, a PEM private key), not every secret there is.
fn mask_credentials(line: &str) -> String {
    if line.contains("PRIVATE KEY") {
        return REDACTED.to_string();
    }
    // Words are cut at any whitespace (PR #806 review: a tab too), and every
    // separator is kept, so the line reads as it did.
    let mut out = String::with_capacity(line.len());
    let mut mask_next = false;
    let mut rest = line;
    while !rest.is_empty() {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(end);
        out.push_str(&mask_word(word, &mut mask_next));
        let gap = tail
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(tail.len());
        out.push_str(&tail[..gap]);
        rest = &tail[gap..];
    }
    out
}

fn is_scheme(word: &str) -> bool {
    word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic")
}

/// One whitespace-separated word. Its pairs — joined by the `,` `;` `&` `?` of
/// env lines, cookies and URL queries — are read left to right, so a
/// credential after an ordinary pair is found (PR #806 review). A credential's
/// value runs to the end of the word: a value with a comma in it is masked
/// whole, never shown in part, at the price of masking what follows it there.
/// `mask_next` carries a value that is the next word: `password: hunter2`,
/// `Bearer abc`.
fn mask_word(word: &str, mask_next: &mut bool) -> String {
    if word.is_empty() {
        return String::new();
    }
    if is_scheme(word) {
        *mask_next = true;
        return word.to_string();
    }
    if *mask_next {
        *mask_next = false;
        return REDACTED.to_string();
    }
    let is_joiner = |c: char| matches!(c, ',' | ';' | '&' | '?');
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    loop {
        let end = rest.find(is_joiner).unwrap_or(rest.len());
        let (piece, tail) = rest.split_at(end);
        if let Some(at) = piece.find(['=', ':']) {
            let lower = piece[..at].to_ascii_lowercase();
            let key = lower.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if !key.is_empty() && CREDENTIAL_KEYS.iter().any(|k| key.contains(k)) {
                let value = piece[at + 1..].trim_matches(|c| c == '"' || c == '\'');
                if tail.is_empty() && (value.is_empty() || is_scheme(value)) {
                    // The value is the next word, or a scheme whose value is.
                    out.push_str(piece);
                    *mask_next = true;
                    return out;
                }
                out.push_str(&piece[..=at]);
                out.push_str(REDACTED);
                return out;
            }
        }
        out.push_str(if looks_like_a_token(piece) {
            REDACTED
        } else {
            piece
        });
        let gap = tail.find(|c: char| !is_joiner(c)).unwrap_or(tail.len());
        out.push_str(&tail[..gap]);
        rest = &tail[gap..];
        if rest.is_empty() {
            return out;
        }
    }
}

/// A word in the shape of a well-known token: GitHub (`ghp_` …), OpenAI and
/// Anthropic (`sk-`), Slack (`xox?-`), an AWS access key id, or a JWT.
fn looks_like_a_token(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
    let prefixed = [
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "ghr_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
    ]
    .iter()
    .any(|p| w.starts_with(p) && w.len() > p.len() + 8);
    let openai = w.starts_with("sk-") && w.len() >= 20;
    let aws = w.len() == 20
        && w.starts_with("AKIA")
        && w.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let jwt = w.starts_with("eyJ") && w.matches('.').count() == 2;
    prefixed || openai || aws || jwt
}

/// The singular a plural field name really has (PR #806 review):
/// `networkpolicies` → `networkpolicy`, `ingresses` → `ingress`, `matches` →
/// `match`, `pods` → `pod`. Cutting the last letter wrote `networkpolicie`.
fn singular(plural: &str) -> String {
    if let Some(stem) = plural.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if ["sses", "ches", "shes", "xes"]
        .iter()
        .any(|ending| plural.ends_with(ending))
    {
        return plural[..plural.len() - 2].to_string();
    }
    plural.strip_suffix('s').unwrap_or(plural).to_string()
}

/// Whether a failed tool result's text is a consent refusal rather than an
/// execution error. `contains`, not `starts_with`: a CLI may wrap the tool
/// text (e.g. "Error: …"), and result text only ever comes from our own MCP
/// server, so a false positive would require the server itself to emit the
/// marker mid-output.
pub fn is_denial_text(text: &str) -> bool {
    text.contains(DENIED_PREFIX)
}

/// One normalized event from any agent CLI. `#[serde(tag = "type")]` so the
/// WebView switches on a single discriminant, camelCase to match the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// A chunk of streamed assistant text.
    TextDelta { text: String },
    /// The agent has begun a tool call. `id` correlates with the matching
    /// `ToolResult`.
    ToolCallStart {
        id: String,
        tool: String,
        args: serde_json::Value,
    },
    /// A tool call finished with this status — and, when its result says
    /// something short and honest, what (#385, see [`summarize_result`]).
    ToolResult {
        id: String,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    /// A chunk of the agent's internal reasoning/thinking, shown separately
    /// from its final response text.
    Thinking { text: String },
    /// Token usage reported by the provider for the current turn.
    #[serde(rename_all = "camelCase")]
    Usage {
        prompt_tokens: usize,
        completion_tokens: usize,
        cached_tokens: usize,
        total_tokens: usize,
    },
    /// The agent finished this turn and is waiting for the next user message.
    TurnDone,
    /// A fatal error for this turn (parse failure, process died, transport).
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_serializes_with_a_tagged_type() {
        let e = AgentEvent::TextDelta { text: "hi".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "textDelta");
        assert_eq!(v["text"], "hi");
    }

    #[test]
    fn tool_call_start_carries_name_and_args() {
        let e = AgentEvent::ToolCallStart {
            id: "t1".into(),
            tool: "k8s.listPods".into(),
            args: serde_json::json!({ "namespace": "default" }),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "toolCallStart");
        assert_eq!(v["tool"], "k8s.listPods");
        assert_eq!(v["args"]["namespace"], "default");
    }

    #[test]
    fn tool_result_reports_a_status() {
        let e = AgentEvent::ToolResult {
            id: "t1".into(),
            status: ToolStatus::Ok,
            summary: None,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "toolResult");
        assert_eq!(v["status"], "ok");
    }

    #[test]
    fn thinking_serializes_with_a_tagged_type() {
        let e = AgentEvent::Thinking {
            text: "pondering...".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "thinking");
        assert_eq!(v["text"], "pondering...");
    }

    #[test]
    fn usage_serializes_with_a_tagged_type() {
        let e = AgentEvent::Usage {
            prompt_tokens: 1500,
            completion_tokens: 200,
            cached_tokens: 500,
            total_tokens: 1700,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "usage");
        assert_eq!(v["promptTokens"], 1500);
        assert_eq!(v["completionTokens"], 200);
        assert_eq!(v["cachedTokens"], 500);
        assert_eq!(v["totalTokens"], 1700);
    }

    #[test]
    fn error_and_turn_done_are_distinct_variants() {
        assert_eq!(
            serde_json::to_value(AgentEvent::TurnDone).unwrap()["type"],
            "turnDone"
        );
        let err = AgentEvent::Error {
            message: "boom".into(),
        };
        assert_eq!(serde_json::to_value(&err).unwrap()["type"], "error");
    }

    /// #385: a tool call's row says what its result says — only as much as the
    /// text supports, and nothing for a structure it cannot read honestly.
    #[test]
    fn a_result_is_summarised_by_what_it_actually_says() {
        let cases: &[(&str, bool, Option<&str>)] = &[
            (
                r#"{"pods":[{"name":"a"},{"name":"b"},{"name":"c"}]}"#,
                false,
                Some("3 pods"),
            ),
            (
                r#"{"replicasets":[{"name":"a"}]}"#,
                false,
                Some("1 replicaset"),
            ),
            (r#"[1,2]"#, false, Some("2 items")),
            (
                r#"{"kind":"Pod","metadata":{"name":"api-0"}}"#,
                false,
                Some("Pod api-0"),
            ),
            (r#"{"a":1,"b":[1],"c":[2,3]}"#, false, None),
            ("first line\nsecond", false, Some("first line")),
            ("\n\n  spaced  \n", false, Some("spaced")),
            (
                "consent denied: user declined `k8s.scale`\nmore",
                true,
                Some("consent denied: user declined `k8s.scale`"),
            ),
            ("", false, None),
        ];
        for (text, is_error, want) in cases {
            assert_eq!(
                summarize_result(text, *is_error).as_deref(),
                *want,
                "for {text:?}"
            );
        }
    }

    /// PR #806 review: one of something reads in the singular the word really
    /// has, not the plural with its last letter cut off.
    #[test]
    fn a_single_result_is_named_in_its_real_singular() {
        for (field, want) in [
            ("networkpolicies", "1 networkpolicy"),
            ("ingresses", "1 ingress"),
            ("storageclasses", "1 storageclass"),
            ("matches", "1 match"),
            ("pods", "1 pod"),
            ("releases", "1 release"),
        ] {
            let text = format!(r#"{{"{field}":[{{}}]}}"#);
            assert_eq!(
                summarize_result(&text, false).as_deref(),
                Some(want),
                "for {field}"
            );
        }
    }

    /// PR #806 review: `k8s.listNamespaces` returns its namespaces twice, as
    /// names and as `summaries` rows; that one list is counted once, whatever
    /// order the object holds them in. Arrays that merely share a length are
    /// not one list — a graph's nodes and edges, a list beside its errors —
    /// and say nothing short and honest.
    #[test]
    fn only_known_views_of_one_list_are_counted_once() {
        let names_first = r#"{"namespaces":["a","b"],"summaries":[{},{}]}"#;
        let rows_first = r#"{"summaries":[{},{}],"namespaces":["a","b"]}"#;
        assert_eq!(
            summarize_result(names_first, false).as_deref(),
            Some("2 namespaces")
        );
        assert_eq!(
            summarize_result(rows_first, false).as_deref(),
            Some("2 namespaces")
        );
        assert_eq!(
            summarize_result(r#"{"namespaces":["a"],"summaries":[{},{}]}"#, false),
            None
        );
        assert_eq!(
            summarize_result(r#"{"edges":[1,2],"nodes":[1,2]}"#, false),
            None
        );
        assert_eq!(
            summarize_result(r#"{"errors":["failed"],"pods":["api-0"]}"#, false),
            None
        );
        assert_eq!(
            summarize_result(r#"{"pods":[1,2],"warnings":[1]}"#, false),
            None
        );
    }

    /// PR #806 review: a summary is written into the saved conversation, and
    /// a first line of free text can be anything — a shell's `env`, a log line.
    /// Credentials in well-known shapes are masked before it is made.
    #[test]
    fn a_credential_in_a_first_line_is_masked() {
        let cases = [
            ("API_KEY=sk-abcdef0123456789abcdef", "API_KEY=[redacted]"),
            (
                "db password: hunter2 for admin",
                "db password: [redacted] for admin",
            ),
            (
                "Authorization: Bearer eyJhbGciOi.eyJzdWIiOi.c2ln",
                "Authorization: Bearer [redacted]",
            ),
            (
                "pushed with ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                "pushed with [redacted]",
            ),
            (
                "aws key AKIAIOSFODNN7EXAMPLE in use",
                "aws key [redacted] in use",
            ),
            ("-----BEGIN RSA PRIVATE KEY-----", "[redacted]"),
        ];
        for (line, want) in cases {
            assert_eq!(
                summarize_result(line, false).as_deref(),
                Some(want),
                "for {line:?}"
            );
            assert_eq!(
                summarize_result(line, true).as_deref(),
                Some(want),
                "error {line:?}"
            );
        }
        // Ordinary results are left exactly as they were.
        for line in [
            "5 pods running",
            "handler error: failed to load current context: no-such-context",
        ] {
            assert_eq!(summarize_result(line, false).as_deref(), Some(line));
        }
    }

    /// PR #806 review: a credential is masked wherever its pair sits — after a
    /// tab, after another pair in the same word, or in a URL's query — and its
    /// value is masked whole, to the end of its word: a value with a comma in
    /// it must not show its tail. Over-masking what follows in that word is the
    /// price. (A tab is a control character, so the bounded summary drops it.)
    #[test]
    fn a_credential_is_masked_whatever_separates_it() {
        let cases = [
            ("Bearer\tsecret-value-123", "Bearer[redacted]"),
            ("user=bob,password=hunter2", "user=bob,password=[redacted]"),
            ("user=bob;secret=abc", "user=bob;secret=[redacted]"),
            ("password=abc,def is set", "password=[redacted] is set"),
            (
                "callback https://x.test/cb?token=abc123&state=1",
                "callback https://x.test/cb?token=[redacted]",
            ),
            (
                "Authorization:Bearer abc123",
                "Authorization:Bearer [redacted]",
            ),
        ];
        for (line, want) in cases {
            assert_eq!(
                summarize_result(line, false).as_deref(),
                Some(want),
                "for {line:?}"
            );
        }
    }

    #[test]
    fn a_summary_is_bounded_and_clean() {
        let long = "x".repeat(200);
        let got = summarize_result(&long, false).unwrap();
        assert_eq!(got.chars().count(), 80);
        assert!(got.ends_with('…'));
        assert_eq!(
            summarize_result("a\u{1b}[31mred", false).as_deref(),
            Some("a[31mred")
        );
    }

    #[test]
    fn a_result_without_a_summary_serialises_as_before() {
        let v = serde_json::to_value(AgentEvent::ToolResult {
            id: "t".into(),
            status: ToolStatus::Ok,
            summary: None,
        })
        .unwrap();
        assert!(v.get("summary").is_none(), "got {v}");
    }
}
