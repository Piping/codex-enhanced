use anyhow::Context;
use anyhow::Result;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::ConnectionStatus;
use codex_clawbot::FeishuConfig;
use codex_clawbot::ForwardingDirection;
use codex_clawbot::ForwardingState;
use codex_clawbot::ProviderKind;
use codex_clawbot::ProviderRuntimeState;
use codex_clawbot::ProviderSessionRef;

use super::App;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotFeishuConfigField;
use crate::app_event::ClawbotForwardingChannel;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;

const CLAWBOT_MANAGEMENT_VIEW_ID: &str = "clawbot-management";

impl ClawbotFeishuConfigField {
    fn title(self) -> &'static str {
        match self {
            Self::AppId => "Feishu App ID",
            Self::AppSecret => "Feishu App Secret",
            Self::VerificationToken => "Feishu Verification Token",
            Self::EncryptKey => "Feishu Encrypt Key",
            Self::BotOpenId => "Feishu Bot Open ID",
            Self::BotUserId => "Feishu Bot User ID",
        }
    }

    fn current_value(self, config: Option<&FeishuConfig>) -> Option<String> {
        let value = match self {
            Self::AppId => config.map(|config| config.app_id.clone()),
            Self::AppSecret => config.map(|config| config.app_secret.clone()),
            Self::VerificationToken => config.and_then(|config| config.verification_token.clone()),
            Self::EncryptKey => config.and_then(|config| config.encrypt_key.clone()),
            Self::BotOpenId => config.and_then(|config| config.bot_open_id.clone()),
            Self::BotUserId => config.and_then(|config| config.bot_user_id.clone()),
        }?;
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    fn description(self, config: Option<&FeishuConfig>) -> String {
        let Some(value) = self.current_value(config) else {
            return "Not set".to_string();
        };
        if self.is_secret() {
            format!("Configured: {}", mask_secret(&value))
        } else {
            format!("Configured: {}", truncate_value(&value, 28))
        }
    }

    fn prompt_placeholder(self) -> &'static str {
        match self {
            Self::AppId => "Paste the Feishu app_id and press Enter",
            Self::AppSecret => "Paste the Feishu app_secret and press Enter",
            Self::VerificationToken => {
                "Paste the verification token, or submit an empty value to clear it"
            }
            Self::EncryptKey => "Paste the encrypt key, or submit an empty value to clear it",
            Self::BotOpenId => "Paste the bot open_id, or submit an empty value to clear it",
            Self::BotUserId => "Paste the bot user_id, or submit an empty value to clear it",
        }
    }

    fn prompt_context_label(self, config: Option<&FeishuConfig>) -> String {
        match self.current_value(config) {
            Some(value) if self.is_secret() => {
                format!("Current: {}", mask_secret(&value))
            }
            Some(value) => {
                format!("Current: {}", truncate_value(&value, 40))
            }
            None => "Current: not set".to_string(),
        }
    }

    fn is_secret(self) -> bool {
        matches!(
            self,
            Self::AppSecret | Self::VerificationToken | Self::EncryptKey
        )
    }
}

impl ClawbotForwardingChannel {
    fn title(self) -> &'static str {
        match self {
            Self::Inbound => "Inbound Forwarding",
            Self::Outbound => "Outbound Forwarding",
        }
    }

    fn description(self, enabled: bool) -> String {
        let direction = match self {
            Self::Inbound => "Feishu -> Codex",
            Self::Outbound => "Codex -> Feishu",
        };
        let state = if enabled { "Enabled" } else { "Disabled" };
        format!("{state}: {direction}")
    }

    fn selected_description(self, enabled: bool) -> String {
        match (self, enabled) {
            (Self::Inbound, true) => {
                "Disable automatic delivery of unread Feishu messages into the bound thread."
                    .to_string()
            }
            (Self::Inbound, false) => {
                "Re-enable automatic delivery of unread Feishu messages into the bound thread."
                    .to_string()
            }
            (Self::Outbound, true) => {
                "Disable reply forwarding from Codex back to the bound Feishu session.".to_string()
            }
            (Self::Outbound, false) => {
                "Re-enable reply forwarding from Codex back to the bound Feishu session."
                    .to_string()
            }
        }
    }
}

