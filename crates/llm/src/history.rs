//! In-memory conversation history distillation and safe trimming.
//!
//! In multi-turn assistant conversations, past rounds accumulate raw, verbose tool results
//! (e.g. 50 KB JSON responses from `listPods` or large logs). Re-sending these full dumps
//! on subsequent turns quickly exhausts the LLM's context window and inflates token cost.
//!
//! `distill_and_trim_history` summarizes completed tool outcomes from previous turns,
//! keeps recent context, caps history to `max_turns`, and guarantees the conversation
//! strictly begins on a `Turn::User`.

use crate::types::Turn;

/// Distills intermediate tool results into a compact index and trims conversation
/// history so that it never exceeds `max_turns` and ALWAYS starts on a `Turn::User`.
pub fn distill_and_trim_history(turns: Vec<Turn>, max_turns: usize) -> Vec<Turn> {
    let mut distilled = Vec::with_capacity(turns.len());

    for turn in turns {
        match turn {
            Turn::ToolResults(outcomes) => {
                let summary_outcomes = outcomes
                    .into_iter()
                    .map(|mut o| {
                        if o.content.len() > 300 {
                            let prefix: String = o.content.chars().take(120).collect();
                            o.content = format!(
                                "{}... [Detailed output of tool `{}` ({} bytes) cleared between turns]",
                                prefix.trim_end(),
                                o.name,
                                o.content.len()
                            );
                        }
                        o
                    })
                    .collect();
                distilled.push(Turn::ToolResults(summary_outcomes));
            }
            other => distilled.push(other),
        }
    }

    // Ensure history does not exceed max_turns and strictly begins on Turn::User
    while distilled.len() > max_turns {
        distilled.remove(0);
        while distilled
            .first()
            .is_some_and(|t| !matches!(t, Turn::User(_)))
        {
            distilled.remove(0);
        }
    }

    // If history still starts with a non-user turn, clean it up
    while distilled
        .first()
        .is_some_and(|t| !matches!(t, Turn::User(_)))
    {
        distilled.remove(0);
    }

    distilled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolOutcome;

    #[test]
    fn test_distill_and_trim_history_clears_heavy_tool_results_between_turns() {
        let heavy_output = "x".repeat(10_000);
        let turns = vec![
            Turn::User("Check pods".into()),
            Turn::ToolResults(vec![ToolOutcome {
                id: "call-1".into(),
                name: "k8s_listPods".into(),
                content: heavy_output,
                is_error: false,
            }]),
            Turn::Assistant {
                text: "Pods are running".into(),
                tool_calls: vec![],
            },
        ];

        let distilled = distill_and_trim_history(turns, 10);
        assert_eq!(distilled.len(), 3);
        match &distilled[1] {
            Turn::ToolResults(outcomes) => {
                assert!(outcomes[0].content.contains("k8s_listPods"));
                assert!(outcomes[0].content.contains("cleared between turns"));
                assert!(outcomes[0].content.len() < 500);
            }
            _ => panic!("Expected Turn::ToolResults"),
        }
    }

    #[test]
    fn test_distill_and_trim_history_preserves_short_content() {
        let short_output = "{\"status\": \"ok\"}";
        let turns = vec![
            Turn::User("Restart pod".into()),
            Turn::ToolResults(vec![ToolOutcome {
                id: "call-1".into(),
                name: "k8s_deletePod".into(),
                content: short_output.into(),
                is_error: false,
            }]),
        ];

        let distilled = distill_and_trim_history(turns, 10);
        match &distilled[1] {
            Turn::ToolResults(outcomes) => {
                assert_eq!(outcomes[0].content, short_output);
            }
            _ => panic!("Expected Turn::ToolResults"),
        }
    }

    #[test]
    fn test_distill_and_trim_history_always_begins_on_user_turn() {
        let turns = vec![
            Turn::Assistant {
                text: "Old reply".into(),
                tool_calls: vec![],
            },
            Turn::User("First real user turn".into()),
            Turn::Assistant {
                text: "Second reply".into(),
                tool_calls: vec![],
            },
        ];

        let trimmed = distill_and_trim_history(turns, 10);
        assert_eq!(trimmed.len(), 2);
        assert!(matches!(&trimmed[0], Turn::User(u) if u == "First real user turn"));
    }

    #[test]
    fn test_distill_and_trim_history_respects_max_turns() {
        let mut turns = Vec::new();
        for i in 0..15 {
            turns.push(Turn::User(format!("User {i}")));
            turns.push(Turn::Assistant {
                text: format!("Reply {i}"),
                tool_calls: vec![],
            });
        }

        let trimmed = distill_and_trim_history(turns, 6);
        assert!(trimmed.len() <= 6);
        assert!(matches!(&trimmed[0], Turn::User(_)));
    }
}
