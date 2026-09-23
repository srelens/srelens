//! What a dashboard card's predicate (#540) means: the action predicates'
//! path grammar and `equals` / `absent`, plus two date operators that read
//! the value as a timestamp relative to a clock the caller supplies.

use serde_json::{json, Value};
use srelens_capability::CardPredicate;

fn predicate(value: Value) -> CardPredicate {
    serde_json::from_value(value).expect("a card predicate deserializes")
}

/// 2026-09-23T00:00:00Z, the clock every case below reads.
const NOW: i64 = 1_790_121_600;
const DAY: i64 = 86_400;

/// An RFC 3339 timestamp `offset` seconds from [`NOW`].
fn at(offset: i64) -> String {
    let seconds = NOW + offset;
    let days = seconds.div_euclid(DAY);
    let rest = seconds.rem_euclid(DAY);
    // Civil date from days since the epoch (Howard Hinnant's algorithm), so
    // the test builds its timestamps without the parser it is testing.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn certificate(not_after: &str) -> Value {
    json!({"metadata": {"name": "web"}, "status": {"notAfter": not_after}})
}

#[test]
fn the_test_clock_is_the_one_its_timestamps_say() {
    assert_eq!(at(0), "2026-09-23T00:00:00Z");
    assert_eq!(at(-DAY), "2026-09-22T00:00:00Z");
}

#[test]
fn within_a_positive_window_holds_from_now_until_that_far_ahead() {
    let soon = predicate(json!({"jsonPath": ".status.notAfter", "within": "14d"}));
    assert!(soon.check().is_ok());
    assert!(soon.holds_at(&certificate(&at(3 * DAY)), NOW));
    assert!(
        soon.holds_at(&certificate(&at(14 * DAY)), NOW),
        "the edge is inside"
    );
    assert!(!soon.holds_at(&certificate(&at(15 * DAY)), NOW));
    assert!(
        !soon.holds_at(&certificate(&at(-DAY)), NOW),
        "already past is not within a window ahead; that is `before`"
    );
}

#[test]
fn within_a_negative_window_holds_for_the_recent_past() {
    let recent = predicate(json!({"jsonPath": ".status.lastFailure", "within": "-1h"}));
    let failed = |offset: i64| json!({"status": {"lastFailure": at(offset)}});
    assert!(recent.holds_at(&failed(-30 * 60), NOW));
    assert!(!recent.holds_at(&failed(-2 * 3_600), NOW));
    assert!(
        !recent.holds_at(&failed(60), NOW),
        "the future is not the recent past"
    );
}

#[test]
fn before_holds_for_anything_earlier_than_the_offset_including_the_past() {
    let expiring = predicate(json!({"jsonPath": ".status.notAfter", "before": "14d"}));
    assert!(expiring.holds_at(&certificate(&at(-30 * DAY)), NOW));
    assert!(expiring.holds_at(&certificate(&at(13 * DAY)), NOW));
    assert!(
        !expiring.holds_at(&certificate(&at(14 * DAY)), NOW),
        "strictly before"
    );
    let expired = predicate(json!({"jsonPath": ".status.notAfter", "before": "0d"}));
    assert!(expired.holds_at(&certificate(&at(-1)), NOW));
    assert!(!expired.holds_at(&certificate(&at(1)), NOW));
}

#[test]
fn a_date_operator_fails_closed_on_a_value_that_is_not_a_timestamp() {
    let soon = predicate(json!({"jsonPath": ".status.notAfter", "within": "14d"}));
    let expiring = predicate(json!({"jsonPath": ".status.notAfter", "before": "14d"}));
    for value in [
        json!({"status": {}}),
        json!({"status": {"notAfter": null}}),
        json!({"status": {"notAfter": "next tuesday"}}),
        json!({"status": {"notAfter": 1_790_121_600}}),
        json!({"status": {"notAfter": "2026-09-24"}}),
    ] {
        assert!(!soon.holds_at(&value, NOW), "{value}");
        assert!(!expiring.holds_at(&value, NOW), "{value}");
    }
    // An offset timestamp is the same instant as its UTC spelling.
    assert!(soon.holds_at(&certificate("2026-09-24T02:00:00+02:00"), NOW));
}

#[test]
fn equals_and_absent_mean_what_they_mean_for_actions() {
    let failing = predicate(json!({"jsonPath": ".status.conditions[0].status", "equals": "False"}));
    let unset = predicate(json!({"jsonPath": ".spec.suspend", "absent": true}));
    let object =
        json!({"spec": {"suspend": null}, "status": {"conditions": [{"status": "False"}]}});
    assert!(failing.holds_at(&object, NOW));
    assert!(unset.holds_at(&object, NOW));
    assert!(!failing.holds_at(&json!({"status": {"conditions": []}}), NOW));
    assert!(!unset.holds_at(&json!({"spec": {"suspend": false}}), NOW));
}

#[test]
fn exactly_one_operator_is_declared() {
    for (value, why) in [
        (json!({"jsonPath": ".a"}), "no operator"),
        (
            json!({"jsonPath": ".a", "equals": 1, "absent": true}),
            "two operators",
        ),
        (
            json!({"jsonPath": ".a", "within": "1d", "before": "1d"}),
            "two dates",
        ),
        (
            json!({"jsonPath": ".a", "absent": false}),
            "absent is written true",
        ),
        (
            json!({"jsonPath": ".a", "equals": {"x": 1}}),
            "a literal comparand",
        ),
    ] {
        let declared = predicate(value);
        assert!(declared.check().is_err(), "{why}");
        assert!(
            !declared.holds_at(&json!({"a": 1}), NOW),
            "{why} must not hold"
        );
    }
}

#[test]
fn a_duration_is_a_signed_count_of_one_unit_and_bounded() {
    for good in ["30s", "15m", "1h", "14d", "2w", "-1h", "0d", "3650d"] {
        let declared = predicate(json!({"jsonPath": ".a", "before": good}));
        assert!(declared.check().is_ok(), "{good}");
    }
    for bad in [
        "14",
        "d",
        "1.5d",
        "1d2h",
        "+1d",
        "1y",
        "P14D",
        "3651d",
        "",
        " 1d",
        "99999999d",
    ] {
        let declared = predicate(json!({"jsonPath": ".a", "before": bad}));
        let error = declared.check().expect_err(bad);
        assert!(error.contains("duration"), "{bad}: {error}");
    }
    let empty = predicate(json!({"jsonPath": ".a", "within": "0d"}));
    assert!(
        empty.check().unwrap_err().contains("window"),
        "a window of no length matches nothing"
    );
}

#[test]
fn a_path_this_host_cannot_evaluate_is_refused_and_never_holds() {
    for path in ["status.notAfter", ".status[*].x", ".status..x", ".a b"] {
        let declared = predicate(json!({"jsonPath": path, "absent": true}));
        assert!(declared.check().is_err(), "{path}");
        assert!(
            !declared.holds_at(&json!({}), NOW),
            "{path}: a typo must not read as a match, even for `absent`"
        );
    }
}

#[test]
fn unknown_fields_are_refused() {
    assert!(serde_json::from_value::<CardPredicate>(
        json!({"jsonPath": ".a", "equals": 1, "reason": "cards carry no reason"})
    )
    .is_err());
    assert!(
        serde_json::from_value::<CardPredicate>(json!({"jsonPath": ".a", "notEquals": 1})).is_err()
    );
}
