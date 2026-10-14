//! Redaction for one app's log lines (#575), applied on the way into the
//! buffer and to text shown beside it.
//!
//! Two kinds of rule, because a log line is free text:
//!
//! - **Values the host knows.** A secret, token or password the host has
//!   handed the app, or read on its behalf, is scrubbed wherever it appears,
//!   verbatim or JSON-escaped ([`Scrubber::remember`]). This is the guarantee:
//!   a value srelens knows is secret never reaches the buffer.
//! - **Shapes that carry credentials.** An `Authorization` header, a bearer or
//!   basic credential, a URL's user and password, a `key=value` whose key
//!   names a credential (`token`, `password`, `client-key-data`, `sig`…), a
//!   JWT (a Kubernetes service account token is one), a PEM private key, and
//!   the prefixes of common API tokens. This is best effort: nothing can
//!   recognise every secret in arbitrary text, and a sidecar that means to
//!   hide one in its own log can. What it catches is a credential written by
//!   mistake, which is how they end up in logs.
//!
//! Over-scrubbing is the safe direction, as in the audit trail
//! (`srelens_capability::audit`): `author=` loses its value too, and a log
//! reader loses a word, where the alternative is a credential on screen.
//!
//! Every pattern is compiled by the `regex` crate, which matches in time
//! linear in the line, so nothing a sidecar writes can make redaction slow.

use regex::{Captures, Regex};
use std::sync::OnceLock;

/// What a redacted value is replaced with. The audit trail's sentinel, so a
/// reader who knows one knows both.
pub(super) const REDACTED: &str = "<redacted>";

struct Patterns {
    header: Regex,
    scheme: Regex,
    userinfo: Regex,
    key_value: Regex,
    jwt: Regex,
    token: Regex,
    pem_begin: Regex,
    pem_end: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let compile = |pattern: &str| Regex::new(pattern).expect("a valid redaction pattern");
        Patterns {
            // A header whose whole value is a credential: the rest of the line goes.
            header: compile(
                r"(?i)\b(authorization|proxy-authorization|set-cookie|cookie|x-api-key|x-auth-token|x-amz-security-token)(\s*[:=]\s*)\S.*$",
            ),
            // `Bearer <token>`, `Basic <base64>`; the word after is judged in `scheme_credential`.
            scheme: compile(r"(?i)\b(bearer|basic)(\s+)([A-Za-z0-9._~+/=-]{6,})"),
            // `scheme://user:password@host`: the user goes with the password.
            userinfo: compile(r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/@]+@"),
            // `key=value`, `key: value`, `"key": "value"`, where the key names a credential.
            key_value: compile(
                r#"(?i)([A-Za-z0-9_.-]*(?:token|secret|passw(?:or)?d|pwd|api[_-]?key|access[_-]?key|private[_-]?key|secret[_-]?key|client[_-]?key|signing[_-]?key|credential|signature|session[_-]?id|auth|\bsig)[A-Za-z0-9_.-]*["']?\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s"',;&)\]}]+)"#,
            ),
            jwt: compile(r"\beyJ[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]*"),
            // GitHub, GitLab, Slack, AWS access key IDs, Google API keys, Stripe, Anthropic.
            token: compile(
                r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|xox[abposr]-[A-Za-z0-9-]{10,}|(?:AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|sk_live_[0-9A-Za-z]{16,}|sk-ant-[A-Za-z0-9_-]{16,})",
            ),
            pem_begin: compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
            pem_end: compile(r"-----END [A-Z0-9 ]*PRIVATE KEY-----"),
        }
    })
}

/// Whether the word after `Bearer` or `Basic` is a credential rather than
/// prose ("basic scanning"): it carries a digit or a token character, or is
/// long.
fn scheme_credential(word: &str) -> bool {
    word.len() >= 20
        || word
            .chars()
            .any(|c| c.is_ascii_digit() || "._~+/=-".contains(c))
}

