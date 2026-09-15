//! Test support for the cargo-fuzz targets in `fuzz/` and the property tests below. An
//! entry point takes arbitrary bytes and panics only when a parser breaks its contract, so
//! the fuzzer and `cargo test` hold the same properties. Compiled for this crate's tests and
//! under the `fuzzing` feature; not an API.
use crate::{Manifest, ValidationCode, ValidationErrors, MAX_MANIFEST_BYTES};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// `Manifest::decode`, `validate` and `parse` on arbitrary input. None of them panics, each
/// yields a value or at least one reported problem, `parse` agrees with `decode` followed by
/// `validate`, nothing over [`MAX_MANIFEST_BYTES`] decodes, and an accepted manifest
/// re-serializes to one that is accepted and equal.
pub fn manifest(data: &[u8]) {
    // The host refuses a manifest that is not UTF-8 before it parses anything.
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let parsed = Manifest::parse(source);
    match Manifest::decode(source) {
        Err(problems) => {
            reported(&problems);
            if source.len() > MAX_MANIFEST_BYTES {
                assert_eq!(codes(&problems), [ValidationCode::TooLarge]);
            }
            assert_eq!(
                parsed.err(),
                Some(problems),
                "parse reports what decode found"
            );
        }
        Ok(decoded) => {
            assert!(
                source.len() <= MAX_MANIFEST_BYTES,
                "a {}-byte manifest decoded",
                source.len()
            );
            let validated = decoded.validate();
            if let Err(problems) = &validated {
                reported(problems);
            }
            match parsed {
                Ok(manifest) => {
                    assert!(validated.is_ok(), "parse accepted what validate refused");
                    round_trip(&manifest);
                }
                Err(problems) => assert_eq!(validated.err(), Some(problems)),
            }
        }
    }
}

/// A refusal names at least one problem, and every problem says what is wrong.
fn reported(problems: &ValidationErrors) {
    assert!(!problems.0.is_empty(), "a refusal reports no problem");
    for problem in &problems.0 {
        assert!(
            !problem.message.trim().is_empty(),
            "{problem:?} has no message"
        );
    }
    assert!(!problems.to_string().is_empty());
}

fn codes(problems: &ValidationErrors) -> Vec<ValidationCode> {
    problems.0.iter().map(|problem| problem.code).collect()
}

/// An accepted manifest written back out is accepted again, unchanged. The inventory stores
/// manifests this way, and reverifies a signed one by comparing the two.
fn round_trip(manifest: &Manifest) {
    let value = serde_json::to_value(manifest).expect("an accepted manifest serializes");
    let text = value.to_string();
    let again = if text.len() <= MAX_MANIFEST_BYTES {
        Manifest::parse(&text)
    } else {
        // Writing a number out can lengthen it (`1e5` becomes `100000.0`), so a manifest
        // near the limit may no longer fit. Only a source is held to it; a stored one is not.
        let again: Manifest =
            serde_json::from_str(&text).expect("a re-serialized manifest decodes");
        again.validate().map(|()| again)
    }
    .unwrap_or_else(|problems| panic!("a re-serialized manifest is refused:\n{problems}"));
    assert_eq!(
        serde_json::to_value(&again).expect("a manifest serializes"),
        value,
        "a manifest changed on a round trip"
    );
}

/// A document a few edits away from `seed`, each edit read from `choices`. Random bytes
/// almost never get past a JSON parser; edits to a valid document reach the rules behind the
/// schema. The same `choices` always make the same document, and no choices leave `seed`.
pub fn mutate(seed: &Value, choices: &[u8]) -> Value {
    let mut words = BTreeSet::new();
    collect_words(seed, &mut words);
    let words: Vec<String> = words.into_iter().collect();
    let mut choices = Choices(choices);
    let mut document = seed.clone();
    while let Some(edit) = choices.byte() {
        let mut paths = Vec::new();
        collect_paths(&document, &mut Vec::new(), &mut paths);
        let path = paths[choices.below(paths.len())].clone();
        let donor = node(&mut document, &paths[choices.below(paths.len())]).clone();
        match edit % 8 {
            // Another value from the document: a type change that still looks like the format.
            0 => *node(&mut document, &path) = donor,
            1 => remove(&mut document, &path),
            // Beside itself, which makes duplicates.
            2 => duplicate(&mut document, &path),
            3 => *node(&mut document, &path) = scalar(&mut choices),
            4 => *node(&mut document, &path) = Value::String(text(&words, &mut choices)),
            5 => {
                let target = node(&mut document, &path);
                let inner = target.take();
                *target = if choices.byte().unwrap_or(0) % 2 == 0 {
                    Value::Array(vec![inner])
                } else {
                    Value::Object(Map::from_iter([(text(&words, &mut choices), inner)]))
                };
            }
            6 => match node(&mut document, &path) {
                Value::Object(fields) => {
                    fields.insert(text(&words, &mut choices), donor);
                }
                Value::Array(items) => items.push(donor),
                other => *other = donor,
            },
            _ => tweak(node(&mut document, &path), &mut choices),
        }
    }
    document
}