impl App {
    pub(crate) fn open_clawbot_management_popup(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_MANAGEMENT_VIEW_ID);
        let params = self.clawbot_management_popup_params(initial_selected_idx);
        if !self
            .chat_widget
            .replace_selection_view_if_active(CLAWBOT_MANAGEMENT_VIEW_ID, params)
        {
            self.chat_widget
                .show_selection_view(self.clawbot_management_popup_params(initial_selected_idx));
        }
    }

    pub(crate) fn open_clawbot_feishu_config_prompt(&mut self, field: ClawbotFeishuConfigField) {
        let config = ClawbotRuntime::load(self.config.cwd.to_path_buf())
            .ok()
            .and_then(|runtime| runtime.snapshot().config.feishu.clone());
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            field.title().to_string(),
            field.prompt_placeholder().to_string(),
            Some(field.prompt_context_label(config.as_ref())),
            Box::new(move |value| {
                tx.send(AppEvent::SaveClawbotFeishuConfigValue { field, value });
            }),
        );
        self.chat_widget.show_view(Box::new(view));
    }

    pub(crate) fn open_clawbot_manual_bind_prompt(&mut self) {
        let current_thread = self
            .active_thread_id
            .map(|thread_id| thread_id.to_string())
            .unwrap_or_else(|| "No active thread".to_string());
        let current_binding = self
            .active_thread_id
            .and_then(|thread_id| {
                ClawbotRuntime::load(self.config.cwd.to_path_buf())
                    .ok()
                    .and_then(|runtime| {
                        runtime
                            .bound_session_for_thread(&thread_id.to_string())
                            .ok()
                    })
                    .flatten()
                    .map(|session| session.session_id)
            })
            .unwrap_or_else(|| "not bound".to_string());
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Bind Feishu session to current thread".to_string(),
            "Paste a Feishu session id and press Enter".to_string(),
            Some(format!(
                "Thread: {current_thread} · Current binding: {current_binding}"
            )),
            Box::new(move |session_id| {
                tx.send(AppEvent::SaveClawbotManualBindSessionId { session_id });
            }),
        );
        self.chat_widget.show_view(Box::new(view));
    }

    pub(crate) fn save_clawbot_feishu_config_value(
        &mut self,
        field: ClawbotFeishuConfigField,
        value: String,
    ) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        let mut config = runtime.snapshot().config.feishu.clone().unwrap_or_default();
        let trimmed = value.trim().to_string();
        match field {
            ClawbotFeishuConfigField::AppId => {
                config.app_id = trimmed;
            }
            ClawbotFeishuConfigField::AppSecret => {
                config.app_secret = trimmed;
            }
            ClawbotFeishuConfigField::VerificationToken => {
                config.verification_token = (!trimmed.is_empty()).then_some(trimmed);
            }
            ClawbotFeishuConfigField::EncryptKey => {
                config.encrypt_key = (!trimmed.is_empty()).then_some(trimmed);
            }
            ClawbotFeishuConfigField::BotOpenId => {
                config.bot_open_id = (!trimmed.is_empty()).then_some(trimmed);
            }
            ClawbotFeishuConfigField::BotUserId => {
                config.bot_user_id = (!trimmed.is_empty()).then_some(trimmed);
            }
        }
        runtime.update_feishu_config(Some(config))?;
        self.refresh_clawbot_provider_runtime()?;
        self.open_clawbot_management_popup();
        self.chat_widget
            .add_info_message(format!("Updated {}.", field.title()), /*hint*/ None);
        Ok(())
    }

    pub(crate) async fn save_clawbot_manual_bind_session_id(
        &mut self,
        app_server: &mut AppServerSession,
        session_id: String,
    ) -> Result<()> {
        let thread_id = self
            .active_thread_id
            .context("no active thread available for Clawbot binding")?;
        let trimmed = session_id.trim().to_string();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("session id cannot be empty"));
        }
        let session = ProviderSessionRef::new(ProviderKind::Feishu, trimmed.clone());
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        runtime.connect_session_to_thread(&session, thread_id.to_string())?;
        self.refresh_clawbot_provider_runtime()?;
        self.dispatch_next_clawbot_message(app_server, &session)
            .await?;
        self.open_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!("Bound thread {thread_id} to Feishu session {trimmed}."),
            /*hint*/ None,
        );
        Ok(())
    }

    pub(crate) fn clawbot_disconnect_current_thread(&mut self) -> Result<()> {
        let thread_id = self
            .active_thread_id
            .context("no active thread available for Clawbot disconnect")?;
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        let Some(session) = runtime.disconnect_thread(&thread_id.to_string())? else {
            return Err(anyhow::anyhow!(
                "current thread is not bound to a Clawbot session"
            ));
        };
        self.open_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!(
                "Disconnected Feishu session {} from thread {thread_id}.",
                session.session_id
            ),
            /*hint*/ None,
        );
        Ok(())
    }

    pub(crate) fn clawbot_set_current_thread_forwarding(
        &mut self,
        channel: ClawbotForwardingChannel,
        enabled: bool,
    ) -> Result<()> {
        let thread_id = self
            .active_thread_id
            .context("no active thread available for Clawbot forwarding")?;
        let direction = match channel {
            ClawbotForwardingChannel::Inbound => ForwardingDirection::Inbound,
            ClawbotForwardingChannel::Outbound => ForwardingDirection::Outbound,
        };
        let state = if enabled {
            ForwardingState::Enabled
        } else {
            ForwardingState::Disabled
        };
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        runtime
            .set_forwarding_state_for_thread(&thread_id.to_string(), direction, state)?
            .context("current thread is not bound to a Clawbot session")?;
        self.open_clawbot_management_popup();
        self.chat_widget
            .add_info_message(channel.description(enabled), /*hint*/ None);
        Ok(())
    }

    pub(crate) fn retry_clawbot_feishu_connection(&mut self) -> Result<()> {
        let runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        let Some(config) = runtime.snapshot().config.feishu.as_ref() else {
            return Err(anyhow::anyhow!("Feishu credentials are not configured"));
        };
        if !config.has_api_credentials() {
            return Err(anyhow::anyhow!("Feishu app_id and app_secret are required"));
        }
        self.refresh_clawbot_provider_runtime()?;
        self.open_clawbot_management_popup();
        self.chat_widget.add_info_message(
            "Restarted Feishu runtime bridge.".to_string(),
            /*hint*/ None,
        );
        Ok(())
    }

    fn clawbot_management_popup_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let snapshot = ClawbotRuntime::load(self.config.cwd.to_path_buf())
            .map(|runtime| runtime.snapshot().clone())
            .unwrap_or_default();
        let feishu_config = snapshot.config.feishu.as_ref();
        let provider_state = snapshot
            .runtime
            .iter()
            .find(|state| state.provider == ProviderKind::Feishu)
            .cloned()
            .unwrap_or(ProviderRuntimeState::unconfigured(ProviderKind::Feishu));
        let active_thread_id = self.active_thread_id.map(|thread_id| thread_id.to_string());
        let current_binding = active_thread_id.as_deref().and_then(|thread_id| {
            snapshot
                .bindings
                .iter()
                .find(|binding| binding.thread_id == thread_id)
        });
        let mut items = vec![
            clawbot_config_item(ClawbotFeishuConfigField::AppId, feishu_config),
            clawbot_config_item(ClawbotFeishuConfigField::AppSecret, feishu_config),
            clawbot_config_item(ClawbotFeishuConfigField::VerificationToken, feishu_config),
            clawbot_config_item(ClawbotFeishuConfigField::EncryptKey, feishu_config),
            clawbot_config_item(ClawbotFeishuConfigField::BotOpenId, feishu_config),
            clawbot_config_item(ClawbotFeishuConfigField::BotUserId, feishu_config),
            SelectionItem {
                name: "Retry Feishu Connection".to_string(),
                description: Some(connection_description(&provider_state)),
                selected_description: Some(
                    "Restart the workspace-local Feishu websocket/runtime bridge.".to_string(),
                ),
                is_disabled: !feishu_config.is_some_and(FeishuConfig::has_api_credentials),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::RetryClawbotFeishuConnection)
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
        ];

        let bind_description = match (&active_thread_id, current_binding) {
            (Some(thread_id), Some(binding)) => {
                format!("Thread {thread_id} -> {}", binding.session_id)
            }
            (Some(thread_id), None) => format!("Thread {thread_id} is not bound"),
            (None, _) => "No active thread".to_string(),
        };
        items.push(SelectionItem {
            name: "Bind Current Thread".to_string(),
            description: Some(bind_description),
            selected_description: Some(
                "Manually bind the current Codex thread to a Feishu session id.".to_string(),
            ),
            is_disabled: active_thread_id.is_none(),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::OpenClawbotManualBindPrompt)
            })],
            dismiss_on_select: false,
            ..Default::default()
        });

        for channel in [
            ClawbotForwardingChannel::Inbound,
            ClawbotForwardingChannel::Outbound,
        ] {
            let enabled = current_binding.is_some_and(|binding| match channel {
                ClawbotForwardingChannel::Inbound => binding.inbound_forwarding_enabled,
                ClawbotForwardingChannel::Outbound => binding.outbound_forwarding_enabled,
            });
            items.push(SelectionItem {
                name: channel.title().to_string(),
                description: Some(channel.description(enabled)),
                selected_description: Some(channel.selected_description(enabled)),
                is_disabled: current_binding.is_none(),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetCurrentThreadForwarding {
                        channel,
                        enabled: !enabled,
                    });
                })],
                dismiss_on_select: false,
                ..Default::default()
            });
        }

        items.push(SelectionItem {
            name: "Disconnect Current Thread".to_string(),
            description: Some(
                current_binding
                    .map(|binding| format!("Unbind {}", binding.session_id))
                    .unwrap_or_else(|| "No binding for current thread".to_string()),
            ),
            selected_description: Some(
                "Remove the current thread's Feishu binding without deleting cached session state."
                    .to_string(),
            ),
            is_disabled: current_binding.is_none(),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::ClawbotDisconnectCurrentThread)
            })],
            dismiss_on_select: false,
            ..Default::default()
        });

        SelectionViewParams {
            view_id: Some(CLAWBOT_MANAGEMENT_VIEW_ID),
            title: Some("Clawbot".to_string()),
            subtitle: Some(
                "Manage workspace-local Feishu credentials and the current thread binding."
                    .to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            ..Default::default()
        }
    }
}

