mod sessions;
pub(super) mod turns;

use anyhow::Result;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::ClawbotStore;
use codex_clawbot::ClawbotTurnMode;
use codex_clawbot::ConnectionStatus;
use codex_clawbot::FeishuConfig;
use codex_clawbot::ProviderEvent;
use codex_clawbot::ProviderKind;
use codex_clawbot::ProviderMessageRef;
use codex_clawbot::ProviderOutboundAction;
use codex_clawbot::ProviderOutboundReaction;
use codex_clawbot::ProviderOutboundTextMessage;
use codex_clawbot::ProviderReactionReceipt;
use codex_clawbot::ProviderRuntime;
use codex_clawbot::ProviderRuntimeState;
use codex_clawbot::ProviderSession;
use codex_clawbot::ProviderSessionRef;
use codex_clawbot::SessionForwardingDirection;
use codex_clawbot::SessionForwardingMode;
use codex_clawbot::feishu_failure_reply_text;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::GranularApprovalConfig;
use codex_protocol::protocol::Op;
use codex_protocol::request_permissions::PermissionGrantScope;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputResponse;
use std::sync::Arc;
use tokio::sync::mpsc;

use self::sessions::CLAWBOT_SESSIONS_PANEL_VIEW_ID;
use self::sessions::feishu_sessions_menu_description;
use self::turns::FEISHU_AUTO_ACK_DISPLAY;
use self::turns::FEISHU_AUTO_ACK_EMOJI_TYPE;
use self::turns::PendingClawbotTurn;
use super::App;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotFeishuConfigField;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell;
use crate::history_cell::HistoryCell;

const CLAWBOT_PANEL_VIEW_ID: &str = "fork-clawbot-panel";
const CLAWBOT_CONFIG_PANEL_VIEW_ID: &str = "fork-clawbot-config-panel";
const CLAWBOT_SESSION_ACTIONS_VIEW_ID: &str = "fork-clawbot-session-actions-panel";

#[derive(Clone, Copy)]
enum ClawbotDrainReason {
    Automatic,
    Manual,
}

pub(crate) fn control_panel_clawbot_item() -> SelectionItem {
    SelectionItem {
        name: "Clawbot".to_string(),
        description: None,
        selected_description: Some(
            "Inspect the workspace-local IM gateway and session bindings.".to_string(),
        ),
        actions: vec![Box::new(|tx| tx.send(AppEvent::OpenClawbotPanel))],
        dismiss_on_select: false,
        ..Default::default()
    }
}

impl ClawbotFeishuConfigField {
    fn title(self) -> &'static str {
        match self {
            Self::AppId => "App ID",
            Self::AppSecret => "App Secret",
            Self::VerificationToken => "Verification Token",
            Self::EncryptKey => "Encrypt Key",
            Self::BotOpenId => "Bot Open ID",
            Self::BotUserId => "Bot User ID",
        }
    }

    pub(crate) fn prompt_title(self) -> String {
        format!("Edit Feishu {}", self.title())
    }

    pub(crate) fn prompt_placeholder(self) -> String {
        match self {
            Self::AppId => "Paste the Feishu app_id and press Enter".to_string(),
            Self::AppSecret => "Paste the Feishu app_secret and press Enter".to_string(),
            Self::VerificationToken => "Paste the verification token and press Enter".to_string(),
            Self::EncryptKey => "Paste the encrypt key and press Enter".to_string(),
            Self::BotOpenId => "Paste the bot open_id and press Enter".to_string(),
            Self::BotUserId => "Paste the bot user_id and press Enter".to_string(),
        }
    }

    pub(crate) fn prompt_context_label(self) -> String {
        let scope = "Workspace-local clawbot config";
        match self {
            Self::AppId | Self::AppSecret => format!("{scope} · Required for API and websocket"),
            Self::VerificationToken | Self::EncryptKey => {
                format!("{scope} · Optional for webhook verification")
            }
            Self::BotOpenId | Self::BotUserId => {
                format!("{scope} · Optional bot identity metadata")
            }
        }
    }

    fn selected_description(self) -> String {
        match self {
            Self::AppId | Self::AppSecret => {
                "Edit this required Feishu credential and persist it under .codex/clawbot."
                    .to_string()
            }
            Self::VerificationToken | Self::EncryptKey => {
                "Edit this optional Feishu gateway setting and persist it under .codex/clawbot."
                    .to_string()
            }
            Self::BotOpenId | Self::BotUserId => {
                "Edit this optional bot identity field and persist it under .codex/clawbot."
                    .to_string()
            }
        }
    }

    fn current_value(self, config: &FeishuConfig) -> String {
        match self {
            Self::AppId => config.app_id.clone(),
            Self::AppSecret => config.app_secret.clone(),
            Self::VerificationToken => config.verification_token.clone().unwrap_or_default(),
            Self::EncryptKey => config.encrypt_key.clone().unwrap_or_default(),
            Self::BotOpenId => config.bot_open_id.clone().unwrap_or_default(),
            Self::BotUserId => config.bot_user_id.clone().unwrap_or_default(),
        }
    }

    fn value_description(self, config: Option<&FeishuConfig>) -> String {
        let value = config
            .map(|config| self.current_value(config))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        match value {
            Some(value) if self.is_secret() => format!("Configured · {}", mask_secret(&value)),
            Some(value) => {
                format!("Configured · {}", truncate_value(&value, /*max_chars*/ 28))
            }
            None => "Not set".to_string(),
        }
    }

    fn is_secret(self) -> bool {
        matches!(
            self,
            Self::AppSecret | Self::VerificationToken | Self::EncryptKey
        )
    }
}

