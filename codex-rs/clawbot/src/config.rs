use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClawbotConfig {
    pub feishu: Option<FeishuConfig>,
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
}

impl FeishuConfig {
    pub fn has_api_credentials(&self) -> bool {
        !self.app_id.trim().is_empty() && !self.app_secret.trim().is_empty()
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
    }
}