fn clawbot_config_item(
    field: ClawbotFeishuConfigField,
    config: Option<&FeishuConfig>,
) -> SelectionItem {
    SelectionItem {
        name: field.title().to_string(),
        description: Some(field.description(config)),
        selected_description: Some(
            "Persist this workspace-local Feishu setting under .codex/clawbot/config.toml."
                .to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenClawbotFeishuConfigPrompt { field });
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn connection_description(state: &ProviderRuntimeState) -> String {
    let status = match state.connection {
        ConnectionStatus::Unconfigured => "Unconfigured",
        ConnectionStatus::Disconnected => "Disconnected",
        ConnectionStatus::Connecting => "Connecting",
        ConnectionStatus::Connected => "Connected",
        ConnectionStatus::Error => "Error",
    };
    match state.last_error.as_deref() {
        Some(error) if !error.trim().is_empty() => {
            format!("{status}: {}", truncate_value(error, 48))
        }
        _ => status.to_string(),
    }
}

fn mask_secret(value: &str) -> String {
    let chars = value.chars().count();
    if chars <= 4 {
        return "*".repeat(chars.max(1));
    }
    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}{}", "*".repeat(chars.saturating_sub(4)), suffix)
}

fn truncate_value(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    let prefix = chars
        .into_iter()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    format!("{prefix}…")
}
