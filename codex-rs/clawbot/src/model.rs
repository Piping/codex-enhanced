use serde::Deserialize;
use serde::Serialize;

use crate::config::ClawbotConfig;

pub const CLAWBOT_RELATIVE_DIR: &str = ".codex/clawbot";
pub const CLAWBOT_CONFIG_RELATIVE_PATH: &str = ".codex/clawbot/config.toml";
pub const CLAWBOT_SESSIONS_RELATIVE_PATH: &str = ".codex/clawbot/sessions.json";
pub const CLAWBOT_BINDINGS_RELATIVE_PATH: &str = ".codex/clawbot/bindings.json";
pub const CLAWBOT_UNREAD_MESSAGES_RELATIVE_PATH: &str = ".codex/clawbot/unread_messages.jsonl";
pub const CLAWBOT_RUNTIME_RELATIVE_PATH: &str = ".codex/clawbot/runtime.json";
pub const CLAWBOT_INBOUND_RECEIPTS_RELATIVE_PATH: &str = ".codex/clawbot/inbound_receipts.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Feishu,
}

impl ProviderKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Feishu => "Feishu",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    #[default]
    Unconfigured,
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl ConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Discovered,
    Bound,
    Disconnected,
    Error,
}

impl SessionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Bound => "bound",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRuntimeState {
    pub provider: ProviderKind,
    pub connection: ConnectionStatus,
    pub last_error: Option<String>,
    pub updated_at: Option<i64>,
}

impl ProviderRuntimeState {
    pub fn unconfigured(provider: ProviderKind) -> Self {
        Self {
            provider,
            connection: ConnectionStatus::Unconfigured,
            last_error: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProviderSessionRef {
    pub provider: ProviderKind,
    pub session_id: String,
}

impl ProviderSessionRef {
    pub fn new(provider: ProviderKind, session_id: impl Into<String>) -> Self {
        Self {
            provider,
            session_id: session_id.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProviderMessageRef {
    pub provider: ProviderKind,
    pub session_id: String,
    pub message_id: String,
}

impl ProviderMessageRef {
    pub fn new(
        provider: ProviderKind,
        session_id: impl Into<String>,
        message_id: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            session_id: session_id.into(),
            message_id: message_id.into(),
        }
    }

    pub fn session_ref(&self) -> ProviderSessionRef {
        ProviderSessionRef::new(self.provider, self.session_id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSession {
    pub provider: ProviderKind,
    pub session_id: String,
    pub display_name: Option<String>,
    pub unread_count: usize,
    pub last_message_at: Option<i64>,
    pub status: SessionStatus,
    pub bound_thread_id: Option<String>,
}

impl ProviderSession {
    pub fn session_ref(&self) -> ProviderSessionRef {
        ProviderSessionRef::new(self.provider, self.session_id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBinding {
    pub provider: ProviderKind,
    pub session_id: String,
    pub thread_id: String,
    #[serde(default = "default_session_forwarding_enabled")]
    pub inbound_forwarding_enabled: bool,
    #[serde(default = "default_session_forwarding_enabled")]
    pub outbound_forwarding_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SessionBinding {
    pub fn session_ref(&self) -> ProviderSessionRef {
        ProviderSessionRef::new(self.provider, self.session_id.clone())
    }

    pub fn forwarding_enabled(&self, direction: SessionForwardingDirection) -> bool {
        match direction {
            SessionForwardingDirection::Inbound => self.inbound_forwarding_enabled,
            SessionForwardingDirection::Outbound => self.outbound_forwarding_enabled,
        }
    }

    pub fn set_forwarding_mode(&mut self, mode: SessionForwardingMode) {
        match mode {
            SessionForwardingMode::InboundEnabled => {
                self.inbound_forwarding_enabled = true;
            }
            SessionForwardingMode::InboundDisabled => {
                self.inbound_forwarding_enabled = false;
            }
            SessionForwardingMode::OutboundEnabled => {
                self.outbound_forwarding_enabled = true;
            }
            SessionForwardingMode::OutboundDisabled => {
                self.outbound_forwarding_enabled = false;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionForwardingDirection {
    Inbound,
    Outbound,
}

impl SessionForwardingDirection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inbound => "Inbound",
            Self::Outbound => "Outbound",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionForwardingMode {
    InboundEnabled,
    InboundDisabled,
    OutboundEnabled,
    OutboundDisabled,
}

impl SessionForwardingMode {
    pub fn direction(self) -> SessionForwardingDirection {
        match self {
            Self::InboundEnabled | Self::InboundDisabled => SessionForwardingDirection::Inbound,
            Self::OutboundEnabled | Self::OutboundDisabled => SessionForwardingDirection::Outbound,
        }
    }

    pub fn enabled(self) -> bool {
        match self {
            Self::InboundEnabled | Self::OutboundEnabled => true,
            Self::InboundDisabled | Self::OutboundDisabled => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedUnreadMessage {
    pub provider: ProviderKind,
    pub session_id: String,
    pub message_id: String,
    pub text: String,
    pub received_at: i64,
}

impl CachedUnreadMessage {
    pub fn session_ref(&self) -> ProviderSessionRef {
        ProviderSessionRef::new(self.provider, self.session_id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundMessageReceipt {
    pub provider: ProviderKind,
    pub session_id: String,
    pub message_id: String,
    pub received_at: i64,
}

impl InboundMessageReceipt {
    pub fn session_ref(&self) -> ProviderSessionRef {
        ProviderSessionRef::new(self.provider, self.session_id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClawbotSnapshot {
    pub config: ClawbotConfig,
    pub runtime: Vec<ProviderRuntimeState>,
    pub sessions: Vec<ProviderSession>,
    pub bindings: Vec<SessionBinding>,
    pub unread_message_count: usize,
}

impl ClawbotSnapshot {
    pub fn provider_state(&self, provider: ProviderKind) -> Option<&ProviderRuntimeState> {
        self.runtime.iter().find(|state| state.provider == provider)
    }
}

fn default_session_forwarding_enabled() -> bool {
    true
}
