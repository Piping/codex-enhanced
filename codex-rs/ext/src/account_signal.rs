use serde::Deserialize;
use serde::Serialize;

const DEFAULT_GENERIC_LIMIT_COOLDOWN_SECS: i64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRateLimitWindow {
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRateLimitSnapshot {
    pub limit_name: Option<String>,
    pub primary: Option<AccountRateLimitWindow>,
    pub secondary: Option<AccountRateLimitWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitSignalKind {
    UsageLimit,
    RateLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLimitSignal {
    pub kind: LimitSignalKind,
    pub recorded_at: i64,
    pub cooldown_until: Option<i64>,
}

pub fn infer_limit_signal(
    kind: LimitSignalKind,
    recorded_at: i64,
    snapshot: Option<&AccountRateLimitSnapshot>,
) -> AccountLimitSignal {
    let cooldown_until = snapshot
        .and_then(|snapshot| blocking_resets_at(snapshot, recorded_at))
        .or_else(|| match kind {
            LimitSignalKind::UsageLimit => None,
            LimitSignalKind::RateLimit => Some(recorded_at + DEFAULT_GENERIC_LIMIT_COOLDOWN_SECS),
        });

    AccountLimitSignal {
        kind,
        recorded_at,
        cooldown_until,
    }
}

fn blocking_resets_at(snapshot: &AccountRateLimitSnapshot, recorded_at: i64) -> Option<i64> {
    let mut saturated = Vec::new();
    let mut any_future = Vec::new();

    for window in [snapshot.primary.as_ref(), snapshot.secondary.as_ref()]
        .into_iter()
        .flatten()
    {
        let Some(resets_at) = window
            .resets_at
            .filter(|resets_at| *resets_at > recorded_at)
        else {
            continue;
        };

        any_future.push(resets_at);
        if window.used_percent >= 99.5 {
            saturated.push(resets_at);
        }
    }

    saturated
        .into_iter()
        .min()
        .or_else(|| any_future.into_iter().min())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::AccountLimitSignal;
    use super::AccountRateLimitSnapshot;
    use super::AccountRateLimitWindow;
    use super::LimitSignalKind;
    use super::infer_limit_signal;

    #[test]
    fn usage_limit_prefers_saturated_window_reset() {
        let signal = infer_limit_signal(
            LimitSignalKind::UsageLimit,
            100,
            Some(&AccountRateLimitSnapshot {
                limit_name: Some("codex".to_string()),
                primary: Some(AccountRateLimitWindow {
                    used_percent: 92.0,
                    window_minutes: Some(300),
                    resets_at: Some(500),
                }),
                secondary: Some(AccountRateLimitWindow {
                    used_percent: 100.0,
                    window_minutes: Some(10080),
                    resets_at: Some(900),
                }),
            }),
        );

        assert_eq!(
            signal,
            AccountLimitSignal {
                kind: LimitSignalKind::UsageLimit,
                recorded_at: 100,
                cooldown_until: Some(900),
            }
        );
    }

    #[test]
    fn rate_limit_falls_back_to_short_cooldown_without_snapshot() {
        let signal = infer_limit_signal(LimitSignalKind::RateLimit, 100, None);

        assert_eq!(
            signal,
            AccountLimitSignal {
                kind: LimitSignalKind::RateLimit,
                recorded_at: 100,
                cooldown_until: Some(1000),
            }
        );
    }
}
