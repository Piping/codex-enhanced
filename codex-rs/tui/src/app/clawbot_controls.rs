use anyhow::Context;
use anyhow::Result;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::ClawbotTurnMode;
use codex_clawbot::ConnectionStatus;
use codex_clawbot::FeishuConfig;
use codex_clawbot::ForwardingDirection;
use codex_clawbot::ForwardingState;
use codex_clawbot::ProviderKind;
use codex_clawbot::ProviderRuntimeState;
use codex_clawbot::ProviderSession;
use codex_clawbot::ProviderSessionRef;
use codex_protocol::ThreadId;

use super::App;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotFeishuConfigField;
use crate::app_event::ClawbotForwardingChannel;
use crate::app_event_sender::AppEventSender;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionAction;
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
            format!("Configured: {}", truncate_value(&value, /*max_chars*/ 28))
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
                format!("Current: {}", truncate_value(&value, /*max_chars*/ 40))
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

    pub(crate) fn save_clawbot_turn_mode(&mut self, mode: ClawbotTurnMode) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        runtime.update_turn_mode(mode)?;
        self.open_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!(
                "Clawbot turn mode set to {}.",
                clawbot_turn_mode_label(mode)
            ),
            /*hint*/ None,
        );
        Ok(())
    }

    pub(crate) async fn bind_clawbot_session_to_current_thread(
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
        if runtime.snapshot().config.feishu.is_some() {
            runtime.scan_feishu_sessions().await?;
            if !runtime
                .snapshot()
                .sessions
                .iter()
                .any(|existing| existing.session_ref() == session)
            {
                return Err(anyhow::anyhow!(
                    "Feishu session {trimmed} is not visible to the current bot"
                ));
            }
        }
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

    pub(crate) async fn scan_clawbot_feishu_sessions(&mut self) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        runtime.scan_feishu_sessions().await?;
        let discovered = runtime
            .snapshot()
            .sessions
            .iter()
            .filter(|session| session.provider == ProviderKind::Feishu)
            .count();
        self.open_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!("Scanned Feishu sessions. {discovered} discovered."),
            /*hint*/ None,
        );
        Ok(())
    }

    pub(crate) fn clear_clawbot_feishu_sessions(&mut self) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        let sessions_before = runtime
            .snapshot()
            .sessions
            .iter()
            .filter(|session| {
                session.provider == ProviderKind::Feishu && session.bound_thread_id.is_none()
            })
            .count();
        let unread_before = runtime
            .store()
            .load_unread_messages()?
            .into_iter()
            .filter(|message| message.provider == ProviderKind::Feishu)
            .filter(|message| {
                !runtime
                    .snapshot()
                    .bindings
                    .iter()
                    .any(|binding| binding.session_ref() == message.session_ref())
            })
            .count();
        runtime.clear_unbound_feishu_sessions()?;
        self.open_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!(
                "Cleared {sessions_before} unbound Feishu sessions and {unread_before} cached unread messages."
            ),
            /*hint*/ None,
        );
        Ok(())
    }

    fn clawbot_management_popup_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let (snapshot, clearable_unread_count) =
            ClawbotRuntime::load(self.config.cwd.to_path_buf())
                .map(|runtime| {
                    let snapshot = runtime.snapshot().clone();
                    let clearable_unread_count = runtime
                        .store()
                        .load_unread_messages()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|message| message.provider == ProviderKind::Feishu)
                        .filter(|message| {
                            !snapshot
                                .bindings
                                .iter()
                                .any(|binding| binding.session_ref() == message.session_ref())
                        })
                        .count();
                    (snapshot, clearable_unread_count)
                })
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
        let turn_mode = snapshot.config.turn_mode;
        let next_turn_mode = match turn_mode {
            ClawbotTurnMode::Interactive => ClawbotTurnMode::NonInteractive,
            ClawbotTurnMode::NonInteractive => ClawbotTurnMode::Interactive,
        };
        let mut feishu_sessions = snapshot
            .sessions
            .iter()
            .filter(|session| session.provider == ProviderKind::Feishu)
            .cloned()
            .collect::<Vec<_>>();
        feishu_sessions.sort_by(|left, right| {
            right
                .bound_thread_id
                .is_some()
                .cmp(&left.bound_thread_id.is_some())
                .then(session_title(left).cmp(&session_title(right)))
                .then(left.session_id.cmp(&right.session_id))
        });
        let bound_session_count = feishu_sessions
            .iter()
            .filter(|session| session.bound_thread_id.is_some())
            .count();
        let unbound_session_count = feishu_sessions.len().saturating_sub(bound_session_count);
        let mut items = vec![
            SelectionItem {
                name: "Turn Mode".to_string(),
                description: Some(clawbot_turn_mode_summary(turn_mode)),
                selected_description: Some(match turn_mode {
                    ClawbotTurnMode::Interactive => {
                        "Switch clawbot-originated turns into non-interactive mode so remote sessions do not block on prompts.".to_string()
                    }
                    ClawbotTurnMode::NonInteractive => {
                        "Restore normal interactive prompt handling for clawbot-originated turns.".to_string()
                    }
                }),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetTurnMode {
                        mode: next_turn_mode,
                    });
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Feishu Sessions".to_string(),
                description: Some(format!(
                    "{} total · {} bound · {} unbound",
                    feishu_sessions.len(),
                    bound_session_count,
                    unbound_session_count
                )),
                selected_description: Some(
                    "Scan discovered Feishu chats, inspect bound/unbound state, and clear stale unbound cache."
                        .to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Scan Feishu Sessions".to_string(),
                description: Some(
                    "Discover Feishu chats and bot groups using the persisted workspace credentials."
                        .to_string(),
                ),
                selected_description: Some(
                    "Refresh the discovered session list before binding or cleanup."
                        .to_string(),
                ),
                is_disabled: !feishu_config.is_some_and(FeishuConfig::has_api_credentials),
                actions: vec![Box::new(|tx| tx.send(AppEvent::ScanClawbotFeishuSessions))],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Clear Unbound Sessions".to_string(),
                description: Some(format!(
                    "Remove {unbound_session_count} unbound sessions and {clearable_unread_count} cached unread messages."
                )),
                selected_description: Some(
                    "Keep active bindings intact while dropping stale discovered-session cache."
                        .to_string(),
                ),
                is_disabled: unbound_session_count == 0 && clearable_unread_count == 0,
                actions: vec![Box::new(|tx| tx.send(AppEvent::ClearClawbotFeishuSessions))],
                dismiss_on_select: false,
                ..Default::default()
            },
        ];

        if feishu_sessions.is_empty() {
            items.push(SelectionItem {
                name: "No Feishu sessions discovered".to_string(),
                description: Some(
                    "Run Scan Feishu Sessions to refresh the workspace-local discovered session list."
                        .to_string(),
                ),
                selected_description: Some(
                    "Discovered sessions appear here with their current bound or unbound state."
                        .to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            });
        } else {
            items.extend(feishu_sessions.iter().map(|session| {
                clawbot_session_item(
                    session,
                    active_thread_id.as_deref(),
                    current_binding.map(|binding| binding.session_id.as_str()),
                )
            }));
        }

        items.extend([
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
        ]);

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
                "Manage workspace-local Feishu credentials, sessions, and the current thread binding."
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
            format!("{status}: {}", truncate_value(error, /*max_chars*/ 48))
        }
        _ => status.to_string(),
    }
}

