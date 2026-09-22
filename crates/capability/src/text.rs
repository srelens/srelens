//! Text that came from somewhere else and is about to be shown to a person.
//!
//! One copy of the rule, in the lowest crate, because the rule has three
//! callers that must not drift apart: an extension manifest's validation
//! problems (`srelens-plugin-host`), the public catalog's licence check
//! (`srelens-registry`), and the sentence a confirmation dialog asks
//! (`super::annotations`). The table below started as one of those and the
//! others reached for it; keeping it here is what stops a fourth caller from
//! writing a fifth version of the ranges.

use std::fmt::Write as _;

/// Unicode's format characters, general category Cf, as of Unicode 17.0: the soft hyphen,
/// bidirectional marks, embeddings, overrides and isolates, zero-width spaces and joiners,
/// invisible operators, the byte order mark, tags, and a few script-specific marks. Each
/// changes how text displays without being seen itself. `char::is_control` covers only
/// category Cc. Listed here rather than taken from a Unicode crate; the ranges are those
/// of Unicode's `DerivedGeneralCategory.txt`.
const FORMAT_CHARACTERS: &[(char, char)] = &[
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

/// Whether `c` is a Unicode format character (category Cf), such as a right-to-left
/// override or a zero-width space. Text shown as an app's identity refuses them, because
/// they can make it display differently from what it holds.
pub fn is_format_character(c: char) -> bool {
    FORMAT_CHARACTERS
        .iter()
        .any(|&(first, last)| (first..=last).contains(&c))
}

/// Append `c` to `out`, written as a `\u{…}` escape if it is a control or
/// format character, and return how many characters were appended.
///
/// The count is the point. One character in can be eight or ten out, so a
/// caller that has to bound the *escaped* length would otherwise have to build
/// the whole escaped string before it could measure it — which is the work a
/// caller-controlled value of a few megabytes makes expensive. Escaping one
/// character at a time lets the caller stop as soon as it has enough.
pub fn push_escaped(out: &mut String, c: char) -> usize {
    if c.is_control() || is_format_character(c) {
        let before = out.len();
        // Infallible: `String`'s `fmt::Write` never errors.
        let _ = write!(out, "\\u{{{:x}}}", c as u32);
        // The escape is ASCII, so its byte length is its character length.
        out.len() - before
    } else {
        out.push(c);
        1
    }
}

/// `text` with control and format characters written as `\u{…}` escapes.
///
/// For text that is *shown* rather than *identifying*: a problem message, a
/// value quoted back, the sentence a confirmation asks. Escaping keeps the
/// value visible and readable while stripping it of the power to reorder or
/// hide the host's own words around it. Identity — a manifest's name, a
/// capability id — refuses these characters outright instead.
pub fn escape_invisible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        push_escaped(&mut out, c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_a_format_character_at_each_end_of_a_range() {
        for c in [
            '\u{00AD}',
            '\u{200B}',
            '\u{200F}',
            '\u{202E}',
            '\u{FEFF}',
            '\u{E007F}',
        ] {
            assert!(is_format_character(c), "missed U+{:04X}", c as u32);
        }
        for c in ['a', ' ', '\u{0}', '\u{2010}', '\u{FFFC}'] {
            assert!(!is_format_character(c), "claimed U+{:04X}", c as u32);
        }
    }

    #[test]
    fn escapes_control_and_format_characters_and_nothing_else() {
        assert_eq!(escape_invisible("api"), "api");
        assert_eq!(escape_invisible("a\u{202E}b"), "a\\u{202e}b");
        assert_eq!(escape_invisible("a\nb"), "a\\u{a}b");
        assert_eq!(escape_invisible("naïve — dash"), "naïve — dash");
    }
}
