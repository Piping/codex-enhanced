use crate::account_pool::AccountPoolState;
use crate::account_pool::AccountRecord;
use serde::Deserialize;
use serde::Serialize;

const DEFAULT_USAGE_THRESHOLD_PERMILLE: u16 = 850;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingTrigger {
    NormalTurn,
    RetryAfterHardError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRouterDecisionReason {
    KeepActiveAccount,
    RetryWithFallbackAccount,
    ActiveAccountCoolingDown,
    ActiveAccountOverThreshold,
    PreferredFallbackSelected,
    NoHealthyAccount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteTurnRequest {
    pub now_ts: i64,
    pub trigger: RoutingTrigger,
    pub active_account_id: Option<String>,
    pub preferred_account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRouterDecision {
    pub account_id: Option<String>,
    pub reason: AccountRouterDecisionReason,
    pub retry_immediately: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultAccountRouter {
    usage_threshold_permille: u16,
}

impl Default for DefaultAccountRouter {
    fn default() -> Self {
        Self {
            usage_threshold_permille: DEFAULT_USAGE_THRESHOLD_PERMILLE,
        }
    }
}

impl DefaultAccountRouter {
    pub fn new(usage_threshold_permille: u16) -> Self {
        Self {
            usage_threshold_permille,
        }
    }

    pub fn select_account(
        &self,
        state: &AccountPoolState,
        request: &RouteTurnRequest,
    ) -> AccountRouterDecision {
        let active = request
            .active_account_id
            .as_deref()
            .or(state.active_account_id.as_deref())
            .and_then(|account_id| {
                state
                    .accounts
                    .iter()
                    .find(|account| account.id == account_id)
            });

        if request.trigger == RoutingTrigger::NormalTurn
            && let Some(active) = active
            && self.is_healthy(active, request.now_ts)
        {
            return AccountRouterDecision {
                account_id: Some(active.id.clone()),
                reason: AccountRouterDecisionReason::KeepActiveAccount,
                retry_immediately: false,
            };
        }

        if request.trigger == RoutingTrigger::RetryAfterHardError {
            if let Some(fallback) = self.pick_fallback(state, request, active) {
                return AccountRouterDecision {
                    account_id: Some(fallback.id.clone()),
                    reason: AccountRouterDecisionReason::RetryWithFallbackAccount,
                    retry_immediately: true,
                };
            }

            return AccountRouterDecision {
                account_id: None,
                reason: AccountRouterDecisionReason::NoHealthyAccount,
                retry_immediately: false,
            };
        }

        if let Some(active) = active {
            let reason = if active.is_available_at(request.now_ts) {
                AccountRouterDecisionReason::ActiveAccountOverThreshold
            } else {
                AccountRouterDecisionReason::ActiveAccountCoolingDown
            };

            if let Some(fallback) = self.pick_fallback(state, request, Some(active)) {
                return AccountRouterDecision {
                    account_id: Some(fallback.id.clone()),
                    reason,
                    retry_immediately: false,
                };
            }
        }

        if let Some(preferred) = request
            .preferred_account_id
            .as_deref()
            .and_then(|account_id| {
                state
                    .accounts
                    .iter()
                    .find(|account| account.id == account_id)
            })
            .filter(|account| self.is_healthy(account, request.now_ts))
        {
            return AccountRouterDecision {
                account_id: Some(preferred.id.clone()),
                reason: AccountRouterDecisionReason::PreferredFallbackSelected,
                retry_immediately: false,
            };
        }

        AccountRouterDecision {
            account_id: None,
            reason: AccountRouterDecisionReason::NoHealthyAccount,
            retry_immediately: false,
        }
    }

    fn is_healthy(&self, account: &AccountRecord, now_ts: i64) -> bool {
        account.is_available_at(now_ts)
            && account
                .highest_pressure_permille()
                .is_none_or(|pressure| pressure < self.usage_threshold_permille)
    }

    fn pick_fallback<'a>(
        &self,
        state: &'a AccountPoolState,
        request: &RouteTurnRequest,
        active: Option<&AccountRecord>,
    ) -> Option<&'a AccountRecord> {
        let active_id = active.map(|account| account.id.as_str());
        let mut candidates: Vec<&AccountRecord> = state
            .accounts
            .iter()
            .filter(|account| Some(account.id.as_str()) != active_id)
            .filter(|account| self.is_healthy(account, request.now_ts))
            .collect();

        candidates.sort_by_key(|account| (account.priority, account.last_selected_at.unwrap_or(0)));
        candidates.into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use crate::account_pool::AccountPoolState;
    use crate::account_pool::AccountRecord;
    use crate::account_pool::AccountUsageWindow;
    use crate::account_pool::AccountUsageWindowKind;
    use crate::account_pool::UsageEstimateSource;

    use super::AccountRouterDecision;
    use super::AccountRouterDecisionReason;
    use super::DefaultAccountRouter;
    use super::RouteTurnRequest;
    use super::RoutingTrigger;

    fn account(
        id: &str,
        priority: u32,
        used: u32,
        limit: u32,
        cooldown_until: Option<i64>,
    ) -> AccountRecord {
        AccountRecord {
            id: id.to_string(),
            alias: id.to_string(),
            masked_email: None,
            plan_label: None,
            priority,
            enabled: true,
            cooldown_until,
            last_limit_error_at: None,
            last_selected_at: None,
            usage_windows: vec![AccountUsageWindow {
                kind: AccountUsageWindowKind::FiveHour,
                label: "5h".to_string(),
                estimated_used_units: used,
                estimated_limit_units: Some(limit),
                reset_at: None,
                source: UsageEstimateSource::LocalHeuristic,
            }],
        }
    }

    #[test]
    fn normal_turn_keeps_healthy_active_account() {
        let router = DefaultAccountRouter::default();
        let state = AccountPoolState {
            version: 1,
            active_account_id: Some("primary".to_string()),
            accounts: vec![
                account("primary", 0, 10, 40, None),
                account("backup", 1, 1, 40, None),
            ],
        };
        let request = RouteTurnRequest {
            now_ts: 100,
            trigger: RoutingTrigger::NormalTurn,
            active_account_id: Some("primary".to_string()),
            preferred_account_id: None,
        };

        assert_eq!(
            router.select_account(&state, &request),
            AccountRouterDecision {
                account_id: Some("primary".to_string()),
                reason: AccountRouterDecisionReason::KeepActiveAccount,
                retry_immediately: false,
            }
        );
    }
}