impl App {
    pub(crate) fn open_clawbot_panel(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_PANEL_VIEW_ID);
        if !self.chat_widget.replace_selection_view_if_active(
            CLAWBOT_PANEL_VIEW_ID,
            self.clawbot_panel_params(initial_selected_idx),
        ) {
            self.chat_widget
                .show_selection_view(self.clawbot_panel_params(initial_selected_idx));
        }
    }

    pub(crate) fn open_clawbot_session_actions(&mut self, session: ProviderSessionRef) {
        self.open_clawbot_session_actions_with_selection(
            session, /*initial_selected_idx*/ None,
        );
    }

    fn open_clawbot_session_actions_with_selection(
        &mut self,
        session: ProviderSessionRef,
        initial_selected_idx: Option<usize>,
    ) {
        let initial_selected_idx = initial_selected_idx.or_else(|| {
            self.chat_widget
                .selected_index_for_active_view(CLAWBOT_SESSION_ACTIONS_VIEW_ID)
        });
        if !self.chat_widget.replace_selection_view_if_active(
            CLAWBOT_SESSION_ACTIONS_VIEW_ID,
            self.clawbot_session_actions_panel_params(session.clone(), initial_selected_idx),
        ) {
            self.chat_widget.show_selection_view(
                self.clawbot_session_actions_panel_params(session, initial_selected_idx),
            );
        }
    }

    pub(crate) fn open_clawbot_config_panel(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_CONFIG_PANEL_VIEW_ID);
        if !self.chat_widget.replace_selection_view_if_active(
            CLAWBOT_CONFIG_PANEL_VIEW_ID,
            self.clawbot_config_panel_params(initial_selected_idx),
        ) {
            self.chat_widget
                .show_selection_view(self.clawbot_config_panel_params(initial_selected_idx));
        }
    }

    pub(crate) fn open_clawbot_feishu_config_prompt(
        &mut self,
        field: ClawbotFeishuConfigField,
    ) -> Result<()> {
        let config = self
            .clawbot_runtime()?
            .snapshot()
            .config
            .feishu
            .clone()
            .unwrap_or_default();
        self.chat_widget
            .open_clawbot_feishu_config_prompt(field, field.current_value(&config));
        Ok(())
    }

    pub(crate) fn open_clawbot_manual_bind_prompt(&mut self) {
        let current_thread_label = self
            .active_thread_id
            .map(|thread_id| thread_id.to_string())
            .unwrap_or_else(|| "No active thread".to_string());
        let current_value = self
            .active_thread_id
            .and_then(|thread_id| {
                self.clawbot_runtime()
                    .ok()
                    .and_then(|runtime| {
                        runtime
                            .bound_session_for_thread(&thread_id.to_string())
                            .ok()
                    })
                    .flatten()
                    .map(|session| session.session_id)
            })
            .unwrap_or_default();
        self.chat_widget
            .open_clawbot_manual_bind_prompt(current_value, current_thread_label);
    }

    pub(crate) async fn clawbot_connect_current_thread(
        &mut self,
        session: ProviderSessionRef,
    ) -> Result<()> {
        let Some(thread_id) = self.active_thread_id else {
            self.open_clawbot_sessions_panel();
            return Ok(());
        };

        let mut runtime = self.clawbot_runtime()?;
        runtime.connect_session_to_thread(&session, thread_id.to_string())?;
        self.drain_clawbot_cached_messages_to_thread(session, ClawbotDrainReason::Manual)
            .await?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(crate) fn clawbot_disconnect(&mut self, session: ProviderSessionRef) -> Result<()> {
        let mut runtime = self.clawbot_runtime()?;
        runtime.disconnect_session(&session)?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(crate) fn clawbot_set_session_forwarding(
        &mut self,
        session: ProviderSessionRef,
        mode: SessionForwardingMode,
    ) -> Result<()> {
        let mut runtime = self.clawbot_runtime()?;
        runtime.set_session_forwarding(&session, mode)?;
        let selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_SESSION_ACTIONS_VIEW_ID);
        self.open_clawbot_session_actions_with_selection(session, selected_idx);
        self.refresh_clawbot_views_if_active();
        Ok(())
    }

    pub(crate) async fn save_clawbot_manual_bind_session_id(
        &mut self,
        session_id: String,
    ) -> Result<()> {
        let Some(thread_id) = self.active_thread_id else {
            self.open_clawbot_sessions_panel();
            return Ok(());
        };

        let trimmed = session_id.trim().to_string();
        if trimmed.is_empty() {
            self.open_clawbot_sessions_panel();
            return Ok(());
        }

        let mut runtime = self.clawbot_runtime()?;
        let session = ProviderSessionRef::new(ProviderKind::Feishu, trimmed);
        runtime.connect_session_to_thread(&session, thread_id.to_string())?;
        self.drain_clawbot_cached_messages_to_thread(session, ClawbotDrainReason::Manual)
            .await?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(crate) fn save_clawbot_feishu_config_value(
        &mut self,
        field: ClawbotFeishuConfigField,
        value: String,
    ) -> Result<()> {
        let mut runtime = self.clawbot_runtime()?;
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
                config.verification_token = Some(trimmed);
            }
            ClawbotFeishuConfigField::EncryptKey => {
                config.encrypt_key = Some(trimmed);
            }
            ClawbotFeishuConfigField::BotOpenId => {
                config.bot_open_id = Some(trimmed);
            }
            ClawbotFeishuConfigField::BotUserId => {
                config.bot_user_id = Some(trimmed);
            }
        }

        let next_config = (!config.is_empty()).then_some(config.clone());
        runtime.update_feishu_config(next_config)?;
        runtime.persist_runtime_state(feishu_runtime_state_for_config(config))?;
        self.open_clawbot_config_panel();
        Ok(())
    }

    pub(crate) fn save_clawbot_turn_mode(&mut self, mode: ClawbotTurnMode) -> Result<()> {
        let mut runtime = self.clawbot_runtime()?;
        runtime.update_turn_mode(mode)?;
        self.open_clawbot_panel();
        Ok(())
    }

    pub(crate) async fn clawbot_flush_cached_messages(
        &mut self,
        session: ProviderSessionRef,
    ) -> Result<()> {
        self.drain_clawbot_cached_messages_to_thread(session, ClawbotDrainReason::Manual)
            .await?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(crate) async fn bootstrap_clawbot_runtime(&mut self) -> Result<()> {
        self.start_clawbot_provider_runtime(ProviderKind::Feishu)?;

        let store = ClawbotStore::new(self.config.cwd.clone());
        let snapshot = store.load_snapshot()?;
        let sessions_to_flush = snapshot
            .sessions
            .iter()
            .filter(|session| session.provider == ProviderKind::Feishu)
            .filter(|session| session.bound_thread_id.is_some())
            .filter(|session| session.unread_count > 0)
            .map(ProviderSession::session_ref)
            .collect::<Vec<_>>();
        for session in sessions_to_flush {
            self.drain_clawbot_cached_messages_to_thread(session, ClawbotDrainReason::Automatic)
                .await?;
        }
        Ok(())
    }

    async fn drain_clawbot_cached_messages_to_thread(
        &mut self,
        session: ProviderSessionRef,
        reason: ClawbotDrainReason,
    ) -> Result<()> {
        let store = ClawbotStore::new(self.config.cwd.clone());
        let snapshot = store.load_snapshot()?;
        let Some(binding) = snapshot
            .bindings
            .iter()
            .find(|binding| binding.session_ref() == session)
        else {
            let mut runtime = self.clawbot_runtime()?;
            let _flushed = runtime.flush_cached_messages(&session)?;
            self.refresh_clawbot_views_if_active();
            return Ok(());
        };

        if matches!(reason, ClawbotDrainReason::Automatic) && !binding.inbound_forwarding_enabled {
            return Ok(());
        }

        let thread_id = ThreadId::from_string(&binding.thread_id).map_err(|error| {
            anyhow::anyhow!("invalid bound thread id `{}`: {error}", binding.thread_id)
        })?;
        let cached_messages = store
            .load_unread_messages()?
            .into_iter()
            .filter(|message| message.session_ref() == session)
            .collect::<Vec<_>>();
        let session_label = snapshot
            .sessions
            .iter()
            .find(|candidate| candidate.session_ref() == session)
            .and_then(|candidate| candidate.display_name.clone())
            .unwrap_or_else(|| session.session_id.clone());
        let turn_mode = snapshot.config.turn_mode;

        for cached_message in &cached_messages {
            let source_message = ProviderMessageRef::new(
                cached_message.provider,
                cached_message.session_id.clone(),
                cached_message.message_id.clone(),
            );
            if let Err(err) = self.send_clawbot_auto_ack(source_message, thread_id).await {
                self.chat_widget.add_error_message(format!(
                    "Failed to add clawbot auto reaction for thread {thread_id}: {err}"
                ));
            }
            self.insert_clawbot_origin_info_cell(thread_id, &session_label);
            let turn_id = self
                .submit_clawbot_message_to_thread(thread_id, cached_message.text.clone(), turn_mode)
                .await?;
            self.register_pending_clawbot_turn(PendingClawbotTurn {
                turn_id,
                thread_id,
                turn_mode,
            });
        }

        let mut runtime = self.clawbot_runtime()?;
        let _flushed = runtime.flush_cached_messages(&session)?;
        self.refresh_clawbot_views_if_active();
        Ok(())
    }

    pub(crate) async fn clawbot_retry_connection(&mut self, provider: ProviderKind) -> Result<()> {
        self.start_clawbot_provider_runtime(provider)?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(crate) async fn handle_clawbot_provider_event(
        &mut self,
        event: ProviderEvent,
    ) -> Result<()> {
        let inbound_session = match &event {
            ProviderEvent::InboundMessage(message) => Some(message.session.clone()),
            _ => None,
        };

        let mut runtime = self.clawbot_runtime()?;
        runtime.apply_provider_event(event)?;
        let session_to_flush = match inbound_session {
            Some(session) => runtime
                .load_binding_for_session(&session)?
                .filter(|binding| binding.inbound_forwarding_enabled)
                .map(|_| session),
            None => None,
        };

        self.refresh_clawbot_views_if_active();
        if let Some(session) = session_to_flush {
            self.drain_clawbot_cached_messages_to_thread(session, ClawbotDrainReason::Automatic)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn maybe_auto_respond_to_clawbot_interactive_event(
        &mut self,
        thread_id: ThreadId,
        event: &EventMsg,
    ) -> Result<bool> {
        match event {
            EventMsg::RequestUserInput(request) => {
                self.auto_respond_to_clawbot_user_input_request(thread_id, &request.turn_id)
                    .await
            }
            EventMsg::RequestPermissions(request) => {
                self.auto_respond_to_clawbot_permissions_request(
                    thread_id,
                    &request.turn_id,
                    &request.call_id,
                )
                .await
            }
            _ => Ok(false),
        }
    }

    pub(crate) async fn handle_clawbot_thread_terminal_event(
        &mut self,
        thread_id: ThreadId,
        event: &EventMsg,
    ) {
        let _pending_turn = self.take_pending_clawbot_turn(thread_id, event);

        let outbound_text = match event {
            EventMsg::TurnComplete(turn_complete) => turn_complete
                .last_agent_message
                .as_deref()
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(ToOwned::to_owned),
            EventMsg::Error(error) => Some(feishu_failure_reply_text(&error.message)),
            _ => None,
        };

        let Some(text) = outbound_text else {
            return;
        };

        if let Err(err) = self.send_clawbot_thread_reply(thread_id, text).await {
            self.chat_widget.add_error_message(format!(
                "Failed to forward clawbot reply for thread {thread_id}: {err}"
            ));
        }
    }

    fn clawbot_panel_params(&self, initial_selected_idx: Option<usize>) -> SelectionViewParams {
        let store = ClawbotStore::new(self.config.cwd.clone());
        let snapshot = store.load_snapshot().unwrap_or_default();
        let provider_state = snapshot.provider_state(ProviderKind::Feishu);
        let config_description = feishu_config_summary(snapshot.config.feishu.as_ref());
        let turn_mode = snapshot.config.turn_mode;
        let next_turn_mode = match turn_mode {
            ClawbotTurnMode::Interactive => ClawbotTurnMode::NonInteractive,
            ClawbotTurnMode::NonInteractive => ClawbotTurnMode::Interactive,
        };
        let items = vec![
            SelectionItem {
                name: "Turn Mode".to_string(),
                description: Some(clawbot_turn_mode_summary(turn_mode)),
                selected_description: Some(match turn_mode {
                    ClawbotTurnMode::Interactive => {
                        "Switch clawbot-originated turns into non-interactive mode so question, permission, and approval prompts do not block remote sessions.".to_string()
                    }
                    ClawbotTurnMode::NonInteractive => {
                        "Restore normal interactive prompts for clawbot-originated turns.".to_string()
                    }
                }),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetTurnMode {
                        mode: next_turn_mode,
                    })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Sessions".to_string(),
                description: Some(feishu_sessions_menu_description(
                    provider_state,
                    &snapshot.sessions,
                )),
                selected_description: Some(
                    "Inspect Feishu session status and run scan / clear operations."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenClawbotSessionsPanel))],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Configuration".to_string(),
                description: Some(config_description),
                selected_description: Some(
                    "Edit and persist workspace-local Feishu credentials.".to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenClawbotConfigPanel))],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Bindings".to_string(),
                description: Some(format!(
                    "{} session-thread bindings persisted.",
                    snapshot.bindings.len()
                )),
                selected_description: Some(
                    "Each binding maps one external IM session to one Codex thread."
                        .to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Unread Cache".to_string(),
                description: Some(format!(
                    "{} cached inbound messages awaiting binding or replay.",
                    snapshot.unread_message_count
                )),
                selected_description: Some(
                    "Unbound sessions accumulate unread messages here until the operator connects them."
                        .to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            },
        ];
        let discovered_session_count = snapshot.sessions.len();

        SelectionViewParams {
            view_id: Some(CLAWBOT_PANEL_VIEW_ID),
            title: Some("Clawbot".to_string()),
            subtitle: Some(format!(
                "Feishu private chat bridge · {discovered_session_count} sessions discovered."
            )),
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(store.root_dir().display().to_string()),
            initial_selected_idx,
            items,
            ..Default::default()
        }
    }

    fn clawbot_config_panel_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let store = ClawbotStore::new(self.config.cwd.clone());
        let snapshot = store.load_snapshot().unwrap_or_default();
        let config = snapshot.config.feishu.unwrap_or_default();
        let items = [
            ClawbotFeishuConfigField::AppId,
            ClawbotFeishuConfigField::AppSecret,
            ClawbotFeishuConfigField::VerificationToken,
            ClawbotFeishuConfigField::EncryptKey,
            ClawbotFeishuConfigField::BotOpenId,
            ClawbotFeishuConfigField::BotUserId,
        ]
        .into_iter()
        .map(|field| SelectionItem {
            name: field.title().to_string(),
            description: Some(field.value_description(Some(&config))),
            selected_description: Some(field.selected_description()),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenClawbotFeishuConfigPrompt { field })
            })],
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();

        SelectionViewParams {
            view_id: Some(CLAWBOT_CONFIG_PANEL_VIEW_ID),
            title: Some("Clawbot".to_string()),
            subtitle: Some("Feishu Configuration".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(store.config_path().display().to_string()),
            initial_selected_idx,
            items,
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenClawbotPanel))),
            ..Default::default()
        }
    }

    fn clawbot_session_actions_panel_params(
        &self,
        session: ProviderSessionRef,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let store = ClawbotStore::new(self.config.cwd.clone());
        let snapshot = store.load_snapshot().unwrap_or_default();
        let selected_session = snapshot
            .sessions
            .iter()
            .find(|candidate| candidate.session_ref() == session)
            .cloned();
        let title = selected_session
            .as_ref()
            .and_then(|selected_session| selected_session.display_name.clone())
            .unwrap_or_else(|| session.session_id.clone());

        let items = match selected_session {
            Some(selected_session) => self.clawbot_session_action_items(selected_session),
            None => vec![SelectionItem {
                name: "Session not found".to_string(),
                description: Some(
                    "The persisted session disappeared before the actions panel opened."
                        .to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            }],
        };

        SelectionViewParams {
            view_id: Some(CLAWBOT_SESSION_ACTIONS_VIEW_ID),
            title: Some("Clawbot".to_string()),
            subtitle: Some(format!("Session Actions · {title}")),
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(store.root_dir().display().to_string()),
            initial_selected_idx,
            items,
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenClawbotSessionsPanel))),
            ..Default::default()
        }
    }

    fn clawbot_session_action_items(&self, session: ProviderSession) -> Vec<SelectionItem> {
        let session_ref = session.session_ref();
        if session.bound_thread_id.is_none() {
            let session_for_action = session_ref;
            return vec![SelectionItem {
                name: "Connect To Current Thread".to_string(),
                description: Some(format!(
                    "Bind this session to the current thread and persist the mapping. Current unread: {}.",
                    session.unread_count
                )),
                selected_description: Some(
                    "Future inbound messages for this session will route to the current thread."
                        .to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotConnectCurrentThread {
                        session: session_for_action.clone(),
                    })
                })],
                dismiss_on_select: false,
                is_disabled: self.active_thread_id.is_none(),
                ..Default::default()
            }];
        }

        let binding = self
            .clawbot_runtime()
            .ok()
            .and_then(|runtime| runtime.load_binding_for_session(&session_ref).ok())
            .flatten();
        let inbound_forwarding_enabled = binding
            .as_ref()
            .map(|binding| binding.inbound_forwarding_enabled)
            .unwrap_or(true);
        let outbound_forwarding_enabled = binding
            .as_ref()
            .map(|binding| binding.outbound_forwarding_enabled)
            .unwrap_or(true);
        let session_for_disconnect = session_ref.clone();
        let session_for_inbound = session_ref.clone();
        let session_for_outbound = session_ref.clone();
        let session_for_flush = session_ref;
        vec![
            SelectionItem {
                name: "Forwarding".to_string(),
                description: Some(format!(
                    "{} {} · {} {}",
                    SessionForwardingDirection::Inbound.label(),
                    on_off_label(inbound_forwarding_enabled),
                    SessionForwardingDirection::Outbound.label(),
                    on_off_label(outbound_forwarding_enabled)
                )),
                selected_description: Some(
                    "Inbound controls automatic session-to-thread forwarding. Outbound controls final thread replies back to Feishu.".to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            },
            SelectionItem {
                name: if inbound_forwarding_enabled {
                    "Disable Inbound Forwarding".to_string()
                } else {
                    "Enable Inbound Forwarding".to_string()
                },
                description: Some(if inbound_forwarding_enabled {
                    "Keep caching Feishu messages, but stop auto-forwarding them into the bound thread.".to_string()
                } else {
                    "Resume automatic forwarding from this Feishu session into the bound thread."
                        .to_string()
                }),
                selected_description: Some(if inbound_forwarding_enabled {
                    "Manual flush still works even when automatic inbound forwarding is disabled."
                        .to_string()
                } else {
                    "New inbound messages will resume draining into the bound thread as they arrive."
                        .to_string()
                }),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetSessionForwarding {
                        session: session_for_inbound.clone(),
                        mode: if inbound_forwarding_enabled {
                            SessionForwardingMode::InboundDisabled
                        } else {
                            SessionForwardingMode::InboundEnabled
                        },
                    })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: if outbound_forwarding_enabled {
                    "Disable Outbound Forwarding".to_string()
                } else {
                    "Enable Outbound Forwarding".to_string()
                },
                description: Some(if outbound_forwarding_enabled {
                    "Stop forwarding final thread replies back into the bound Feishu session."
                        .to_string()
                } else {
                    "Resume forwarding final thread replies back into the bound Feishu session."
                        .to_string()
                }),
                selected_description: Some(if outbound_forwarding_enabled {
                    "Inbound message handling stays unchanged; this only affects outbound final replies."
                        .to_string()
                } else {
                    "Future turn completions and failure summaries will be sent back to Feishu again."
                        .to_string()
                }),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotSetSessionForwarding {
                        session: session_for_outbound.clone(),
                        mode: if outbound_forwarding_enabled {
                            SessionForwardingMode::OutboundDisabled
                        } else {
                            SessionForwardingMode::OutboundEnabled
                        },
                    })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Disconnect".to_string(),
                description: Some(
                    "Remove the persisted thread binding and stop routing this session."
                        .to_string(),
                ),
                selected_description: Some(
                    "Unread cache is preserved; only the session-thread binding is removed."
                        .to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotDisconnect {
                        session: session_for_disconnect.clone(),
                    })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Flush Cached Messages".to_string(),
                description: Some(format!(
                    "Clear {} cached inbound messages for this session.",
                    session.unread_count
                )),
                selected_description: Some(
                    "This drains the persisted unread cache for the selected session.".to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ClawbotFlushCachedMessages {
                        session: session_for_flush.clone(),
                    })
                })],
                dismiss_on_select: false,
                is_disabled: session.unread_count == 0,
                ..Default::default()
            },
        ]
    }

    fn clawbot_runtime(&self) -> Result<ClawbotRuntime> {
        ClawbotRuntime::load(self.config.cwd.clone().into())
    }

    fn refresh_clawbot_views_if_active(&mut self) {
        let clawbot_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_PANEL_VIEW_ID);
        if clawbot_selected_idx.is_some() {
            let _ = self.chat_widget.replace_selection_view_if_active(
                CLAWBOT_PANEL_VIEW_ID,
                self.clawbot_panel_params(clawbot_selected_idx),
            );
        }
        let clawbot_sessions_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_SESSIONS_PANEL_VIEW_ID);
        if clawbot_sessions_selected_idx.is_some() {
            let _ = self.chat_widget.replace_selection_view_if_active(
                CLAWBOT_SESSIONS_PANEL_VIEW_ID,
                self.clawbot_sessions_panel_params(clawbot_sessions_selected_idx),
            );
        }
    }

    pub(crate) fn replay_clawbot_history_cells_for_active_thread(&mut self) {
        let Some(thread_id) = self.active_thread_id else {
            return;
        };
        let Some(cells) = self.clawbot_thread_history_cells.get(&thread_id) else {
            return;
        };
        let width = 80;
        for cell in cells {
            self.transcript_cells.push(cell.clone());
            let mut display = cell.display_lines(width);
            if !display.is_empty() {
                if !cell.is_stream_continuation() {
                    if self.has_emitted_history_lines {
                        display.insert(0, ratatui::text::Line::default());
                    } else {
                        self.has_emitted_history_lines = true;
                    }
                }
                self.deferred_history_lines.extend(display);
            }
        }
    }

    fn insert_clawbot_origin_info_cell(&mut self, thread_id: ThreadId, session_label: &str) {
        let stored_cell: Arc<dyn HistoryCell> = Arc::new(clawbot_origin_info_cell(session_label));
        self.clawbot_thread_history_cells
            .entry(thread_id)
            .or_default()
            .push(stored_cell);
        if self.active_thread_id == Some(thread_id) {
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                clawbot_origin_info_cell(session_label),
            )));
        }
    }

    fn insert_clawbot_action_info_cell(&mut self, thread_id: ThreadId, title: &str, hint: &str) {
        let stored_cell: Arc<dyn HistoryCell> = Arc::new(clawbot_action_info_cell(title, hint));
        self.clawbot_thread_history_cells
            .entry(thread_id)
            .or_default()
            .push(stored_cell);
        if self.active_thread_id == Some(thread_id) {
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                clawbot_action_info_cell(title, hint),
            )));
        }
    }

    async fn submit_clawbot_message_to_thread(
        &mut self,
        thread_id: ThreadId,
        message: String,
        turn_mode: ClawbotTurnMode,
    ) -> Result<String> {
        let trimmed = message.trim().to_string();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("cannot submit empty clawbot message"));
        }

        let op = {
            let thread = self.server.get_thread(thread_id).await.map_err(|error| {
                anyhow::anyhow!("failed to find bound thread {thread_id}: {error}")
            })?;
            let config_snapshot = thread.config_snapshot().await;
            Op::UserTurn {
                items: vec![codex_protocol::user_input::UserInput::Text {
                    text: trimmed,
                    text_elements: Vec::new(),
                }],
                cwd: config_snapshot.cwd,
                approval_policy: clawbot_approval_policy(
                    config_snapshot.approval_policy,
                    turn_mode,
                ),
                approvals_reviewer: Some(config_snapshot.approvals_reviewer),
                sandbox_policy: config_snapshot.sandbox_policy,
                model: config_snapshot.model,
                effort: config_snapshot.reasoning_effort,
                summary: None,
                service_tier: config_snapshot.service_tier.map(Some),
                final_output_json_schema: None,
                collaboration_mode: None,
                personality: self.config.personality,
            }
        };

        let replay_state_op =
            super::ThreadEventStore::op_can_change_pending_replay_state(&op).then(|| op.clone());
        crate::session_log::log_outbound_op(&op);
        let thread =
            self.server.get_thread(thread_id).await.map_err(|error| {
                anyhow::anyhow!("failed to find bound thread {thread_id}: {error}")
            })?;
        let turn_id = thread
            .submit(op)
            .await
            .map_err(|error| anyhow::anyhow!("failed to submit clawbot op: {error}"))?;

        if let Some(op) = replay_state_op.as_ref() {
            self.note_thread_outbound_op(thread_id, op).await;
        }
        Ok(turn_id)
    }

    async fn send_clawbot_thread_reply(&mut self, thread_id: ThreadId, text: String) -> Result<()> {
        let runtime = self.clawbot_runtime()?;
        let Some(binding) = runtime.load_binding_for_thread(&thread_id.to_string())? else {
            return Ok(());
        };
        if !binding.outbound_forwarding_enabled {
            return Ok(());
        }
        let session = binding.session_ref();

        self.send_clawbot_outbound_text(ProviderOutboundTextMessage { session, text })
            .await
    }

    fn register_pending_clawbot_turn(&mut self, turn: PendingClawbotTurn) {
        self.clawbot_pending_turns
            .entry(turn.thread_id)
            .or_default()
            .push_back(turn);
    }

    fn take_pending_clawbot_turn(
        &mut self,
        thread_id: ThreadId,
        event: &EventMsg,
    ) -> Option<PendingClawbotTurn> {
        let queue = self.clawbot_pending_turns.get_mut(&thread_id)?;
        let pending = match event {
            EventMsg::TurnComplete(turn_complete) => queue
                .iter()
                .position(|pending| pending.turn_id == turn_complete.turn_id)
                .and_then(|index| queue.remove(index)),
            EventMsg::Error(_) => queue.pop_front(),
            _ => None,
        };
        if queue.is_empty() {
            self.clawbot_pending_turns.remove(&thread_id);
        }
        pending
    }

    fn clawbot_turn_mode_for_turn(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Option<ClawbotTurnMode> {
        self.clawbot_pending_turns
            .get(&thread_id)?
            .iter()
            .find(|pending| pending.turn_id == turn_id)
            .map(|pending| pending.turn_mode)
    }

    async fn send_clawbot_auto_ack(
        &mut self,
        target: ProviderMessageRef,
        thread_id: ThreadId,
    ) -> Result<()> {
        self.send_clawbot_outbound_reaction(ProviderOutboundReaction {
            target,
            emoji_type: FEISHU_AUTO_ACK_EMOJI_TYPE.to_string(),
        })
        .await?;
        self.insert_clawbot_action_info_cell(
            thread_id,
            "Feishu auto reaction",
            FEISHU_AUTO_ACK_DISPLAY,
        );
        Ok(())
    }

    async fn auto_respond_to_clawbot_user_input_request(
        &mut self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Result<bool> {
        let Some(turn_mode) = self.clawbot_turn_mode_for_turn(thread_id, turn_id) else {
            return Ok(false);
        };
        if !turn_mode.uses_noninteractive_prompt_handling() {
            return Ok(false);
        }

        self.submit_op_to_thread(
            thread_id,
            Op::UserInputAnswer {
                id: turn_id.to_string(),
                response: RequestUserInputResponse {
                    answers: std::collections::HashMap::new(),
                },
            },
        )
        .await;
        self.insert_clawbot_action_info_cell(
            thread_id,
            "Clawbot auto response",
            "question tool skipped",
        );
        Ok(true)
    }

    async fn auto_respond_to_clawbot_permissions_request(
        &mut self,
        thread_id: ThreadId,
        turn_id: &str,
        call_id: &str,
    ) -> Result<bool> {
        let Some(turn_mode) = self.clawbot_turn_mode_for_turn(thread_id, turn_id) else {
            return Ok(false);
        };
        if !turn_mode.uses_noninteractive_prompt_handling() {
            return Ok(false);
        }

        self.submit_op_to_thread(
            thread_id,
            Op::RequestPermissionsResponse {
                id: call_id.to_string(),
                response: RequestPermissionsResponse {
                    permissions: Default::default(),
                    scope: PermissionGrantScope::Turn,
                },
            },
        )
        .await;
        self.insert_clawbot_action_info_cell(
            thread_id,
            "Clawbot auto response",
            "permission request denied",
        );
        Ok(true)
    }

    fn start_clawbot_provider_runtime(&mut self, provider: ProviderKind) -> Result<()> {
        let runtime = self.clawbot_runtime()?;
        match provider {
            ProviderKind::Feishu => {
                if let Some(task) = self.clawbot_provider_tasks.remove(&provider) {
                    task.abort();
                }

                if let Some(provider_runtime) = runtime.feishu_provider() {
                    let app_event_tx = self.app_event_tx.clone();
                    let handle = tokio::spawn(async move {
                        let (provider_event_tx, mut provider_event_rx) =
                            mpsc::unbounded_channel::<ProviderEvent>();
                        let app_event_forwarder = tokio::spawn(async move {
                            while let Some(event) = provider_event_rx.recv().await {
                                app_event_tx.send(AppEvent::ClawbotProviderEvent {
                                    event: Box::new(event),
                                });
                            }
                        });

                        let _ = provider_runtime.run(provider_event_tx).await;
                        app_event_forwarder.abort();
                    });
                    self.clawbot_provider_tasks.insert(provider, handle);
                }
            }
        }
        Ok(())
    }

    async fn send_clawbot_outbound_text(
        &mut self,
        message: ProviderOutboundTextMessage,
    ) -> Result<()> {
        if let Some(tx) = &self.clawbot_outbound_tx {
            tx.send(ProviderOutboundAction::Text(message))
                .map_err(|err| {
                    anyhow::anyhow!("failed to capture clawbot outbound message: {err}")
                })?;
            return Ok(());
        }

        match message.session.provider {
            ProviderKind::Feishu => {
                let runtime = self.clawbot_runtime()?;
                let Some(mut provider_runtime) = runtime.feishu_provider() else {
                    return Err(anyhow::anyhow!("missing Feishu provider config"));
                };
                provider_runtime.send_text(message).await?;
            }
        }

        self.refresh_clawbot_views_if_active();
        Ok(())
    }

    async fn send_clawbot_outbound_reaction(
        &mut self,
        reaction: ProviderOutboundReaction,
    ) -> Result<ProviderReactionReceipt> {
        if let Some(tx) = &self.clawbot_outbound_tx {
            let receipt = ProviderReactionReceipt {
                target: reaction.target.clone(),
                reaction_id: format!("captured-{}", reaction.emoji_type),
                emoji_type: reaction.emoji_type.clone(),
            };
            tx.send(ProviderOutboundAction::AddReaction(reaction))
                .map_err(|err| anyhow::anyhow!("failed to capture clawbot reaction: {err}"))?;
            return Ok(receipt);
        }

        let receipt = match reaction.target.provider {
            ProviderKind::Feishu => {
                let runtime = self.clawbot_runtime()?;
                let Some(mut provider_runtime) = runtime.feishu_provider() else {
                    return Err(anyhow::anyhow!("missing Feishu provider config"));
                };
                provider_runtime.add_reaction(reaction).await?
            }
        };

        self.refresh_clawbot_views_if_active();
        Ok(receipt)
    }
}

fn feishu_config_summary(config: Option<&FeishuConfig>) -> String {
    let Some(config) = config else {
        return "No Feishu credentials saved yet.".to_string();
    };

    if config.has_api_credentials() {
        let verification_state = if config
            .verification_token
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            "verification token set"
        } else {
            "verification token not set"
        };
        format!("API credentials configured · {verification_state}.")
    } else if config.is_empty() {
        "No Feishu credentials saved yet.".to_string()
    } else {
        "Incomplete API credentials. Set both app_id and app_secret.".to_string()
    }
}