/// Reads choices off the front of a byte string, as zeros once it runs out.
struct Choices<'a>(&'a [u8]);

impl Choices<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&first, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(first)
    }

    /// Below `len`, which is not zero.
    fn below(&mut self, len: usize) -> usize {
        let high = usize::from(self.byte().unwrap_or(0));
        let low = usize::from(self.byte().unwrap_or(0));
        ((high << 8) | low) % len
    }

    fn word(&mut self) -> u64 {
        (0..8).fold(0, |word, _| {
            (word << 8) | u64::from(self.byte().unwrap_or(0))
        })
    }
}

#[derive(Clone)]
enum Step {
    Key(String),
    Index(usize),
}

fn collect_paths(value: &Value, at: &mut Vec<Step>, into: &mut Vec<Vec<Step>>) {
    into.push(at.clone());
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                at.push(Step::Key(key.clone()));
                collect_paths(child, at, into);
                at.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                at.push(Step::Index(index));
                collect_paths(child, at, into);
                at.pop();
            }
        }
        _ => {}
    }
}

fn collect_words(value: &Value, into: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            into.insert(text.clone());
        }
        Value::Object(fields) => {
            for (key, child) in fields {
                into.insert(key.clone());
                collect_words(child, into);
            }
        }
        Value::Array(items) => items.iter().for_each(|child| collect_words(child, into)),
        _ => {}
    }
}

/// The value at a path [`collect_paths`] found in this same document.
fn node<'a>(document: &'a mut Value, path: &[Step]) -> &'a mut Value {
    path.iter()
        .fold(document, |value, step| match (value, step) {
            (Value::Object(fields), Step::Key(key)) => {
                fields.get_mut(key).expect("a collected key")
            }
            (Value::Array(items), Step::Index(index)) => {
                items.get_mut(*index).expect("a collected index")
            }
            _ => unreachable!("a collected path"),
        })
}

fn remove(document: &mut Value, path: &[Step]) {
    let Some((last, parent)) = path.split_last() else {
        *document = Value::Object(Map::new());
        return;
    };
    match (node(document, parent), last) {
        (Value::Object(fields), Step::Key(key)) => {
            fields.remove(key);
        }
        (Value::Array(items), Step::Index(index)) => {
            items.remove(*index);
        }
        _ => unreachable!("a collected path"),
    }
}

fn duplicate(document: &mut Value, path: &[Step]) {
    if let Some((Step::Index(index), parent)) = path.split_last() {
        if let Value::Array(items) = node(document, parent) {
            items.insert(*index, items[*index].clone());
            return;
        }
    }
    if let Value::Array(items) = node(document, path) {
        if let Some(first) = items.first().cloned() {
            items.push(first);
        }
    }
}

/// Numbers and literals at the edges of what a field accepts.
fn scalar(choices: &mut Choices) -> Value {
    match choices.byte().unwrap_or(0) % 12 {
        0 => Value::Null,
        1 => Value::Bool(true),
        2 => Value::Bool(false),
        3 => Value::from(0),
        4 => Value::from(-1),
        5 => Value::from(64),
        6 => Value::from(u64::MAX),
        7 => Value::from(i64::MIN),
        8 => Value::from(0.5),
        9 => Value::from(1e300),
        // Any double: the round trip must keep every number exactly.
        10 => Value::from(f64::from_bits(choices.word())),
        _ => Value::Array(Vec::new()),
    }
}

/// Strings that sit at the edge of an identifier, label, kind, URL or version rule.
const EXTREMES: &[&str] = &[
    "",
    " ",
    "\u{0}",
    "\t",
    "\u{7f}",
    "\u{202e}",
    "é",
    "/",
    ".",
    "-",
    "a/b",
    "a..b",
    "plugin/",
    "^99",
    "*",
    "0.0.0-a",
    "https://github.com/",
];

/// A string from the document, two of them joined, an extreme, or one of a boundary length.
fn text(words: &[String], choices: &mut Choices) -> String {
    let word = |choices: &mut Choices| match words.len() {
        0 => String::new(),
        len => words[choices.below(len)].clone(),
    };
    match choices.byte().unwrap_or(0) % 4 {
        0 => word(choices),
        1 => {
            let separator = [".", "/", "-", ""][usize::from(choices.byte().unwrap_or(0) % 4)];
            format!("{}{separator}{}", word(choices), word(choices))
        }
        2 => EXTREMES[choices.below(EXTREMES.len())].to_owned(),
        _ => {
            let lengths = [64, 65, 120, 121, 128, 129, 256];
            "a".repeat(lengths[choices.below(lengths.len())])
        }
    }
}

