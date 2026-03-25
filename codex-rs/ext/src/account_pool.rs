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

        if signal.cooldown_until.is_some_and(|cooldown_until| {
            account
                .cooldown_until
                .is_none_or(|current| cooldown_until > current)
        }) {
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
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(AccountPoolState::default());
            }
            Err(err) => return Err(err),
        };

        serde_json::from_str(&contents).map_err(io::Error::other)
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
    [snapshot.primary.as_ref(), snapshot.secondary.as_ref()]
        .into_iter()
        .flatten()
        .map(rate_limit_window_to_usage_window)
        .collect()
}

fn rate_limit_window_to_usage_window(
    window: &crate::account_signal::AccountRateLimitWindow,
) -> AccountUsageWindow {
    let used_percent = window.used_percent.clamp(0.0, 100.0).round() as u32;
    let (kind, label) = match window.window_minutes {
        Some(300) => (AccountUsageWindowKind::FiveHour, "5h".to_string()),
        Some(10_080) => (AccountUsageWindowKind::Weekly, "week".to_string()),
        Some(minutes) => (AccountUsageWindowKind::Custom, format!("{minutes}m")),
        None => (AccountUsageWindowKind::Custom, "window".to_string()),
    };

    AccountUsageWindow {
        kind,
        label,
        estimated_used_units: used_percent,
        estimated_limit_units: Some(100),
        reset_at: window.resets_at,
        source: UsageEstimateSource::ResponseErrorInference,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::AccountManagementProfile;
    use super::AccountPoolState;
    use super::AccountPoolStore;
    use super::AccountRecord;
    use super::AccountUsageWindow;
    use super::AccountUsageWindowKind;
    use super::UsageEstimateSource;
    use crate::account_signal::AccountLimitSignal;
    use crate::account_signal::AccountRateLimitSnapshot;
    use crate::account_signal::AccountRateLimitWindow;
    use crate::account_signal::LimitSignalKind;

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
            active_account_id: Some("acc-primary".to_string()),
            accounts: vec![AccountRecord {
                id: "acc-primary".to_string(),
                alias: "Primary".to_string(),
                masked_email: Some("pri***@example.com".to_string()),
                plan_label: Some("pro".to_string()),
                priority: 0,
                enabled: true,
                cooldown_until: None,
                last_limit_error_at: None,
                last_selected_at: Some(10),
                usage_windows: vec![AccountUsageWindow {
                    kind: AccountUsageWindowKind::FiveHour,
                    label: "5h".to_string(),
                    estimated_used_units: 12,
                    estimated_limit_units: Some(40),
                    reset_at: Some(100),
                    source: UsageEstimateSource::LocalHeuristic,
                }],
            }],
        };

        store.save(&state).expect("save");

        assert_eq!(store.load().expect("reload"), state);
    }

    #[test]
    fn upsert_and_activate_account() {
        let mut state = AccountPoolState::default();

        assert!(state.upsert_account(AccountManagementProfile {
            id: "acc-primary".to_string(),
            alias: Some("Primary".to_string()),
            masked_email: Some("pri***@example.com".to_string()),
            plan_label: Some("pro".to_string()),
            priority: Some(0),
        }));
        assert!(state.set_active_account("acc-primary", 123));

        assert_eq!(state.active_account_id, Some("acc-primary".to_string()));
        assert_eq!(state.accounts[0].alias, "Primary");
        assert_eq!(state.accounts[0].last_selected_at, Some(123));
    }

    #[test]
    fn upsert_without_alias_preserves_existing_alias() {
        let mut state = AccountPoolState {
            version: 1,
            active_account_id: Some("acc-primary".to_string()),
            accounts: vec![AccountRecord {
                id: "acc-primary".to_string(),
                alias: "Primary".to_string(),
                masked_email: Some("pri***@example.com".to_string()),
                plan_label: Some("pro".to_string()),
                priority: 0,
                enabled: true,
                cooldown_until: None,
                last_limit_error_at: None,
                last_selected_at: None,
                usage_windows: Vec::new(),
            }],
        };

        assert!(state.upsert_account(AccountManagementProfile {
            id: "acc-primary".to_string(),
            alias: None,
            masked_email: Some("new***@example.com".to_string()),
            plan_label: Some("plus".to_string()),
            priority: None,
        }));

        assert_eq!(state.accounts[0].alias, "Primary");
        assert_eq!(
            state.accounts[0].masked_email,
            Some("new***@example.com".to_string())
        );
        assert_eq!(state.accounts[0].plan_label, Some("plus".to_string()));
    }

    #[test]
    fn applying_rate_limit_snapshot_replaces_usage_windows() {
        let mut state = AccountPoolState {
            version: 1,
            active_account_id: Some("acc-primary".to_string()),
            accounts: vec![AccountRecord {
                id: "acc-primary".to_string(),
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

        assert!(state.apply_rate_limit_snapshot(
            "acc-primary",
            &AccountRateLimitSnapshot {
                limit_name: Some("codex".to_string()),
                primary: Some(AccountRateLimitWindow {
                    used_percent: 87.0,
                    window_minutes: Some(300),
                    resets_at: Some(500),
                }),
                secondary: None,
            }
        ));

        assert_eq!(
            state.accounts[0].usage_windows,
            vec![AccountUsageWindow {
                kind: AccountUsageWindowKind::FiveHour,
                label: "5h".to_string(),
                estimated_used_units: 87,
                estimated_limit_units: Some(100),
                reset_at: Some(500),
                source: UsageEstimateSource::ResponseErrorInference,
            }]
        );
    }

    #[test]
    fn limit_signal_updates_cooldown() {
        let mut state = AccountPoolState {
            version: 1,
            active_account_id: Some("acc-primary".to_string()),
            accounts: vec![AccountRecord {
                id: "acc-primary".to_string(),
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

        assert!(state.apply_limit_signal(
            "acc-primary",
            &AccountLimitSignal {
                kind: LimitSignalKind::UsageLimit,
                recorded_at: 100,
                cooldown_until: Some(500),
            }
        ));

        assert_eq!(state.accounts[0].last_limit_error_at, Some(100));
        assert_eq!(state.accounts[0].cooldown_until, Some(500));
    }

    #[test]
    fn remove_account_updates_active_selection() {
        let mut state = AccountPoolState {
            version: 1,
            active_account_id: Some("acc-primary".to_string()),
            accounts: vec![
                AccountRecord {
                    id: "acc-primary".to_string(),
                    alias: "Primary".to_string(),
                    masked_email: None,
                    plan_label: None,
                    priority: 0,
                    enabled: true,
                    cooldown_until: None,
                    last_limit_error_at: None,
                    last_selected_at: None,
                    usage_windows: Vec::new(),
                },
                AccountRecord {
                    id: "acc-backup".to_string(),
                    alias: "Backup".to_string(),
                    masked_email: None,
                    plan_label: None,
                    priority: 1,
                    enabled: true,
                    cooldown_until: None,
                    last_limit_error_at: None,
                    last_selected_at: None,
                    usage_windows: Vec::new(),
                },
            ],
        };

        assert!(state.remove_account("acc-primary"));

        assert_eq!(state.active_account_id, Some("acc-backup".to_string()));
        assert_eq!(state.accounts.len(), 1);
        assert_eq!(state.accounts[0].id, "acc-backup");
    }
}
