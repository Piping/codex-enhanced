use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use codex_clawbot::CLAWBOT_DIAGNOSTICS_RELATIVE_PATH;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::ClawbotStore;
use codex_clawbot::ClawbotTurnMode;
use codex_clawbot::ConnectionStatus;
use codex_clawbot::FeishuConfig;
use codex_clawbot::FeishuCoordinationConfig;
use codex_clawbot::ForwardingDirection;
use codex_clawbot::ForwardingState;
use codex_clawbot::ProviderKind;
use codex_clawbot::ProviderRuntimeState;
use codex_clawbot::ProviderSession;
use codex_clawbot::ProviderSessionRef;
use codex_clawbot::SessionBinding;
use codex_protocol::ThreadId;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::App;
use super::editor_helpers::ExternalEditorErrorTarget;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotControlsDestination;
use crate::app_event::ClawbotFeishuConfigField;
use crate::app_event::ClawbotForwardingChannel;
use crate::app_event_sender::AppEventSender;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionAction;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::render::renderable::ColumnRenderable;
use crate::tui;

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
            Self::CoordinationBaseToken => "Coordination Base Token",
            Self::CoordinationHeartbeatTableId => "Heartbeat Table ID",
            Self::CoordinationForceTableId => "Force Table ID",
            Self::CoordinationInstanceId => "Coordination Instance ID",
            Self::CoordinationOwnerPriority => "Coordination Priority",
            Self::CoordinationForceConnect => "Force Connect",
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
            Self::CoordinationBaseToken => config
                .and_then(|config| config.coordination.as_ref())
                .map(|coordination| coordination.base_token.clone()),
            Self::CoordinationHeartbeatTableId => config
                .and_then(|config| config.coordination.as_ref())
                .map(|coordination| coordination.heartbeat_table_id.clone()),
            Self::CoordinationForceTableId => config
                .and_then(|config| config.coordination.as_ref())
                .map(|coordination| coordination.force_table_id.clone()),
            Self::CoordinationInstanceId => config
                .and_then(|config| config.coordination.as_ref())
                .and_then(|coordination| coordination.instance_id.clone()),
            Self::CoordinationOwnerPriority => config
                .and_then(|config| config.coordination.as_ref())
                .map(|coordination| coordination.owner_priority.to_string()),
            Self::CoordinationForceConnect => config
                .and_then(|config| config.coordination.as_ref())
                .map(|coordination| coordination.force_connect.to_string()),
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
            Self::CoordinationBaseToken => {
                "Paste the Feishu Base token, or submit an empty value to clear it"
            }
            Self::CoordinationHeartbeatTableId => {
                "Paste the heartbeat table_id, or submit an empty value to clear it"
            }
            Self::CoordinationForceTableId => {
                "Paste the force table_id, or submit an empty value to clear it"
            }
            Self::CoordinationInstanceId => {
                "Paste an optional stable instance_id, or submit an empty value to auto-generate"
            }
            Self::CoordinationOwnerPriority => {
                "Paste the coordination priority integer, or submit an empty value for the default"
            }
            Self::CoordinationForceConnect => {
                "Enter true or false to control whether this Codex process force-preempts leadership"
            }
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
            Self::AppSecret
                | Self::VerificationToken
                | Self::EncryptKey
                | Self::CoordinationBaseToken
        )
    }
}