/// A line of a PEM private key's body, or one of its headers.
fn key_material(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("Proc-Type:")
        || line.starts_with("DEK-Info:")
        || line
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// One app's redaction: the values it scrubs, and whether it is inside a
/// private key written over several lines.
#[derive(Default)]
pub(super) struct Scrubber {
    /// Longest first, so a value that contains another is not left half
    /// visible.
    values: Vec<String>,
    /// Oldest first, to drop the oldest when there are too many.
    order: Vec<String>,
    in_private_key: bool,
}

impl Scrubber {
    /// Scrub `value`, verbatim and JSON-escaped, keeping at most `most` values.
    pub(super) fn remember(&mut self, value: &str, most: usize) {
        if value.is_empty() || self.order.iter().any(|known| known == value) {
            return;
        }
        self.order.push(value.to_owned());
        if self.order.len() > most {
            self.order.remove(0);
        }
        self.values.clear();
        for known in &self.order {
            self.values.push(known.clone());
            let escaped = serde_json::to_string(known).unwrap_or_default();
            let escaped = escaped.trim_matches('"');
            if escaped != known {
                self.values.push(escaped.to_owned());
            }
        }
        self.values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    }

    /// One line of the log. A private key's body is recognised across lines:
    /// from its `BEGIN` line to its `END` line, every line is key material.
    pub(super) fn line(&mut self, line: &str) -> String {
        let p = patterns();
        if self.in_private_key {
            let trimmed = line.trim();
            if p.pem_end.is_match(trimmed) {
                self.in_private_key = false;
                return REDACTED.to_owned();
            }
            if key_material(trimmed) {
                return REDACTED.to_owned();
            }
            // Something else: the key ended without its END line.
            self.in_private_key = false;
        }
        if let Some(begin) = p.pem_begin.find(line) {
            self.in_private_key = !p.pem_end.is_match(&line[begin.end()..]);
            return format!("{}{REDACTED}", self.text(&line[..begin.start()]));
        }
        self.text(line)
    }

    /// `text` with every known value and every credential shape redacted.
    pub(super) fn text(&self, text: &str) -> String {
        let p = patterns();
        let mut out = text.to_owned();
        for value in &self.values {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), REDACTED);
            }
        }
        if let Some(begin) = p.pem_begin.find(&out) {
            out.truncate(begin.start());
            out.push_str(REDACTED);
        }
        let out = p
            .header
            .replace_all(&out, format!("${{1}}${{2}}{REDACTED}"));
        let out = p.scheme.replace_all(&out, |c: &Captures| {
            if scheme_credential(&c[3]) {
                format!("{}{}{REDACTED}", &c[1], &c[2])
            } else {
                c[0].to_owned()
            }
        });
        let out = p.userinfo.replace_all(&out, format!("${{1}}{REDACTED}@"));
        let out = p.key_value.replace_all(&out, |c: &Captures| {
            let value = &c[2];
            match value.chars().next() {
                Some(quote @ ('"' | '\'')) => format!("{}{quote}{REDACTED}{quote}", &c[1]),
                _ => format!("{}{REDACTED}", &c[1]),
            }
        });
        let out = p.jwt.replace_all(&out, REDACTED);
        let out = p.token.replace_all(&out, REDACTED);
        out.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(text: &str) -> String {
        Scrubber::default().text(text)
    }

    #[test]
    fn a_known_value_is_scrubbed_verbatim_and_json_escaped() {
        let mut scrubber = Scrubber::default();
        scrubber.remember("pa\"ss\\word", 8);
        let out = scrubber.text(r#"login "pa"ss\word" then {"p":"pa\"ss\\word"}"#);
        assert!(!out.contains("ss\\word"), "{out}");
        assert!(!out.contains(r#"ss\\word"#), "{out}");
        assert!(out.starts_with("login "), "{out}");
    }

    #[test]
    fn the_longest_known_value_goes_first() {
        let mut scrubber = Scrubber::default();
        scrubber.remember("abc", 8);
        scrubber.remember("abcdef", 8);
        assert_eq!(scrubber.text("x abcdef y"), "x <redacted> y");
    }

    #[test]
    fn only_the_newest_values_are_kept() {
        let mut scrubber = Scrubber::default();
        for n in 0..5 {
            scrubber.remember(&format!("value-{n}"), 3);
        }
        assert_eq!(
            scrubber.text("value-1 value-2 value-4"),
            "value-1 <redacted> <redacted>"
        );
    }

    #[test]
    fn an_empty_value_scrubs_nothing() {
        let mut scrubber = Scrubber::default();
        scrubber.remember("", 8);
        assert_eq!(scrubber.text("ready"), "ready");
    }

    #[test]
    fn a_credential_header_loses_its_whole_value() {
        assert_eq!(
            redact("request failed: Authorization: Bearer abc.def"),
            "request failed: Authorization: <redacted>"
        );
        assert_eq!(
            redact("Cookie: session=1; theme=dark"),
            "Cookie: <redacted>"
        );
        assert_eq!(redact("x-api-key=0123"), "x-api-key=<redacted>");
    }

    #[test]
    fn a_bearer_or_basic_credential_goes_and_prose_stays() {
        assert_eq!(redact("sent Bearer s3cr3t-t0ken"), "sent Bearer <redacted>");
        assert_eq!(redact("basic dXNlcjpwYXNz="), "basic <redacted>");
        assert_eq!(redact("basic scanning complete"), "basic scanning complete");
    }

    #[test]
    fn a_urls_user_and_password_go_and_its_host_stays() {
        assert_eq!(
            redact("pull https://robot:hunter2@registry.example/v2/app failed"),
            "pull https://<redacted>@registry.example/v2/app failed"
        );
        assert_eq!(
            redact("see https://example.com/@user"),
            "see https://example.com/@user"
        );
    }

    #[test]
    fn a_value_under_a_credential_key_goes() {
        let cases = [
            ("token=abc123 next", "token=<redacted> next"),
            (
                "GET /v2?access_token=abc&page=2",
                "GET /v2?access_token=<redacted>&page=2",
            ),
            (
                "url?X-Amz-Signature=deadbeef&x=1",
                "url?X-Amz-Signature=<redacted>&x=1",
            ),
            ("url?sig=deadbeef", "url?sig=<redacted>"),
            (
                r#"{"password": "two words", "user": "bob"}"#,
                r#"{"password": "<redacted>", "user": "bob"}"#,
            ),
            (
                "client-key-data: LS0tLS1CRUdJTg==",
                "client-key-data: <redacted>",
            ),
            ("apiKey='k-1'", "apiKey='<redacted>'"),
            ("clientSecret=xyz, retry", "clientSecret=<redacted>, retry"),
        ];
        for (line, want) in cases {
            assert_eq!(redact(line), want, "{line}");
        }
    }

    #[test]
    fn a_word_that_only_mentions_a_credential_stays() {
        for line in [
            "the secret default/db was not found",
            "token expired, asking again",
            "signature valid",
        ] {
            assert_eq!(redact(line), line);
        }
    }

    #[test]
    fn a_jwt_goes() {
        let token = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJzeXN0ZW0ifQ.c2lnbmF0dXJl";
        assert_eq!(
            redact(&format!("using {token} now")),
            "using <redacted> now"
        );
    }

    #[test]
    fn a_common_api_token_goes() {
        for token in [
            "ghp_0123456789abcdefghijABCDEFGHIJ012345",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "glpat-0123456789abcdefghij",
            "xoxb-1234567890-abcdefghij",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA-0123456789abcdefghijklmnopqrstu",
            "sk_live_0123456789abcdefghij",
            "sk-ant-api03-0123456789abcdef",
        ] {
            assert_eq!(
                redact(&format!("with {token} here")),
                "with <redacted> here",
                "{token}"
            );
        }
    }

    #[test]
    fn a_private_key_on_one_line_goes_from_its_begin_marker() {
        let mut scrubber = Scrubber::default();
        assert_eq!(
            scrubber.line("key: -----BEGIN PRIVATE KEY-----MIIB-----END PRIVATE KEY-----"),
            "key: <redacted>"
        );
        assert_eq!(scrubber.line("next line"), "next line");
    }

    #[test]
    fn a_private_key_without_its_end_stops_at_the_first_line_that_is_not_key_material() {
        let mut scrubber = Scrubber::default();
        scrubber.line("-----BEGIN EC PRIVATE KEY-----");
        assert_eq!(scrubber.line("Proc-Type: 4,ENCRYPTED"), REDACTED);
        assert_eq!(scrubber.line(""), REDACTED);
        assert_eq!(
            scrubber.line("MHcCAQEEIBkg4LVWM9nuwNSk3yByxZpYRTBnVJk5oCUfaJ8"),
            REDACTED
        );
        assert_eq!(
            scrubber.line("scan finished in 3 s"),
            "scan finished in 3 s"
        );
    }

    #[test]
    fn a_public_key_or_certificate_is_not_redacted() {
        let mut scrubber = Scrubber::default();
        assert_eq!(
            scrubber.line("-----BEGIN CERTIFICATE-----"),
            "-----BEGIN CERTIFICATE-----"
        );
        assert_eq!(
            scrubber.line("MIIDdzCCAl+gAwIBAgIE"),
            "MIIDdzCCAl+gAwIBAgIE"
        );
    }
}
