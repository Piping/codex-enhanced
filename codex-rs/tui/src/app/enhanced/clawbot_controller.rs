use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

use super::App;
use super::feature_dispatch::FeatureDispatchFuture;
use super::feature_dispatch::FeatureDispatchOutcome;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotControlsDestination;
use crate::app_event::ClawbotSessionBindSource;
use crate::app_server_session::AppServerSession;
use crate::tui;
use codex_clawbot::PendingClawbotTurn;
#[cfg(test)]
use codex_clawbot::ProviderOutboundReaction;
#[cfg(test)]
use codex_clawbot::ProviderOutboundTextMessage;
use codex_protocol::ThreadId;
use tokio::task::JoinHandle;

pub(super) struct ClawbotFeatureState {
    pub(super) controls_destination: ClawbotControlsDestination,
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) provider_task: Option<JoinHandle<()>>,
    pub(super) pending_turns: HashMap<ThreadId, VecDeque<PendingClawbotTurn>>,
    #[cfg(test)]
    pub(super) outbound_messages: Vec<ProviderOutboundTextMessage>,
    #[cfg(test)]
    pub(super) outbound_reactions: Vec<ProviderOutboundReaction>,
    #[cfg(test)]
    pub(super) removed_outbound_reactions: Vec<ProviderOutboundReaction>,
}

impl Default for ClawbotFeatureState {
    fn default() -> Self {
        Self {
            controls_destination: ClawbotControlsDestination::Root,
            workspace_root: None,
            provider_task: None,
            pending_turns: HashMap::new(),
            #[cfg(test)]
            outbound_messages: Vec::new(),
            #[cfg(test)]
            outbound_reactions: Vec::new(),
            #[cfg(test)]
            removed_outbound_reactions: Vec::new(),
        }
    }
}

pub(super) struct ClawbotController;

