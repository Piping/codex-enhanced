use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::account_signal::AccountLimitSignal;
use crate::account_signal::AccountRateLimitSnapshot;

pub const ACCOUNT_POOL_STATE_RELATIVE_PATH: &str = "accounts/account-pool.json";
const ACCOUNT_POOL_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageWindowKind {
    FiveHour,
    Weekly,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageEstimateSource {
    Manual,
    ResponseErrorInference,
    LocalHeuristic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsageWindow {
    pub kind: AccountUsageWindowKind,
    pub label: String,
    pub estimated_used_units: u32,
    pub estimated_limit_units: Option<u32>,
    pub reset_at: Option<i64>,
    pub source: UsageEstimateSource,
}

impl AccountUsageWindow {
    pub fn pressure_permille(&self) -> Option<u16> {
        let limit = self.estimated_limit_units?;
        if limit == 0 {
            return None;
        }

        let used = self.estimated_used_units.min(limit);
        let permille = (u64::from(used) * 1000) / u64::from(limit);
        u16::try_from(permille).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: String,
    pub alias: String,
    pub masked_email: Option<String>,
    pub plan_label: Option<String>,
    pub priority: u32,
    pub enabled: bool,
    pub cooldown_until: Option<i64>,
    pub last_limit_error_at: Option<i64>,
    pub last_selected_at: Option<i64>,
    pub usage_windows: Vec<AccountUsageWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountManagementProfile {
    pub id: String,
    pub alias: Option<String>,
    pub masked_email: Option<String>,
    pub plan_label: Option<String>,
    pub priority: Option<u32>,
}

impl AccountRecord {
    pub fn display_name(&self) -> &str {
        if self.alias.trim().is_empty() {
            &self.id
        } else {
            &self.alias
        }
    }

    pub fn is_available_at(&self, now_ts: i64) -> bool {
        if !self.enabled {
            return false;
        }

        match self.cooldown_until {
            Some(cooldown_until) => cooldown_until <= now_ts,
            None => true,
        }
    }

    pub fn highest_pressure_permille(&self) -> Option<u16> {
        self.usage_windows
            .iter()
            .filter_map(AccountUsageWindow::pressure_permille)
            .max()
    }

    pub fn usage_summary(&self) -> Option<String> {
        let windows: Vec<String> = self
            .usage_windows
            .iter()
            .map(|window| {
                let prefix = match window.kind {
                    AccountUsageWindowKind::FiveHour => "5h",
                    AccountUsageWindowKind::Weekly => "week",
                    AccountUsageWindowKind::Custom => window.label.as_str(),
                };
                match window.estimated_limit_units {
                    Some(limit) => format!("{prefix} {}/{}", window.estimated_used_units, limit),
                    None => format!("{prefix} {}", window.estimated_used_units),
                }
            })
            .collect();
        if windows.is_empty() {
            None
        } else {
            Some(windows.join(" · "))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPoolState {
    pub version: u32,
    pub active_account_id: Option<String>,
    pub accounts: Vec<AccountRecord>,
}

impl Default for AccountPoolState {
    fn default() -> Self {
        Self {
            version: ACCOUNT_POOL_STATE_VERSION,
            active_account_id: None,
            accounts: Vec::new(),
        }
    }
}

impl AccountPoolState {
    pub fn upsert_account(&mut self, profile: AccountManagementProfile) -> bool {
        if let Some(account) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == profile.id)
        {
            let mut changed = false;
            let next_alias = profile.alias.unwrap_or_else(|| account.alias.clone());
            if account.alias != next_alias {
                account.alias = next_alias;
                changed = true;
            }
            if account.masked_email != profile.masked_email {
                account.masked_email = profile.masked_email;
                changed = true;
            }
            if account.plan_label != profile.plan_label {
                account.plan_label = profile.plan_label;
                changed = true;
            }
            if let Some(priority) = profile.priority
                && account.priority != priority
            {
                account.priority = priority;
                changed = true;
            }
            return changed;
        }

        self.accounts.push(AccountRecord {
            id: profile.id.clone(),
            alias: profile.alias.unwrap_or(profile.id.clone()),
            masked_email: profile.masked_email,
            plan_label: profile.plan_label,
            priority: profile.priority.unwrap_or_else(|| {
                u32::try_from(self.accounts.len()).unwrap_or(u32::MAX.saturating_sub(1))
            }),
            enabled: true,
            cooldown_until: None,
            last_limit_error_at: None,
            last_selected_at: None,
            usage_windows: Vec::new(),
        });
        if self.active_account_id.is_none() {
            self.active_account_id = Some(profile.id);
        }
        true
    }

    pub fn set_active_account(&mut self, account_id: &str, now_ts: i64) -> bool {
        let Some(account) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == account_id)
        else {
            return false;
        };
        account.last_selected_at = Some(now_ts);
        let should_change = self.active_account_id.as_deref() != Some(account_id);
        self.active_account_id = Some(account_id.to_string());
        should_change || account.last_selected_at == Some(now_ts)
    }

    pub fn rename_account_alias(&mut self, account_id: &str, alias: String) -> bool {
        let normalized = alias.trim();
        let Some(account) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == account_id)
        else {
            return false;
        };
        let next_alias = if normalized.is_empty() {
            account.id.clone()
        } else {
            normalized.to_string()
        };
        if account.alias == next_alias {
            false
        } else {
            account.alias = next_alias;
            true
        }
    }

    pub fn remove_account(&mut self, account_id: &str) -> bool {
        let original_len = self.accounts.len();
        self.accounts.retain(|account| account.id != account_id);
        if self.accounts.len() == original_len {
            return false;
        }

        if self.active_account_id.as_deref() == Some(account_id) {
            self.active_account_id = self.accounts.first().map(|account| account.id.clone());
        }

        true
    }

    pub fn apply_rate_limit_snapshot(
        &mut self,
        account_id: &str,
        snapshot: &AccountRateLimitSnapshot,
    ) -> bool {
        let Some(account) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == account_id)
        else {
            return false;
        };
        let previous = account.usage_windows.clone();
        account.usage_windows = rate_limit_windows(snapshot);
        previous != account.usage_windows
    }

    pub fn apply_limit_signal(&mut self, account_id: &str, signal: &AccountLimitSignal) -> bool {
        let Some(account) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == account_id)
        else {
            return false;
        };

        let mut changed = false;
        if account.last_limit_error_at != Some(signal.recorded_at) {
            account.last_limit_error_at = Some(signal.recorded_at);
            changed = true;
        }
        if account.cooldown_until != signal.cooldown_until {
            account.cooldown_until = signal.cooldown_until;
            changed = true;
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPoolStore {
    codex_home: PathBuf,
}

impl AccountPoolStore {
    pub fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    pub fn path(&self) -> PathBuf {
        self.codex_home.join(ACCOUNT_POOL_STATE_RELATIVE_PATH)
    }

    pub fn load(&self) -> io::Result<AccountPoolState> {
        let path = self.path();
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(io::Error::other),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AccountPoolState::default()),
            Err(err) => Err(err),
        }
    }

    pub fn save(&self, state: &AccountPoolState) -> io::Result<()> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
        fs::write(path, contents)
    }

    pub fn update<F>(&self, updater: F) -> io::Result<AccountPoolState>
    where
        F: FnOnce(&mut AccountPoolState),
    {
        let mut state = self.load()?;
        updater(&mut state);
        self.save(&state)?;
        Ok(state)
    }
}

fn rate_limit_windows(snapshot: &AccountRateLimitSnapshot) -> Vec<AccountUsageWindow> {
    let mut windows = Vec::new();
    for (kind, label, window) in [
        (
            AccountUsageWindowKind::FiveHour,
            "5h",
            snapshot.primary.as_ref(),
        ),
        (
            AccountUsageWindowKind::Weekly,
            "week",
            snapshot.secondary.as_ref(),
        ),
    ] {
        let Some(window) = window else {
            continue;
        };
        windows.push(AccountUsageWindow {
            kind,
            label: label.to_string(),
            estimated_used_units: window.used_percent.round() as u32,
            estimated_limit_units: Some(100),
            reset_at: window.resets_at,
            source: UsageEstimateSource::ResponseErrorInference,
        });
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::AccountManagementProfile;
    use super::AccountPoolState;
    use super::AccountPoolStore;
    use super::AccountRateLimitSnapshot;
    use super::AccountRateLimitWindow;
    use super::AccountRecord;
    use super::AccountUsageWindow;
    use super::AccountUsageWindowKind;
    use super::UsageEstimateSource;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn missing_account_pool_file_returns_default_state() {
        let tempdir = tempdir().expect("tempdir");
        let store = AccountPoolStore::new(tempdir.path().to_path_buf());

        assert_eq!(store.load().expect("load"), AccountPoolState::default());
    }

    #[test]
    fn save_round_trips_account_pool_state() {
        let tempdir = tempdir().expect("tempdir");
        let store = AccountPoolStore::new(tempdir.path().to_path_buf());
        let state = AccountPoolState {
            version: 1,
            active_account_id: Some("account-1".to_string()),
            accounts: vec![AccountRecord {
                id: "account-1".to_string(),
                alias: "Primary".to_string(),
                masked_email: Some("pri***@example.com".to_string()),
                plan_label: Some("pro".to_string()),
                priority: 0,
                enabled: true,
                cooldown_until: None,
                last_limit_error_at: None,
                last_selected_at: Some(12),
                usage_windows: vec![AccountUsageWindow {
                    kind: AccountUsageWindowKind::FiveHour,
                    label: "5h".to_string(),
                    estimated_used_units: 10,
                    estimated_limit_units: Some(20),
                    reset_at: Some(100),
                    source: UsageEstimateSource::Manual,
                }],
            }],
        };

        store.save(&state).expect("save");

        assert_eq!(store.load().expect("load"), state);
    }

    #[test]
    fn upsert_account_preserves_existing_alias_when_profile_has_none() {
        let mut state = AccountPoolState {
            version: 1,
            active_account_id: Some("account-1".to_string()),
            accounts: vec![AccountRecord {
                id: "account-1".to_string(),
                alias: "Primary".to_string(),
                masked_email: None,
                plan_label: None,
                priority: 0,
                enabled: true,
                cooldown_until: None,
                last_limit_error_at: None,
                last_selected_at: None,
                usage_windows: Vec::new(),
            }],
        };

        assert!(state.upsert_account(AccountManagementProfile {
            id: "account-1".to_string(),
            alias: None,
            masked_email: Some("pri***@example.com".to_string()),
            plan_label: Some("pro".to_string()),
            priority: None,
        }));
        assert_eq!(state.accounts[0].alias, "Primary");
    }

    #[test]
    fn apply_rate_limit_snapshot_rewrites_usage_windows() {
        let mut state = AccountPoolState::default();
        state.upsert_account(AccountManagementProfile {
            id: "account-1".to_string(),
            alias: Some("Primary".to_string()),
            masked_email: None,
            plan_label: None,
            priority: Some(0),
        });

        let changed = state.apply_rate_limit_snapshot(
            "account-1",
            &AccountRateLimitSnapshot {
                limit_name: Some("codex".to_string()),
                primary: Some(AccountRateLimitWindow {
                    used_percent: 45.0,
                    window_minutes: Some(300),
                    resets_at: Some(123),
                }),
                secondary: None,
            },
        );

        assert!(changed);
        assert_eq!(
            state.accounts[0].usage_windows,
            vec![AccountUsageWindow {
                kind: AccountUsageWindowKind::FiveHour,
                label: "5h".to_string(),
                estimated_used_units: 45,
                estimated_limit_units: Some(100),
                reset_at: Some(123),
                source: UsageEstimateSource::ResponseErrorInference,
            }]
        );
    }
}