fn clawbot_session_item(
    session: &ProviderSession,
    active_thread_id: Option<&str>,
    current_binding_session_id: Option<&str>,
) -> SelectionItem {
    let is_current = current_binding_session_id.is_some_and(|session_id| {
        session_id == session.session_id && session.provider == ProviderKind::Feishu
    });
    let description = if session.bound_thread_id.is_some() {
        format!("bound · {} unread", session.unread_count)
    } else {
        format!("unbound · {} unread", session.unread_count)
    };
    let jump_target = session
        .bound_thread_id
        .as_deref()
        .and_then(|thread_id| ThreadId::from_string(thread_id).ok());
    let selected_description = match (active_thread_id, session.bound_thread_id.as_deref()) {
        (Some(thread_id), Some(bound_thread_id)) if thread_id == bound_thread_id => {
            format!(
                "Current thread {thread_id} is already bound to Feishu session {}.",
                session.session_id
            )
        }
        (_, Some(bound_thread_id)) if jump_target.is_some() => {
            format!(
                "Jump to bound thread {bound_thread_id} to continue or manage Feishu session {}.",
                session.session_id
            )
        }
        (_, Some(bound_thread_id)) => format!(
            "Feishu session {} is bound to invalid thread id {bound_thread_id}.",
            session.session_id
        ),
        (Some(thread_id), None) if is_current => {
            format!(
                "Thread {thread_id} is already bound to Feishu session {}.",
                session.session_id
            )
        }
        (Some(thread_id), None) => format!(
            "Bind current thread {thread_id} directly to Feishu session {}.",
            session.session_id
        ),
        (None, None) => format!(
            "Open a Codex thread before binding Feishu session {}.",
            session.session_id
        ),
    };
    let session_id = session.session_id.clone();
    let actions: Vec<SelectionAction> = if let Some(thread_id) = jump_target {
        vec![Box::new(move |tx: &AppEventSender| {
            tx.send(AppEvent::SelectAgentThread(thread_id));
        })]
    } else {
        vec![Box::new(move |tx: &AppEventSender| {
            tx.send(AppEvent::SaveClawbotManualBindSessionId {
                session_id: session_id.clone(),
            });
        })]
    };
    let is_disabled = match session.bound_thread_id.as_deref() {
        Some(bound_thread_id) => active_thread_id == Some(bound_thread_id) || jump_target.is_none(),
        None => active_thread_id.is_none() || is_current,
    };
    SelectionItem {
        name: session_title(session),
        description: Some(description),
        selected_description: Some(selected_description),
        is_disabled,
        actions,
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn session_title(session: &ProviderSession) -> String {
    session
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| session.session_id.clone())
}

fn clawbot_turn_mode_label(mode: ClawbotTurnMode) -> &'static str {
    match mode {
        ClawbotTurnMode::Interactive => "interactive",
        ClawbotTurnMode::NonInteractive => "non-interactive",
    }
}

