use serde::Deserialize;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClawbotTurnMode {
    #[default]
    Interactive,
    NonInteractive,
}

impl ClawbotTurnMode {
    pub fn uses_noninteractive_prompt_handling(self) -> bool {
        matches!(self, Self::NonInteractive)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClawbotConfig {
    pub feishu: Option<FeishuConfig>,
    pub turn_mode: ClawbotTurnMode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub verification_token: Option<String>,
    pub encrypt_key: Option<String>,
    pub bot_open_id: Option<String>,
    pub bot_user_id: Option<String>,
    pub coordination: Option<FeishuCoordinationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FeishuCoordinationConfig {
    pub base_token: String,
    pub heartbeat_table_id: String,
    pub force_table_id: String,
    pub instance_id: Option<String>,
    pub owner_priority: i64,
    pub heartbeat_interval_secs: u64,
    pub heartbeat_ttl_secs: u64,
    pub force_connect: bool,
}

impl Default for FeishuCoordinationConfig {
    fn default() -> Self {
        Self {
            base_token: String::new(),
            heartbeat_table_id: String::new(),
            force_table_id: String::new(),
            instance_id: None,
            owner_priority: 100,
            heartbeat_interval_secs: 10,
            heartbeat_ttl_secs: 30,
            force_connect: false,
        }
    }
}

impl FeishuCoordinationConfig {
    pub fn is_configured(&self) -> bool {
        !self.base_token.trim().is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.base_token.trim().is_empty()
            && self.heartbeat_table_id.trim().is_empty()
            && self.force_table_id.trim().is_empty()
            && self
                .instance_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self.owner_priority == Self::default().owner_priority
            && self.heartbeat_interval_secs == Self::default().heartbeat_interval_secs
            && self.heartbeat_ttl_secs == Self::default().heartbeat_ttl_secs
            && !self.force_connect
    }

    pub fn heartbeat_interval(&self) -> Duration {
        Duration::from_secs(self.heartbeat_interval_secs.max(1))
    }

    pub fn heartbeat_ttl(&self) -> Duration {
        Duration::from_secs(
            self.heartbeat_ttl_secs
                .max(self.heartbeat_interval_secs.max(1) * 2),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::FeishuCoordinationConfig;

    #[test]
    fn coordination_is_configured_with_base_token_only() {
        let mut config = FeishuCoordinationConfig {
            base_token: "bascn_test".to_string(),
            ..FeishuCoordinationConfig::default()
        };

        assert!(config.is_configured());

        config.base_token.clear();
        assert!(!config.is_configured());
    }

    #[test]
    fn coordination_ttl_is_at_least_twice_the_interval() {
        let config = FeishuCoordinationConfig {
            base_token: "bascn_test".to_string(),
            heartbeat_interval_secs: 9,
            heartbeat_ttl_secs: 5,
            ..FeishuCoordinationConfig::default()
        };

        assert_eq!(config.heartbeat_ttl().as_secs(), 18);
    }
}

impl FeishuConfig {
    pub fn has_api_credentials(&self) -> bool {
        !self.app_id.trim().is_empty() && !self.app_secret.trim().is_empty()
    }

    pub fn is_bot_sender(
        &self,
        open_id: Option<&str>,
        user_id: Option<&str>,
        app_id: Option<&str>,
    ) -> bool {
        self.bot_open_id
            .as_deref()
            .zip(open_id)
            .is_some_and(|(bot_open_id, sender_open_id)| bot_open_id == sender_open_id)
            || self
                .bot_user_id
                .as_deref()
                .zip(user_id)
                .is_some_and(|(bot_user_id, sender_user_id)| bot_user_id == sender_user_id)
            || app_id.is_some_and(|sender_app_id| sender_app_id == self.app_id)
    }

    pub fn is_empty(&self) -> bool {
        self.app_id.trim().is_empty()
            && self.app_secret.trim().is_empty()
            && self
                .verification_token
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .encrypt_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .bot_open_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .bot_user_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .coordination
                .as_ref()
                .is_none_or(FeishuCoordinationConfig::is_empty)
    }
}