impl ClawbotForwardingChannel {
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
        self.open_clawbot_management_view(ClawbotControlsDestination::Root);
    }

    pub(crate) fn open_clawbot_management_view(&mut self, destination: ClawbotControlsDestination) {
        self.clawbot_controls_destination = destination.clone();
        self.open_selection_popup_for_view(
            CLAWBOT_MANAGEMENT_VIEW_ID,
            |app, active_selected_idx| {
                let initial_selected_idx = if active_selected_idx.is_some()
                    && app.clawbot_controls_destination == destination
                {
                    active_selected_idx
                } else {
                    Some(0)
                };
                app.clawbot_management_popup_params(&destination, initial_selected_idx)
            },
        );
    }

    fn refresh_clawbot_management_popup(&mut self) {
        self.open_clawbot_management_view(self.clawbot_controls_destination.clone());
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

    pub(crate) fn save_clawbot_feishu_config_value(
        &mut self,
        field: ClawbotFeishuConfigField,
        value: String,
    ) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        let mut config = runtime.snapshot().config.feishu.clone().unwrap_or_default();
        let mut coordination = config.coordination.clone().unwrap_or_default();
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
            ClawbotFeishuConfigField::CoordinationBaseToken => {
                coordination.base_token = trimmed;
            }
            ClawbotFeishuConfigField::CoordinationHeartbeatTableId => {
                coordination.heartbeat_table_id = trimmed;
            }
            ClawbotFeishuConfigField::CoordinationForceTableId => {
                coordination.force_table_id = trimmed;
            }
            ClawbotFeishuConfigField::CoordinationInstanceId => {
                coordination.instance_id = (!trimmed.is_empty()).then_some(trimmed);
            }
            ClawbotFeishuConfigField::CoordinationOwnerPriority => {
                coordination.owner_priority = if trimmed.is_empty() {
                    FeishuCoordinationConfig::default().owner_priority
                } else {
                    trimmed.parse().context("coordination priority must be an integer")?
                };
            }
            ClawbotFeishuConfigField::CoordinationForceConnect => {
                coordination.force_connect = match trimmed.to_ascii_lowercase().as_str() {
                    "" | "false" | "0" | "off" | "no" => false,
                    "true" | "1" | "on" | "yes" => true,
                    _ => {
                        return Err(anyhow::anyhow!(
                            "force connect must be one of: true, false, 1, 0, on, off, yes, no"
                        ));
                    }
                };
            }
        }
        config.coordination = (!coordination.is_empty()).then_some(coordination);
        runtime.update_feishu_config(Some(config))?;
        self.refresh_clawbot_provider_runtime()?;
        self.refresh_clawbot_management_popup();
        self.chat_widget
            .add_info_message(format!("Updated {}.", field.title()), /*hint*/ None);
        Ok(())
    }

    pub(crate) fn save_clawbot_turn_mode(&mut self, mode: ClawbotTurnMode) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        runtime.update_turn_mode(mode)?;
        self.refresh_clawbot_management_popup();
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
            if !runtime.can_bind_feishu_session(&session)? {
                return Err(anyhow::anyhow!(
                    "Feishu session {trimmed} is not visible to the current bot"
                ));
            }
        }
        runtime.connect_session_to_thread(
            &session,
            thread_id.to_string(),
            self.clawbot_owner_primary_thread_id(),
        )?;
        self.refresh_clawbot_provider_runtime()?;
        self.dispatch_next_clawbot_message(app_server, &session)
            .await?;
        self.refresh_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!("Bound thread {thread_id} to Feishu session {trimmed}."),
            /*hint*/ None,
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn clawbot_disconnect_current_thread(&mut self) -> Result<()> {
        let thread_id = self
            .active_thread_id
            .context("no active thread available for Clawbot disconnect")?;
        self.clawbot_disconnect_thread(thread_id)
    }

    #[cfg(test)]
    pub(crate) fn clawbot_set_current_thread_forwarding(
        &mut self,
        channel: ClawbotForwardingChannel,
        enabled: bool,
    ) -> Result<()> {
        let thread_id = self
            .active_thread_id
            .context("no active thread available for Clawbot forwarding")?;
        self.clawbot_set_thread_forwarding(thread_id, channel, enabled)
    }

    pub(crate) fn clawbot_disconnect_thread(&mut self, thread_id: ThreadId) -> Result<()> {
        let mut runtime = ClawbotRuntime::load(self.config.cwd.to_path_buf())?;
        let Some(session) = runtime.disconnect_thread(&thread_id.to_string())? else {
            return Err(anyhow::anyhow!(
                "thread {thread_id} is not bound to a Clawbot session"
            ));
        };
        self.refresh_clawbot_management_popup();
        self.chat_widget.add_info_message(
            format!(
                "Disconnected Feishu session {} from thread {thread_id}.",
                session.session_id
            ),
            /*hint*/ None,
        );
        Ok(())
    }

    pub(crate) fn clawbot_set_thread_forwarding(
        &mut self,
        thread_id: ThreadId,
        channel: ClawbotForwardingChannel,
        enabled: bool,
    ) -> Result<()> {
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
            .with_context(|| format!("thread {thread_id} is not bound to a Clawbot session"))?;
        self.refresh_clawbot_management_popup();
        self.chat_widget
            .add_info_message(channel.description(enabled), /*hint*/ None);
        Ok(())
    }

    pub(crate) async fn edit_clawbot_state_file_from_ui(
        &mut self,
        tui: &mut tui::Tui,
        label: &'static str,
        path: PathBuf,
    ) {
        if let Some(parent) = path.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            self.chat_widget
                .add_error_message(format!("Failed to prepare {label}: {err}"));
            return;
        }
        if let Err(err) = fs::OpenOptions::new().create(true).append(true).open(&path) {
            self.chat_widget
                .add_error_message(format!("Failed to prepare {label}: {err}"));
            return;
        }
        if self
            .edit_file_with_external_editor(
                tui,
                ExternalEditorErrorTarget::ErrorMessage,
                path.as_path(),
            )
            .await
            .is_ok()
        {
            self.refresh_clawbot_management_popup();
        }
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
        self.refresh_clawbot_management_popup();
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
        self.refresh_clawbot_management_popup();
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
        self.refresh_clawbot_management_popup();
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
        destination: &ClawbotControlsDestination,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let store = ClawbotStore::new(self.config.cwd.to_path_buf());
        let bindings_state_path = store.bindings_path();
        let sessions_state_path = store.sessions_path();
        let unread_queue_path = store.unread_messages_path();
        let runtime_log_path = store
            .workspace_root()
            .join(CLAWBOT_DIAGNOSTICS_RELATIVE_PATH);
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
        let mut feishu_bindings = snapshot
            .bindings
            .iter()
            .filter(|binding| binding.provider == ProviderKind::Feishu)
            .cloned()
            .collect::<Vec<_>>();
        feishu_bindings.sort_by(|left, right| {
            (active_thread_id.as_deref() == Some(left.thread_id.as_str()))
                .cmp(&(active_thread_id.as_deref() == Some(right.thread_id.as_str())))
                .reverse()
                .then(left.thread_id.cmp(&right.thread_id))
                .then(left.session_id.cmp(&right.session_id))
        });
        let bound_session_count = feishu_bindings.len();
        let unbound_sessions = feishu_sessions
            .iter()
            .filter(|session| session.bound_thread_id.is_none())
            .cloned()
            .collect::<Vec<_>>();
        let unbound_session_count = unbound_sessions.len();
        let has_api_credentials = feishu_config.is_some_and(FeishuConfig::has_api_credentials);

        let items = match destination {
            ClawbotControlsDestination::Root => vec![
                submenu_item(
                    "Channels",
                    format!(
                        "{bound_session_count} bound · {unbound_session_count} unbound · Feishu"
                    ),
                    "Scan Feishu sessions, inspect bindings, and manage channel state."
                        .to_string(),
                    ClawbotControlsDestination::Channels,
                ),
                submenu_item(
                    "Configuration",
                    clawbot_config_summary(feishu_config, turn_mode),
                    "Edit workspace-local Feishu credentials and clawbot turn mode.".to_string(),
                    ClawbotControlsDestination::Configuration,
                ),
                submenu_item(
                    "Diagnostics",
                    connection_description(&provider_state),
                    "Retry the bridge and open local clawbot state files.".to_string(),
                    ClawbotControlsDestination::Diagnostics,
                ),
            ],
            ClawbotControlsDestination::Channels => vec![
                clawbot_back_item(ClawbotControlsDestination::Root, "Return to the clawbot overview."),
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
                    is_disabled: !has_api_credentials,
                    actions: vec![Box::new(|tx| tx.send(AppEvent::ScanClawbotFeishuSessions))],
                    dismiss_on_select: false,
                    ..Default::default()
                },
                submenu_item(
                    "Bound Channels",
                    format!("{bound_session_count} active binding(s)"),
                    "Open a bound channel to jump threads or change forwarding.".to_string(),
                    ClawbotControlsDestination::BoundChannels,
                ),
                submenu_item(
                    "Unbound Sessions",
                    format!("{unbound_session_count} discovered"),
                    "Open an unbound session and bind it to the current Codex thread.".to_string(),
                    ClawbotControlsDestination::UnboundSessions,
                ),
                submenu_item(
                    "Cleanup",
                    format!("{clearable_unread_count} cached unread message(s) can be dropped"),
                    "Clear stale unbound session cache and unread queue entries.".to_string(),
                    ClawbotControlsDestination::Cleanup,
                ),
            ],
            ClawbotControlsDestination::BoundChannels => {
                let mut items = vec![clawbot_back_item(
                    ClawbotControlsDestination::Channels,
                    "Return to channel controls.",
                )];
                if feishu_bindings.is_empty() {
                    items.push(info_item(
                        "No active bindings".to_string(),
                        Some(
                            "Bind a discovered Feishu session to route unread messages into a Codex thread."
                                .to_string(),
                        ),
                    ));
                } else {
                    items.extend(feishu_bindings.iter().map(|binding| {
                        let thread_label =
                            ThreadId::from_string(&binding.thread_id).ok().map_or_else(
                                || binding.thread_id.clone(),
                                |thread_id| self.thread_label(thread_id),
                            );
                        let session = snapshot
                            .sessions
                            .iter()
                            .find(|session| session.session_ref() == binding.session_ref());
                        bound_channel_item(
                            binding,
                            thread_label,
                            session.map_or_else(|| binding.session_id.clone(), session_title),
                            session.map_or(0, |session| session.unread_count),
                        )
                    }));
                }
                items
            }
            ClawbotControlsDestination::BoundChannel { thread_id } => {
                let mut items = vec![clawbot_back_item(
                    ClawbotControlsDestination::BoundChannels,
                    "Return to bound channels.",
                )];
                let binding = feishu_bindings
                    .iter()
                    .find(|binding| binding.thread_id == *thread_id);
                match binding {
                    Some(binding) => {
                        let thread_label =
                            ThreadId::from_string(&binding.thread_id).ok().map_or_else(
                                || binding.thread_id.clone(),
                                |thread_id| self.thread_label(thread_id),
                            );
                        let session = snapshot
                            .sessions
                            .iter()
                            .find(|session| session.session_ref() == binding.session_ref());
                        items.extend(bound_channel_detail_items(
                            binding,
                            active_thread_id.as_deref(),
                            thread_label,
                            session.map_or_else(|| binding.session_id.clone(), session_title),
                            session.map_or(0, |session| session.unread_count),
                        ));
                    }
                    None => items.push(info_item(
                        "Binding not found".to_string(),
                        Some("This binding no longer exists.".to_string()),
                    )),
                }
                items
            }
            ClawbotControlsDestination::UnboundSessions => {
                let mut items = vec![clawbot_back_item(
                    ClawbotControlsDestination::Channels,
                    "Return to channel controls.",
                )];
                if unbound_sessions.is_empty() {
                    items.push(info_item(
                        "No unbound sessions".to_string(),
                        Some("No discovered Feishu sessions are waiting to be bound.".to_string()),
                    ));
                } else {
                    items.extend(
                        unbound_sessions
                            .iter()
                            .map(unbound_session_item),
                    );
                }
                items
            }
            ClawbotControlsDestination::UnboundSession { session_id } => {
                let mut items = vec![clawbot_back_item(
                    ClawbotControlsDestination::UnboundSessions,
                    "Return to unbound sessions.",
                )];
                match unbound_sessions
                    .iter()
                    .find(|session| session.session_id == *session_id)
                {
                    Some(session) => {
                        items.extend(unbound_session_detail_items(
                            session,
                            active_thread_id.as_deref(),
                        ));
                    }
                    None => items.push(info_item(
                        "Session not found".to_string(),
                        Some("This unbound session is no longer available.".to_string()),
                    )),
                }
                items
            }
            ClawbotControlsDestination::Cleanup => vec![
                clawbot_back_item(
                    ClawbotControlsDestination::Channels,
                    "Return to channel controls.",
                ),
                info_item(
                    "Unbound session cache".to_string(),
                    Some(format!(
                        "{unbound_session_count} session(s) · {clearable_unread_count} unread message(s)"
                    )),
                ),
                SelectionItem {
                    name: "Clear Unbound Sessions".to_string(),
                    description: Some(
                        "Remove unbound sessions and cached unread messages while keeping live bindings."
                            .to_string(),
                    ),
                    selected_description: Some(
                        "Keep active bindings intact while dropping stale discovered-session cache."
                            .to_string(),
                    ),
                    is_disabled: unbound_session_count == 0 && clearable_unread_count == 0,
                    actions: vec![Box::new(|tx| tx.send(AppEvent::ClearClawbotFeishuSessions))],
                    dismiss_on_select: false,
                    ..Default::default()
                },
            ],
            ClawbotControlsDestination::Configuration => vec![
                clawbot_back_item(
                    ClawbotControlsDestination::Root,
                    "Return to the clawbot overview.",
                ),
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
                clawbot_config_item(ClawbotFeishuConfigField::AppId, feishu_config),
                clawbot_config_item(ClawbotFeishuConfigField::AppSecret, feishu_config),
                clawbot_config_item(ClawbotFeishuConfigField::VerificationToken, feishu_config),
                clawbot_config_item(ClawbotFeishuConfigField::EncryptKey, feishu_config),
                clawbot_config_item(ClawbotFeishuConfigField::BotOpenId, feishu_config),
                clawbot_config_item(ClawbotFeishuConfigField::BotUserId, feishu_config),
                clawbot_config_item(
                    ClawbotFeishuConfigField::CoordinationBaseToken,
                    feishu_config,
                ),
                clawbot_config_item(
                    ClawbotFeishuConfigField::CoordinationHeartbeatTableId,
                    feishu_config,
                ),
                clawbot_config_item(
                    ClawbotFeishuConfigField::CoordinationForceTableId,
                    feishu_config,
                ),
                clawbot_config_item(
                    ClawbotFeishuConfigField::CoordinationInstanceId,
                    feishu_config,
                ),
                clawbot_config_item(
                    ClawbotFeishuConfigField::CoordinationOwnerPriority,
                    feishu_config,
                ),
                clawbot_config_item(
                    ClawbotFeishuConfigField::CoordinationForceConnect,
                    feishu_config,
                ),
            ],
            ClawbotControlsDestination::Diagnostics => vec![
                clawbot_back_item(
                    ClawbotControlsDestination::Root,
                    "Return to the clawbot overview.",
                ),
                SelectionItem {
                    name: "Retry Feishu Connection".to_string(),
                    description: Some(connection_description(&provider_state)),
                    selected_description: Some(
                        "Restart the workspace-local Feishu websocket/runtime bridge.".to_string(),
                    ),
                    is_disabled: !has_api_credentials,
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::RetryClawbotFeishuConnection)
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                },
                clawbot_state_file_item("Open Runtime Log", runtime_log_path, "runtime log"),
                clawbot_state_file_item(
                    "Open Bindings State",
                    bindings_state_path,
                    "bindings state file",
                ),
                clawbot_state_file_item(
                    "Open Sessions State",
                    sessions_state_path,
                    "sessions state file",
                ),
                clawbot_state_file_item(
                    "Open Unread Queue",
                    unread_queue_path,
                    "unread queue file",
                ),
            ],
        };

        SelectionViewParams {
            view_id: Some(CLAWBOT_MANAGEMENT_VIEW_ID),
            header: Box::new(clawbot_management_header(
                bound_session_count,
                &provider_state,
                turn_mode,
            )),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx: initial_selected_idx.or(Some(0)),
            ..Default::default()
        }
    }
}