impl ClawbotController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::ClawbotProviderEvent { event } => {
                if let Err(err) = app.handle_clawbot_provider_event(app_server, event).await {
                    tracing::warn!(error = %err, "failed to handle clawbot provider event");
                    app.chat_widget
                        .add_error_message(format!("Clawbot provider event failed: {err}"));
                }
            }
            AppEvent::ClawbotTurnCompleted { thread_id, turn } => {
                if let Err(err) = app
                    .handle_clawbot_turn_completed(app_server, thread_id, turn)
                    .await
                {
                    tracing::warn!(error = %err, "failed to handle clawbot turn completion");
                    app.chat_widget
                        .add_error_message(format!("Clawbot turn forwarding failed: {err}"));
                }
            }
            AppEvent::OpenClawbotManagement => {
                app.open_clawbot_management_popup();
            }
            AppEvent::OpenClawbotManagementView { destination } => {
                app.open_clawbot_management_view(destination);
            }
            AppEvent::OpenClawbotFeishuConfigPrompt { field } => {
                app.open_clawbot_feishu_config_prompt(field);
            }
            AppEvent::OpenClawbotManualBindSessionPrompt => {
                app.open_clawbot_manual_bind_session_prompt();
            }
            AppEvent::SaveClawbotFeishuConfigValue { field, value } => {
                if let Err(err) = app.save_clawbot_feishu_config_value(field, value) {
                    app.chat_widget
                        .add_error_message(format!("Failed to save Clawbot config: {err}"));
                }
            }
            AppEvent::BindClawbotDiscoveredSession { session_id } => {
                if let Err(err) = app
                    .bind_clawbot_session_to_current_thread(
                        app_server,
                        session_id,
                        ClawbotSessionBindSource::DiscoveredSession,
                    )
                    .await
                {
                    app.chat_widget
                        .add_error_message(format!("Failed to bind Clawbot session: {err}"));
                }
            }
            AppEvent::BindClawbotSessionAndPreempt { session_id } => {
                if let Err(err) = app
                    .bind_clawbot_session_to_current_thread_and_preempt(app_server, session_id)
                    .await
                {
                    app.chat_widget.add_error_message(format!(
                        "Failed to bind and preempt Clawbot session: {err}"
                    ));
                }
            }
            AppEvent::SaveClawbotManualBindSessionId { session_id } => {
                if let Err(err) = app
                    .bind_clawbot_session_to_current_thread(
                        app_server,
                        session_id,
                        ClawbotSessionBindSource::ManualSessionId,
                    )
                    .await
                {
                    app.chat_widget
                        .add_error_message(format!("Failed to bind Clawbot session: {err}"));
                }
            }
            AppEvent::ClawbotSetTurnMode { mode } => {
                if let Err(err) = app.save_clawbot_turn_mode(mode) {
                    app.chat_widget
                        .add_error_message(format!("Failed to save Clawbot turn mode: {err}"));
                }
            }
            AppEvent::ClawbotSetThreadForwarding {
                thread_id,
                channel,
                enabled,
            } => {
                if let Err(err) = app.clawbot_set_thread_forwarding(thread_id, channel, enabled) {
                    app.chat_widget
                        .add_error_message(format!("Failed to update Clawbot forwarding: {err}"));
                }
            }
            AppEvent::ScanClawbotFeishuSessions => {
                if let Err(err) = app.scan_clawbot_feishu_sessions().await {
                    app.chat_widget
                        .add_error_message(format!("Failed to scan Feishu sessions: {err}"));
                }
            }
            AppEvent::ClearClawbotFeishuSessions => {
                if let Err(err) = app.clear_clawbot_feishu_sessions() {
                    app.chat_widget.add_error_message(format!(
                        "Failed to clear unbound Feishu sessions: {err}"
                    ));
                }
            }
            AppEvent::RetryClawbotFeishuConnection => {
                if let Err(err) = app.retry_clawbot_feishu_connection() {
                    app.chat_widget.add_error_message(format!(
                        "Failed to restart Clawbot Feishu runtime: {err}"
                    ));
                }
            }
            AppEvent::ToggleClawbotForceConnect => {
                if let Err(err) = app.toggle_clawbot_force_connect() {
                    app.chat_widget.add_error_message(format!(
                        "Failed to update Clawbot ws preemption: {err}"
                    ));
                }
            }
            AppEvent::ClawbotDisconnectThread { thread_id } => {
                if let Err(err) = app.clawbot_disconnect_thread(thread_id) {
                    app.chat_widget
                        .add_error_message(format!("Failed to disconnect Clawbot binding: {err}"));
                }
            }
            AppEvent::EditClawbotStateFile { label, path } => {
                app.edit_clawbot_state_file_from_ui(tui, label, path).await;
            }
            _ => unreachable!("non-clawbot event passed to clawbot controller"),
        }
    }
}

pub(super) fn matches_event(event: &AppEvent) -> bool {
    matches!(
        event,
        AppEvent::ClawbotProviderEvent { .. }
            | AppEvent::ClawbotTurnCompleted { .. }
            | AppEvent::OpenClawbotManagement
            | AppEvent::OpenClawbotManagementView { .. }
            | AppEvent::OpenClawbotFeishuConfigPrompt { .. }
            | AppEvent::OpenClawbotManualBindSessionPrompt
            | AppEvent::SaveClawbotFeishuConfigValue { .. }
            | AppEvent::BindClawbotDiscoveredSession { .. }
            | AppEvent::BindClawbotSessionAndPreempt { .. }
            | AppEvent::SaveClawbotManualBindSessionId { .. }
            | AppEvent::ClawbotSetTurnMode { .. }
            | AppEvent::ClawbotSetThreadForwarding { .. }
            | AppEvent::ScanClawbotFeishuSessions
            | AppEvent::ClearClawbotFeishuSessions
            | AppEvent::RetryClawbotFeishuConnection
            | AppEvent::ToggleClawbotForceConnect
            | AppEvent::ClawbotDisconnectThread { .. }
            | AppEvent::EditClawbotStateFile { .. }
    )
}

pub(super) fn dispatch<'a>(
    app: &'a mut App,
    tui: &'a mut tui::Tui,
    app_server: &'a mut AppServerSession,
    event: AppEvent,
) -> FeatureDispatchFuture<'a> {
    Box::pin(async move {
        ClawbotController::handle(app, tui, app_server, event).await;
        Ok(FeatureDispatchOutcome::Handled)
    })
}
