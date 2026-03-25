use serde::Deserialize;
use serde::Serialize;

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
            context_budget_tokens: 2_000,
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

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::BtwCapability;
    use super::BtwResultAction;
    use super::BtwToolPolicy;

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
}