fn clawbot_management_header(
    active_bindings: usize,
    provider_state: &ProviderRuntimeState,
    turn_mode: ClawbotTurnMode,
) -> ColumnRenderable<'static> {
    let mut header = ColumnRenderable::new();
    header.push(Line::from("Clawbot".bold()));
    header.push(Line::from(format!("active bindings: {active_bindings}")));
    header.push(Line::from(format!(
        "ws status: {}",
        connection_description(provider_state)
    )));
    header.push(Line::from(format!(
        "turn mode: {}",
        clawbot_turn_mode_label(turn_mode)
    )));
    header
}

fn submenu_item(
    name: &str,
    description: String,
    selected_description: String,
    destination: ClawbotControlsDestination,
) -> SelectionItem {
    SelectionItem {
        name: name.to_string(),
        description: Some(description),
        selected_description: Some(selected_description),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenClawbotManagementView {
                destination: destination.clone(),
            });
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn clawbot_back_item(
    destination: ClawbotControlsDestination,
    selected_description: &str,
) -> SelectionItem {
    SelectionItem {
        name: "Back".to_string(),
        description: Some("Return to the previous clawbot menu.".to_string()),
        selected_description: Some(selected_description.to_string()),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenClawbotManagementView {
                destination: destination.clone(),
            });
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn info_item(name: String, description: Option<String>) -> SelectionItem {
    SelectionItem {
        name,
        selected_description: description,
        is_disabled: true,
        ..Default::default()
    }
}

fn clawbot_state_file_item(name: &str, path: PathBuf, label: &'static str) -> SelectionItem {
    let display_path = path.display().to_string();
    SelectionItem {
        name: name.to_string(),
        description: Some(display_path.clone()),
        selected_description: Some(format!("Open {display_path} in your external editor.")),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::EditClawbotStateFile {
                label,
                path: path.clone(),
            });
        })],
        dismiss_on_select: false,
        ..Default::default()
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