fn tweak(value: &mut Value, choices: &mut Choices) {
    let choice = choices.byte().unwrap_or(0);
    match value {
        Value::String(text) => match choice % 4 {
            0 => {
                text.pop();
            }
            1 => text.push(['-', '.', '/', 'A', '\u{0}'][usize::from(choice / 4 % 5)]),
            2 => *text = text.to_uppercase(),
            _ => text.insert(0, ' '),
        },
        Value::Number(number) => {
            *value = match number.as_i64() {
                Some(number) if choice % 2 == 0 => Value::from(number.wrapping_add(1)),
                Some(number) => Value::from(number.wrapping_neg()),
                None => Value::from(-number.as_f64().unwrap_or_default()),
            }
        }
        Value::Bool(flag) => *flag = !*flag,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*, test_runner::RngSeed};

    const EXAMPLES: [&str; 2] = [
        include_str!("../../../examples/extensions/argocd.json"),
        include_str!("../../../examples/extensions/flux.json"),
    ];

    /// The same cases on every run, so a red build points at a change rather than a lucky
    /// draw. `PROPTEST_RNG_SEED` explores others; the fuzz workflow keeps exploring.
    fn config() -> ProptestConfig {
        let mut config = ProptestConfig::default();
        if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
            config.rng_seed = RngSeed::Fixed(580);
        }
        config
    }

    fn edited(example: &str, choices: &[u8], pretty: bool) -> String {
        let document = mutate(&serde_json::from_str(example).unwrap(), choices);
        if pretty {
            serde_json::to_string_pretty(&document).unwrap()
        } else {
            document.to_string()
        }
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn arbitrary_bytes_are_decoded_or_reported(data in vec(any::<u8>(), 0..1024)) {
            manifest(&data);
        }

        #[test]
        fn manifests_a_few_edits_from_an_example_are_accepted_or_reported(
            example in 0..EXAMPLES.len(),
            choices in vec(any::<u8>(), 0..96),
            pretty in any::<bool>(),
        ) {
            manifest(edited(EXAMPLES[example], &choices, pretty).as_bytes());
        }
    }

    #[test]
    fn the_examples_are_accepted_and_survive_a_round_trip() {
        for example in EXAMPLES {
            assert!(Manifest::parse(example).is_ok());
            manifest(example.as_bytes());
        }
    }

    #[test]
    fn a_manifest_of_exactly_the_limit_decodes_and_one_byte_more_does_not() {
        let mut source = EXAMPLES[0].trim_end().to_owned();
        source.push_str(&" ".repeat(MAX_MANIFEST_BYTES - source.len()));
        assert!(Manifest::parse(&source).is_ok());
        manifest(source.as_bytes());
        source.push(' ');
        let problems = Manifest::decode(&source).unwrap_err();
        assert_eq!(codes(&problems), [ValidationCode::TooLarge]);
        manifest(source.as_bytes());
    }

    #[test]
    fn edits_are_repeatable_and_none_leave_the_seed() {
        let seed: Value = serde_json::from_str(EXAMPLES[1]).unwrap();
        assert_eq!(mutate(&seed, &[]), seed);
        let choices: Vec<u8> = (0..=u8::MAX).collect();
        assert_eq!(mutate(&seed, &choices), mutate(&seed, &choices));
        assert_ne!(mutate(&seed, &choices), seed);
    }

    #[test]
    fn edits_reach_the_rules_behind_the_schema() {
        let (mut refused_by_schema, mut refused_by_rules, mut accepted) = (0, 0, 0);
        for case in 0..512u32 {
            // One edit of each kind, at paths spread across the document.
            let spread = case.wrapping_mul(2_654_435_761).to_le_bytes();
            let choices = [case as u8 % 8, spread[3], spread[2], spread[1], spread[0]];
            let source = edited(EXAMPLES[1], &choices, false);
            match Manifest::decode(&source) {
                Err(_) => refused_by_schema += 1,
                Ok(decoded) if decoded.validate().is_err() => refused_by_rules += 1,
                Ok(_) => accepted += 1,
            }
        }
        assert!(
            refused_by_schema > 0 && refused_by_rules > 0 && accepted > 0,
            "schema {refused_by_schema}, rules {refused_by_rules}, accepted {accepted}"
        );
    }
}
