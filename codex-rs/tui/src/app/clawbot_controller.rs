use super::App;
use crate::app_event::ClawbotEvent;
use crate::app_server_session::AppServerSession;
use crate::tui;

pub(super) struct ClawbotController;

impl ClawbotController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: ClawbotEvent,
    ) {
        match event {
            ClawbotEvent::ClawbotProviderEvent { event } => {
                if let Err(err) = app.handle_clawbot_provider_event(app_server, event).await {
                    tracing::warn!(error = %err, "failed to handle clawbot provider event");
                    app.chat_widget
                        .add_error_message(format!("Clawbot provider event failed: {err}"));
                }
            }
            ClawbotEvent::ClawbotTurnCompleted { thread_id, turn } => {
                if let Err(err) = app
                    .handle_clawbot_turn_completed(app_server, thread_id, turn)
                    .await
                {
                    tracing::warn!(error = %err, "failed to handle clawbot turn completion");
                    app.chat_widget
                        .add_error_message(format!("Clawbot turn forwarding failed: {err}"));
                }
            }
            ClawbotEvent::OpenClawbotManagement => {
                app.open_clawbot_management_popup();
            }
            ClawbotEvent::OpenClawbotManagementView { destination } => {
                app.open_clawbot_management_view(destination);
            }
            ClawbotEvent::OpenClawbotFeishuConfigPrompt { field } => {
                app.open_clawbot_feishu_config_prompt(field);
            }
            ClawbotEvent::SaveClawbotFeishuConfigValue { field, value } => {
                if let Err(err) = app.save_clawbot_feishu_config_value(field, value) {
                    app.chat_widget
                        .add_error_message(format!("Failed to save Clawbot config: {err}"));
                }
            }
            ClawbotEvent::BindClawbotDiscoveredSession { session_id } => {
                if let Err(err) = app
                    .bind_clawbot_discovered_session_to_current_thread(app_server, session_id)
                    .await
                {
                    app.chat_widget
                        .add_error_message(format!("Failed to bind Clawbot session: {err}"));
                }
            }
            ClawbotEvent::BindClawbotSessionAndPreempt { session_id } => {
                if let Err(err) = app
                    .bind_clawbot_session_to_current_thread_and_preempt(app_server, session_id)
                    .await
                {
                    app.chat_widget.add_error_message(format!(
                        "Failed to bind and preempt Clawbot session: {err}"
                    ));
                }
            }
            ClawbotEvent::ClawbotSetTurnMode { mode } => {
                if let Err(err) = app.save_clawbot_turn_mode(mode) {
                    app.chat_widget
                        .add_error_message(format!("Failed to save Clawbot turn mode: {err}"));
                }
            }
            ClawbotEvent::ClawbotSetThreadForwarding {
                thread_id,
                channel,
                enabled,
            } => {
                if let Err(err) = app.clawbot_set_thread_forwarding(thread_id, channel, enabled) {
                    app.chat_widget
                        .add_error_message(format!("Failed to update Clawbot forwarding: {err}"));
                }
            }
            ClawbotEvent::ScanClawbotFeishuSessions => {
                if let Err(err) = app.scan_clawbot_feishu_sessions().await {
                    app.chat_widget
                        .add_error_message(format!("Failed to scan Feishu sessions: {err}"));
                }
            }
            ClawbotEvent::ClearClawbotFeishuSessions => {
                if let Err(err) = app.clear_clawbot_feishu_sessions() {
                    app.chat_widget.add_error_message(format!(
                        "Failed to clear unbound Feishu sessions: {err}"
                    ));
                }
            }
            ClawbotEvent::RetryClawbotFeishuConnection => {
                if let Err(err) = app.retry_clawbot_feishu_connection() {
                    app.chat_widget.add_error_message(format!(
                        "Failed to restart Clawbot Feishu runtime: {err}"
                    ));
                }
            }
            ClawbotEvent::ToggleClawbotForceConnect => {
                if let Err(err) = app.toggle_clawbot_force_connect() {
                    app.chat_widget.add_error_message(format!(
                        "Failed to update Clawbot ws preemption: {err}"
                    ));
                }
            }
            ClawbotEvent::ClawbotDisconnectThread { thread_id } => {
                if let Err(err) = app.clawbot_disconnect_thread(thread_id) {
                    app.chat_widget
                        .add_error_message(format!("Failed to disconnect Clawbot binding: {err}"));
                }
            }
            ClawbotEvent::EditClawbotStateFile { label, path } => {
                app.edit_clawbot_state_file_from_ui(tui, label, path).await;
            }
        }
    }
}