fn clawbot_turn_mode_summary(mode: ClawbotTurnMode) -> String {
    match mode {
        ClawbotTurnMode::Interactive => {
            "interactive: clawbot turns may surface question and approval prompts.".to_string()
        }
        ClawbotTurnMode::NonInteractive => {
            "non-interactive: clawbot turns auto-dismiss question and permission prompts."
                .to_string()
        }
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

#[cfg(test)]
mod tests {
    use codex_clawbot::ProviderKind;
    use codex_clawbot::ProviderSession;
    use codex_protocol::ThreadId;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    use super::clawbot_session_item;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
    use crate::bottom_pane::SelectionViewParams;
    use crate::render::renderable::Renderable;

    fn render_selection_popup(view: &ListSelectionView, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, width, height);
                view.render(area, frame.buffer_mut());
            })
            .expect("draw popup");
        format!("{:?}", terminal.backend())
    }

    #[test]
    fn bound_session_item_jumps_to_bound_thread() {
        let item = clawbot_session_item(
            &ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_bound".to_string(),
                display_name: Some("tracker".to_string()),
                unread_count: 2,
                last_message_at: None,
                status: codex_clawbot::SessionStatus::Bound,
                bound_thread_id: Some("019d607a-cf72-72e1-a5b7-0dc17ad019ad".to_string()),
            },
            Some("019d607a-cf72-72e1-a5b7-0dc17ad019ae"),
            None,
        );

        assert!(!item.is_disabled);
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        (item.actions[0])(&tx);

        assert!(
            matches!(
                rx.try_recv().expect("event"),
                AppEvent::SelectAgentThread(thread_id)
                    if thread_id
                        == ThreadId::from_string("019d607a-cf72-72e1-a5b7-0dc17ad019ad")
                            .expect("thread id")
            ),
            "expected bound session item to jump to the bound thread"
        );
    }

    #[test]
    fn bound_session_jump_item_snapshot() {
        let item = clawbot_session_item(
            &ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_bound".to_string(),
                display_name: Some("tracker".to_string()),
                unread_count: 2,
                last_message_at: None,
                status: codex_clawbot::SessionStatus::Bound,
                bound_thread_id: Some("019d607a-cf72-72e1-a5b7-0dc17ad019ad".to_string()),
            },
            Some("019d607a-cf72-72e1-a5b7-0dc17ad019ae"),
            None,
        );
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            SelectionViewParams {
                title: Some("Clawbot".to_string()),
                subtitle: Some("Session Jump".to_string()),
                items: vec![item],
                initial_selected_idx: Some(0),
                ..Default::default()
            },
            tx,
        );

        assert_snapshot!(
            "bound_session_jump_item",
            render_selection_popup(&view, 92, 14)
        );
    }
}