fn clawbot_config_summary(config: Option<&FeishuConfig>, turn_mode: ClawbotTurnMode) -> String {
    let configured = [
        ClawbotFeishuConfigField::AppId,
        ClawbotFeishuConfigField::AppSecret,
        ClawbotFeishuConfigField::VerificationToken,
        ClawbotFeishuConfigField::EncryptKey,
        ClawbotFeishuConfigField::BotOpenId,
        ClawbotFeishuConfigField::BotUserId,
        ClawbotFeishuConfigField::CoordinationBaseToken,
        ClawbotFeishuConfigField::CoordinationHeartbeatTableId,
        ClawbotFeishuConfigField::CoordinationForceTableId,
        ClawbotFeishuConfigField::CoordinationInstanceId,
        ClawbotFeishuConfigField::CoordinationOwnerPriority,
        ClawbotFeishuConfigField::CoordinationForceConnect,
    ]
    .into_iter()
    .filter(|field| field.current_value(config).is_some())
    .count();
    format!(
        "{configured}/12 configured · {}",
        clawbot_turn_mode_label(turn_mode)
    )
}

fn bound_channel_item(
    binding: &SessionBinding,
    thread_label: String,
    session_title: String,
    unread_count: usize,
) -> SelectionItem {
    let thread_id = binding.thread_id.clone();
    SelectionItem {
        name: format!("{thread_label} ↔ {session_title}"),
        description: Some(format!(
            "{} unread · inbound {} · outbound {}",
            unread_count,
            forwarding_state_label(binding.inbound_forwarding_enabled),
            forwarding_state_label(binding.outbound_forwarding_enabled),
        )),
        selected_description: Some(
            "Open this binding to jump threads or change forwarding state.".to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenClawbotManagementView {
                destination: ClawbotControlsDestination::BoundChannel {
                    thread_id: thread_id.clone(),
                },
            });
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn bound_channel_detail_items(
    binding: &SessionBinding,
    active_thread_id: Option<&str>,
    thread_label: String,
    session_title: String,
    unread_count: usize,
) -> Vec<SelectionItem> {
    let jump_target = ThreadId::from_string(&binding.thread_id).ok();
    let toggle_inbound_enabled = !binding.inbound_forwarding_enabled;
    let toggle_outbound_enabled = !binding.outbound_forwarding_enabled;
    let mut items = vec![
        info_item(
            "Thread".to_string(),
            Some(format!("{thread_label} ({})", binding.thread_id)),
        ),
        info_item("Session".to_string(), Some(session_title)),
        info_item("Session ID".to_string(), Some(binding.session_id.clone())),
        info_item("Unread".to_string(), Some(unread_count.to_string())),
        info_item(
            "Inbound Forwarding".to_string(),
            Some(forwarding_state_label(binding.inbound_forwarding_enabled).to_string()),
        ),
        info_item(
            "Outbound Forwarding".to_string(),
            Some(forwarding_state_label(binding.outbound_forwarding_enabled).to_string()),
        ),
    ];
    let jump_item = SelectionItem {
        name: "Jump to thread".to_string(),
        description: Some(if active_thread_id == Some(binding.thread_id.as_str()) {
            "Already viewing this thread.".to_string()
        } else {
            "Switch the TUI to this bound thread.".to_string()
        }),
        selected_description: Some(if active_thread_id == Some(binding.thread_id.as_str()) {
            format!(
                "Thread {} is already the active Codex thread.",
                binding.thread_id
            )
        } else {
            format!("Jump to thread {}.", binding.thread_id)
        }),
        is_disabled: active_thread_id == Some(binding.thread_id.as_str()) || jump_target.is_none(),
        actions: jump_target
            .map(|thread_id| {
                vec![Box::new(move |tx: &AppEventSender| {
                    tx.send(AppEvent::SelectAgentThread(thread_id));
                }) as SelectionAction]
            })
            .unwrap_or_default(),
        dismiss_on_select: false,
        ..Default::default()
    };
    items.push(jump_item);
    if let Some(thread_id) = jump_target {
        let inbound_thread_id = thread_id;
        let outbound_thread_id = thread_id;
        let disconnect_thread_id = thread_id;
        items.extend([
            SelectionItem {
                name: "Toggle inbound forwarding".to_string(),
                description: Some(
                    ClawbotForwardingChannel::Inbound
                        .description(binding.inbound_forwarding_enabled),
                ),
                selected_description: Some(
                    ClawbotForwardingChannel::Inbound
                        .selected_description(binding.inbound_forwarding_enabled),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetThreadForwarding {
                        thread_id: inbound_thread_id,
                        channel: ClawbotForwardingChannel::Inbound,
                        enabled: toggle_inbound_enabled,
                    });
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Toggle outbound forwarding".to_string(),
                description: Some(
                    ClawbotForwardingChannel::Outbound
                        .description(binding.outbound_forwarding_enabled),
                ),
                selected_description: Some(
                    ClawbotForwardingChannel::Outbound
                        .selected_description(binding.outbound_forwarding_enabled),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetThreadForwarding {
                        thread_id: outbound_thread_id,
                        channel: ClawbotForwardingChannel::Outbound,
                        enabled: toggle_outbound_enabled,
                    });
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Disconnect binding".to_string(),
                description: Some("Unbind this thread from the Feishu session.".to_string()),
                selected_description: Some(
                    "Remove this Feishu session binding without deleting cached session state."
                        .to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotDisconnectThread {
                        thread_id: disconnect_thread_id,
                    });
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
        ]);
    }
    items
}

fn unbound_session_item(session: &ProviderSession) -> SelectionItem {
    let session_id = session.session_id.clone();
    SelectionItem {
        name: session_title(session),
        description: Some(format!("{} unread", session.unread_count)),
        selected_description: Some(
            "Open this session to inspect it and bind it to the current thread.".to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenClawbotManagementView {
                destination: ClawbotControlsDestination::UnboundSession {
                    session_id: session_id.clone(),
                },
            });
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn unbound_session_detail_items(
    session: &ProviderSession,
    active_thread_id: Option<&str>,
) -> Vec<SelectionItem> {
    let session_id = session.session_id.clone();
    vec![
        info_item("Session".to_string(), Some(session_title(session))),
        info_item("Session ID".to_string(), Some(session.session_id.clone())),
        info_item("Unread".to_string(), Some(session.unread_count.to_string())),
        SelectionItem {
            name: "Bind to current thread".to_string(),
            description: Some(match active_thread_id {
                Some(thread_id) => format!("Target thread {thread_id}"),
                None => "No active thread".to_string(),
            }),
            selected_description: Some(match active_thread_id {
                Some(thread_id) => format!(
                    "Bind current thread {thread_id} directly to Feishu session {}.",
                    session.session_id
                ),
                None => format!(
                    "Open or switch to a Codex thread before binding Feishu session {}.",
                    session.session_id
                ),
            }),
            is_disabled: active_thread_id.is_none(),
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::SaveClawbotManualBindSessionId {
                    session_id: session_id.clone(),
                });
            })],
            dismiss_on_select: false,
            ..Default::default()
        },
    ]
}

fn forwarding_state_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

#[cfg(test)]
fn clawbot_session_item(
    session: &ProviderSession,
    active_thread_id: Option<&str>,
    bound_thread_label: Option<String>,
) -> SelectionItem {
    let description = if let Some(bound_thread_label) = bound_thread_label.as_deref() {
        format!(
            "bound to {bound_thread_label} · {} unread",
            session.unread_count
        )
    } else {
        format!("unbound · {} unread", session.unread_count)
    };
    let selected_description = match (active_thread_id, session.bound_thread_id.as_deref()) {
        (Some(thread_id), Some(bound_thread_id)) if thread_id == bound_thread_id => {
            format!(
                "Current thread {thread_id} is already bound to Feishu session {}.",
                session.session_id
            )
        }
        (_, Some(bound_thread_id)) => format!(
            "Jump to bound thread {bound_thread_id} to continue or manage Feishu session {}.",
            session.session_id
        ),
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
    let jump_target = session
        .bound_thread_id
        .as_deref()
        .and_then(|thread_id| ThreadId::from_string(thread_id).ok());
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
    SelectionItem {
        name: session_title(session),
        description: Some(description),
        selected_description: Some(selected_description),
        is_disabled: match session.bound_thread_id.as_deref() {
            Some(bound_thread_id) => {
                active_thread_id == Some(bound_thread_id) || jump_target.is_none()
            }
            None => active_thread_id.is_none(),
        },
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
    use crate::bottom_pane::SelectionViewParams;
    use crate::bottom_pane::list_selection_view::ListSelectionView;
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
            Some("Inbox Agent".to_string()),
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
            Some("Inbox Agent".to_string()),
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
            render_selection_popup(&view, /*width*/ 92, /*height*/ 14)
        );
    }
}