fn feishu_runtime_state_for_config(config: FeishuConfig) -> ProviderRuntimeState {
    if config.has_api_credentials() {
        ProviderRuntimeState {
            provider: ProviderKind::Feishu,
            connection: ConnectionStatus::Disconnected,
            last_error: None,
            updated_at: None,
        }
    } else {
        ProviderRuntimeState::unconfigured(ProviderKind::Feishu)
    }
}

fn clawbot_turn_mode_summary(mode: ClawbotTurnMode) -> String {
    match mode {
        ClawbotTurnMode::Interactive => {
            "interactive · clawbot turns may surface question and approval prompts.".to_string()
        }
        ClawbotTurnMode::NonInteractive => {
            "non-interactive · clawbot turns auto-dismiss question and approval prompts."
                .to_string()
        }
    }
}

fn on_off_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn clawbot_approval_policy(
    existing_policy: AskForApproval,
    turn_mode: ClawbotTurnMode,
) -> AskForApproval {
    if !turn_mode.uses_noninteractive_prompt_handling() {
        return existing_policy;
    }

    AskForApproval::Granular(GranularApprovalConfig {
        sandbox_approval: false,
        rules: false,
        skill_approval: false,
        request_permissions: false,
        mcp_elicitations: false,
    })
}

fn mask_secret(value: &str) -> String {
    let char_count = value.chars().count();
    if char_count <= 4 {
        return "*".repeat(char_count.max(1));
    }

    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}{}", "*".repeat(char_count - 4), suffix)
}

fn truncate_value(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn clawbot_origin_info_cell(session_label: &str) -> history_cell::PlainHistoryCell {
    history_cell::new_info_event(
        "Feishu message".to_string(),
        Some(session_label.to_string()),
    )
}

fn clawbot_action_info_cell(title: &str, hint: &str) -> history_cell::PlainHistoryCell {
    history_cell::new_info_event(title.to_string(), Some(hint.to_string()))
}
