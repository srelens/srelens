//! Dashboard card contributions (#540): the manifest shape and its rules.
//!
//! A card names a granted custom-resource reader, what to count among the
//! objects it lists, and which app page shows those objects. The host reads,
//! counts and draws; the app supplies data only, never markup or styling.
use super::{identifier, label, unique, Manifest};
use crate::{ValidationCode as Code, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{check_path, CardPredicate};

/// Most dashboard cards one manifest may declare.
pub const MAX_DASHBOARD_CARDS: usize = 16;

/// Most rows a `list` card may show.
pub const MAX_CARD_LIST_ROWS: usize = 10;

/// A card on the cluster dashboard: one figure about the objects one granted
/// custom-resource reader lists, drawn by the host.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DashboardCard {
    pub id: String,
    /// Untrusted display text, drawn as plain text.
    pub title: String,
    pub size: CardSize,
    #[serde(rename = "type")]
    pub card_type: CardType,
    /// The name of a `k8s.listCustomResource` binding in `capabilities`.
    pub source: String,
    /// Which of the source's objects the card counts; every one when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<CardPredicate>,
    /// The app page the card opens, with the predicate applied as its filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<CardTarget>,
    /// What a `metric` card reads; required there and refused elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<CardMetric>,
    /// How a `list` card orders and bounds its rows; refused on other types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<CardList>,
}

/// How much of the dashboard's width a card takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CardSize {
    S,
    M,
    L,
}

impl CardSize {
    /// How many rows a `list` card of this size shows when it names no limit.
    pub fn default_rows(self) -> usize {
        match self {
            CardSize::S => 3,
            CardSize::M => 5,
            CardSize::L => MAX_CARD_LIST_ROWS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CardType {
    /// How many objects the predicate holds for.
    Count,
    /// Those objects counted by the status the app's status resolver gives
    /// them (#541).
    CountByStatus,
    /// One number reduced from a value of those objects.
    Metric,
    /// The first few of those objects.
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardTarget {
    /// The `id` of a page in `contributions.pages` that lists the card's source.
    pub page: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardMetric {
    /// A number on each object, in the predicates' bounded path grammar.
    #[serde(rename = "jsonPath")]
    pub json_path: String,
    pub aggregate: CardAggregate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CardAggregate {
    Sum,
    Min,
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardList {
    /// A value shown beside each row, and what rows are ordered by; by
    /// namespace and name when absent.
    #[serde(default, rename = "jsonPath", skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<CardOrder>,
    /// How many rows to show, 1–10; by the card's size when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CardOrder {
    Asc,
    Desc,
}

/// Every rule a manifest's cards break, at the field that has to change.
pub(super) fn card_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    const LABEL: &str =
        "Must be 1–120 characters with no control characters and no bidirectional or invisible format characters";
    let cards = &manifest.contributions.dashboard_cards;
    if cards.len() > MAX_DASHBOARD_CARDS {
        problems.push(
            Code::InvalidValue,
            "contributions.dashboardCards",
            format!("Declare at most {MAX_DASHBOARD_CARDS} dashboard cards"),
        );
    }
    unique(
        problems,
        cards.iter().enumerate().map(|(index, card)| {
            (
                format!("contributions.dashboardCards[{index}].id"),
                card.id.as_str(),
            )
        }),
    );
    for (index, card) in cards.iter().enumerate() {
        let at = format!("contributions.dashboardCards[{index}]");
        if !identifier(&card.id) {
            problems.push(
                Code::InvalidValue,
                format!("{at}.id"),
                "Must be 1–64 letters, digits and -",
            );
        }
        if !label(&card.title) {
            problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
        }
        if !manifest.capabilities.iter().any(|binding| {
            binding.name == card.source && binding.target == "k8s.listCustomResource"
        }) {
            problems.push(
                Code::UnresolvedCapability,
                format!("{at}.source"),
                format!(
                    "\"{}\" is not a declared k8s.listCustomResource reader",
                    card.source
                ),
            );
        }
        // A status card counts by the app's rules for its source's kind (#541).
        // Whether there are any is known from the manifest alone, so a card that
        // could never show a figure is refused here rather than drawn blank.
        // A source that is not a custom-resource reader is reported at `source`.
        let reader = manifest.capabilities.iter().find(|binding| {
            binding.name == card.source && binding.target == "k8s.listCustomResource"
        });
        if card.card_type == CardType::CountByStatus
            && reader.is_some()
            && manifest.status_rules_for_binding(&card.source).is_none()
        {
            let kind = reader.and_then(Manifest::reader_kind);
            problems.push(
                Code::InvalidBinding,
                format!("{at}.type"),
                match kind {
                    Some(kind) => format!(
                        "countByStatus counts by the status rules for {kind}; declare them in statusResolvers, or use count"
                    ),
                    None => format!(
                        "countByStatus needs \"{}\" to fix its group and kind, so statusResolvers can name it",
                        card.source
                    ),
                },
            );
        }
        if let Some(Err(why)) = card.predicate.as_ref().map(CardPredicate::check) {
            problems.push(Code::InvalidBinding, format!("{at}.predicate"), why);
        }
        if let Some(target) = &card.target {
            let path = format!("{at}.target.page");
            match manifest.contributions.pages.iter().find(|page| page.id == target.page) {
                None => problems.push(
                    Code::UnresolvedPage,
                    path,
                    format!("Page \"{}\" is not declared", target.page),
                ),
                // The target shows the card's objects filtered by its predicate, so it
                // must list the same source; a dashboard page lists no objects at all.
                Some(page) if page.dashboard.is_some() => problems.push(
                    Code::InvalidBinding,
                    path,
                    format!("Page \"{}\" is a dashboard, so it cannot show the objects this card counts", target.page),
                ),
                Some(page) if page.capability != card.source => problems.push(
                    Code::InvalidBinding,
                    path,
                    format!(
                        "Page \"{}\" lists \"{}\", not this card's source \"{}\"",
                        target.page, page.capability, card.source
                    ),
                ),
                Some(_) => {}
            }
        }
        match (card.card_type, &card.metric) {
            (CardType::Metric, None) => problems.push(
                Code::InvalidBinding,
                format!("{at}.metric"),
                "A metric card declares the value it reads in `metric`",
            ),
            (CardType::Metric, Some(metric)) => {
                if let Err(why) = check_path(&metric.json_path) {
                    problems.push(Code::InvalidValue, format!("{at}.metric.jsonPath"), why);
                }
            }
            (_, Some(_)) => problems.push(
                Code::InvalidBinding,
                format!("{at}.metric"),
                "Only a metric card reads a `metric`",
            ),
            (_, None) => {}
        }
        match (card.card_type, &card.list) {
            (CardType::List, Some(list)) => {
                if list
                    .limit
                    .is_some_and(|limit| limit == 0 || limit > MAX_CARD_LIST_ROWS)
                {
                    problems.push(
                        Code::InvalidValue,
                        format!("{at}.list.limit"),
                        format!("A list card shows 1–{MAX_CARD_LIST_ROWS} rows"),
                    );
                }
                if let Some(Err(why)) = list.json_path.as_deref().map(check_path) {
                    problems.push(Code::InvalidValue, format!("{at}.list.jsonPath"), why);
                }
            }
            (CardType::List, None) | (_, None) => {}
            (_, Some(_)) => problems.push(
                Code::InvalidBinding,
                format!("{at}.list"),
                "Only a list card declares `list`",
            ),
        }
    }
}
