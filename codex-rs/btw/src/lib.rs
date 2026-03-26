use serde::Deserialize;
use serde::Serialize;

const DEFAULT_CONTEXT_BUDGET_TOKENS: usize = 2_000;
const PREVIEW_CHAR_LIMIT: usize = 1_200;
const SUMMARY_MAX_LINES: usize = 4;
const SUMMARY_MAX_CHARS: usize = 500;

pub const BTW_DEVELOPER_INSTRUCTIONS: &str = concat!(
    "This is a hidden `/btw` discussion thread. ",
    "Treat it as a temporary scratchpad that must not mutate the workspace or persistent state. ",
    "Do not write files, apply patches, spawn agents, or perform side-effectful actions. ",
    "If you need to inspect local context, keep it read-only and concise. ",
    "Your answer will be shown to the user in a temporary confirmation view and may be inserted ",
    "back into the main composer."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BtwSurface {
    SlashOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BtwVisibility {
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BtwPersistence {
    EphemeralOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BtwToolPolicy {
    InheritMinusSideEffects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BtwResultAction {
    InsertSummary,
    InsertFull,
    Discard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtwPolicy {
    pub context_budget_tokens: usize,
    pub tool_policy: BtwToolPolicy,
    pub result_actions: Vec<BtwResultAction>,
}

impl Default for BtwPolicy {
    fn default() -> Self {
        Self {
            context_budget_tokens: DEFAULT_CONTEXT_BUDGET_TOKENS,
            tool_policy: BtwToolPolicy::InheritMinusSideEffects,
            result_actions: vec![
                BtwResultAction::InsertSummary,
                BtwResultAction::InsertFull,
                BtwResultAction::Discard,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtwCapability {
    pub enabled: bool,
    pub surface: BtwSurface,
    pub visibility: BtwVisibility,
    pub persistence: BtwPersistence,
    pub policy: BtwPolicy,
}

impl Default for BtwCapability {
    fn default() -> Self {
        Self {
            enabled: true,
            surface: BtwSurface::SlashOnly,
            visibility: BtwVisibility::Hidden,
            persistence: BtwPersistence::EphemeralOnly,
            policy: BtwPolicy::default(),
        }
    }
}

pub fn merge_developer_instructions(existing: Option<String>) -> String {
    match existing {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{BTW_DEVELOPER_INSTRUCTIONS}")
        }
        _ => BTW_DEVELOPER_INSTRUCTIONS.to_string(),
    }
}

pub fn preview_text(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.chars().count() <= PREVIEW_CHAR_LIMIT {
        return trimmed.to_string();
    }

    let preview: String = trimmed.chars().take(PREVIEW_CHAR_LIMIT).collect();
    format!("{preview}\n\n…preview truncated…")
}

pub fn summarize_message(message: &str) -> String {
    let mut kept = Vec::new();
    let mut used_chars = 0usize;
    for line in message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let next_len = used_chars.saturating_add(line.chars().count());
        if !kept.is_empty() && (kept.len() >= SUMMARY_MAX_LINES || next_len > SUMMARY_MAX_CHARS) {
            break;
        }
        kept.push(line.to_string());
        used_chars = next_len;
    }

    if kept.is_empty() {
        "BTW summary:\n(Empty answer)".to_string()
    } else {
        format!("BTW summary:\n{}", kept.join("\n"))
    }
}

pub fn full_insert_text(message: &str) -> String {
    format!("BTW discussion:\n{message}")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::BtwCapability;
    use super::BtwResultAction;
    use super::BtwToolPolicy;
    use super::preview_text;
    use super::summarize_message;

    #[test]
    fn btw_capability_defaults_match_hidden_ephemeral_discussion_flow() {
        let capability = BtwCapability::default();

        assert!(capability.enabled);
        assert_eq!(capability.policy.context_budget_tokens, 2_000);
        assert_eq!(
            capability.policy.tool_policy,
            BtwToolPolicy::InheritMinusSideEffects
        );
        assert_eq!(
            capability.policy.result_actions,
            vec![
                BtwResultAction::InsertSummary,
                BtwResultAction::InsertFull,
                BtwResultAction::Discard,
            ]
        );
    }

    #[test]
    fn summarize_message_keeps_short_prefix_for_insertion() {
        let summary = summarize_message(
            "First point.\n\nSecond point.\nThird point.\nFourth point.\nFifth point.",
        );

        assert_eq!(
            summary,
            "BTW summary:\nFirst point.\nSecond point.\nThird point.\nFourth point."
        );
    }

    #[test]
    fn preview_text_truncates_long_messages() {
        let message = "a".repeat(1_250);
        let preview = preview_text(&message);

        assert!(preview.contains("preview truncated"));
    }
}
