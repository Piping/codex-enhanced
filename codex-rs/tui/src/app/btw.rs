use std::collections::HashMap;
use std::path::PathBuf;

use codex_app_server_protocol::ApprovalsReviewer as AppServerApprovalsReviewer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use uuid::Uuid;

use super::App;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::SideContentWidth;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;

const BTW_DEVELOPER_INSTRUCTIONS: &str = concat!(
    "This is a hidden `/btw` discussion thread. ",
    "Treat it as a temporary scratchpad that must not mutate the workspace or persistent state. ",
    "Do not write files, apply patches, spawn agents, or perform side-effectful actions. ",
    "If you need to inspect local context, keep it read-only and concise. ",
    "Your answer will be shown to the user in a temporary confirmation view and may be inserted ",
    "back into the main composer."
);
const BTW_DISCUSSION_VIEW_ID: &str = "btw-discussion";
const PREVIEW_CHAR_LIMIT: usize = 1_200;
const SUMMARY_MAX_LINES: usize = 4;
const SUMMARY_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BtwSessionState {
    pub(crate) thread_id: ThreadId,
    pub(crate) final_message: Option<String>,
}

impl App {
    pub(crate) async fn start_btw_discussion(
        &mut self,
        app_server: &mut AppServerSession,
        prompt: String,
    ) {
        if self.btw_session.is_some() {
            self.chat_widget.add_info_message(
                "A `/btw` discussion is already active.".to_string(),
                Some("Finish or discard it before starting another one.".to_string()),
            );
            return;
        }

        let trimmed_prompt = prompt.trim();
        if trimmed_prompt.is_empty() {
            self.chat_widget
                .add_error_message("Usage: /btw <temporary discussion prompt>".to_string());
            return;
        }

        self.open_btw_loading_popup();

        let thread_id = match self.start_btw_thread(app_server).await {
            Ok(thread_id) => thread_id,
            Err(err) => {
                self.open_btw_failure_popup(&format!("Failed to start `/btw`: {err}"));
                return;
            }
        };
        self.btw_session = Some(BtwSessionState {
            thread_id,
            final_message: None,
        });

        let turn_result = app_server
            .turn_start(
                thread_id,
                btw_turn_input(trimmed_prompt),
                self.btw_turn_cwd_path(app_server),
                AskForApproval::Never,
                self.config.approvals_reviewer,
                SandboxPolicy::new_read_only_policy(),
                self.chat_widget.current_model().to_string(),
                self.chat_widget.current_reasoning_effort(),
                /*summary*/ None,
                self.chat_widget.current_service_tier().map(Some),
                /*collaboration_mode*/ None,
                self.config.personality,
                /*output_schema*/ None,
            )
            .await;
        if let Err(err) = turn_result {
            self.close_btw_session(app_server).await;
            self.open_btw_failure_popup(&format!("Failed to submit `/btw`: {err}"));
        }
    }

    pub(crate) fn finish_btw_discussion(
        &mut self,
        thread_id: ThreadId,
        result: Result<String, String>,
    ) {
        let Some(session) = self.btw_session.as_mut() else {
            return;
        };
        if session.thread_id != thread_id {
            return;
        }

        match result {
            Ok(message) => {
                session.final_message = Some(message.clone());
                self.open_btw_result_popup(&message);
            }
            Err(err) => {
                self.open_btw_failure_popup(&format!("`/btw` failed: {err}"));
            }
        }
    }

    pub(crate) fn handle_btw_notification(
        &mut self,
        thread_id: ThreadId,
        notification: &codex_app_server_protocol::ServerNotification,
    ) -> bool {
        if self.btw_closing_thread_ids.contains(&thread_id) {
            if matches!(
                notification,
                codex_app_server_protocol::ServerNotification::ThreadClosed(_)
            ) {
                self.btw_closing_thread_ids.remove(&thread_id);
            }
            return true;
        }

        let Some(session) = self.btw_session.as_ref() else {
            return false;
        };
        if session.thread_id != thread_id {
            return false;
        }

        if session.final_message.is_some() {
            return true;
        }

        match notification {
            codex_app_server_protocol::ServerNotification::Error(notification)
                if !notification.will_retry =>
            {
                self.app_event_tx.send(AppEvent::BtwCompleted {
                    thread_id,
                    result: Err(notification.error.message.clone()),
                });
            }
            codex_app_server_protocol::ServerNotification::TurnCompleted(notification) => {
                let result = match notification.turn.status {
                    TurnStatus::Completed => last_agent_message_or_error(&notification.turn),
                    TurnStatus::Failed => last_agent_message_or_error(&notification.turn)
                        .or_else(|_| turn_failed_error(&notification.turn)),
                    TurnStatus::Interrupted => {
                        Err("Temporary discussion was interrupted.".to_string())
                    }
                    TurnStatus::InProgress => return true,
                };
                self.app_event_tx
                    .send(AppEvent::BtwCompleted { thread_id, result });
            }
            codex_app_server_protocol::ServerNotification::ThreadClosed(_) => {
                self.app_event_tx.send(AppEvent::BtwCompleted {
                    thread_id,
                    result: Err("Temporary discussion closed before a final answer.".to_string()),
                });
            }
            _ => {}
        }

        true
    }

    pub(crate) async fn insert_btw_summary(&mut self, app_server: &mut AppServerSession) {
        let Some(message) = self
            .btw_session
            .as_ref()
            .and_then(|session| session.final_message.as_deref())
        else {
            self.open_btw_failure_popup("`/btw` summary is unavailable.");
            return;
        };

        self.insert_btw_text(summarize_message(message));
        self.chat_widget.add_info_message(
            "Inserted `/btw` summary into the composer.".to_string(),
            /*hint*/ None,
        );
        self.close_btw_session(app_server).await;
    }

    pub(crate) async fn insert_btw_full(&mut self, app_server: &mut AppServerSession) {
        let Some(message) = self
            .btw_session
            .as_ref()
            .and_then(|session| session.final_message.as_deref())
        else {
            self.open_btw_failure_popup("`/btw` answer is unavailable.");
            return;
        };

        self.insert_btw_text(full_insert_text(message));
        self.chat_widget.add_info_message(
            "Inserted `/btw` answer into the composer.".to_string(),
            /*hint*/ None,
        );
        self.close_btw_session(app_server).await;
    }

    pub(crate) async fn discard_btw_session(&mut self, app_server: &mut AppServerSession) {
        let had_session = self.btw_session.is_some();
        self.close_btw_session(app_server).await;
        if had_session {
            self.chat_widget.add_info_message(
                "Discarded `/btw` discussion.".to_string(),
                /*hint*/ None,
            );
        }
    }

    fn insert_btw_text(&mut self, text: String) {
        if !self
            .chat_widget
            .composer_text_with_pending()
            .trim()
            .is_empty()
        {
            self.chat_widget.insert_str("\n\n");
        }
        self.chat_widget.insert_str(&text);
    }

    pub(crate) async fn close_btw_session(&mut self, app_server: &mut AppServerSession) {
        let Some(session) = self.btw_session.take() else {
            return;
        };
        self.close_btw_thread(app_server, session.thread_id).await;
    }

    async fn close_btw_thread(&mut self, app_server: &mut AppServerSession, thread_id: ThreadId) {
        if !self.btw_closing_thread_ids.insert(thread_id) {
            return;
        }
        if let Err(err) = app_server.thread_unsubscribe(thread_id).await {
            tracing::warn!(thread_id = %thread_id, error = %err, "failed to close `/btw` thread");
        }
    }

    async fn start_btw_thread(&self, app_server: &AppServerSession) -> Result<ThreadId, String> {
        let request_handle = app_server.request_handle();
        if let Some(thread_id) = self.chat_widget.thread_id() {
            let response: ThreadForkResponse = request_handle
                .request_typed(ClientRequest::ThreadFork {
                    request_id: request_id(),
                    params: btw_thread_fork_params(self, thread_id, app_server),
                })
                .await
                .map_err(|err| format!("failed to fork temporary thread: {err}"))?;
            ThreadId::from_string(&response.thread.id)
                .map_err(|err| format!("invalid `/btw` thread id: {err}"))
        } else {
            let response: ThreadStartResponse = request_handle
                .request_typed(ClientRequest::ThreadStart {
                    request_id: request_id(),
                    params: btw_thread_start_params(self, app_server),
                })
                .await
                .map_err(|err| format!("failed to start temporary thread: {err}"))?;
            ThreadId::from_string(&response.thread.id)
                .map_err(|err| format!("invalid `/btw` thread id: {err}"))
        }
    }

    fn open_btw_loading_popup(&mut self) {
        self.show_btw_popup(btw_loading_view_params);
    }

    fn open_btw_result_popup(&mut self, message: &str) {
        self.show_btw_popup(|| btw_result_view_params(message));
    }

    fn open_btw_failure_popup(&mut self, error: &str) {
        self.show_btw_popup(|| btw_failure_view_params(error));
    }

    fn show_btw_popup<F>(&mut self, build: F)
    where
        F: Fn() -> SelectionViewParams,
    {
        if self
            .chat_widget
            .selected_index_for_active_view(BTW_DISCUSSION_VIEW_ID)
            .is_some()
        {
            let _ = self
                .chat_widget
                .replace_selection_view_if_active(BTW_DISCUSSION_VIEW_ID, build());
        } else {
            self.chat_widget.show_selection_view(build());
        }
    }

    fn btw_thread_cwd(&self, app_server: &AppServerSession) -> Option<String> {
        if app_server.is_remote() {
            app_server
                .remote_cwd_override()
                .map(|cwd| cwd.to_string_lossy().to_string())
        } else {
            Some(self.config.cwd.to_string_lossy().to_string())
        }
    }

    fn btw_turn_cwd_path(&self, app_server: &AppServerSession) -> PathBuf {
        if app_server.is_remote() {
            app_server
                .remote_cwd_override()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.config.cwd.to_path_buf())
        } else {
            self.config.cwd.to_path_buf()
        }
    }
}

fn request_id() -> RequestId {
    RequestId::String(format!("btw-{}", Uuid::new_v4()))
}

fn btw_turn_input(prompt: &str) -> Vec<UserInput> {
    vec![UserInput::Text {
        text: prompt.to_string(),
        text_elements: Vec::new(),
    }]
}

fn btw_thread_start_params(app: &App, app_server: &AppServerSession) -> ThreadStartParams {
    ThreadStartParams {
        model: Some(app.chat_widget.current_model().to_string()),
        model_provider: (!app_server.is_remote()).then_some(app.config.model_provider_id.clone()),
        cwd: app.btw_thread_cwd(app_server),
        approval_policy: Some(AskForApproval::Never.into()),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(
            app.config.approvals_reviewer,
        )),
        sandbox: Some(SandboxMode::ReadOnly),
        config: config_overrides(app.active_profile.as_deref()),
        developer_instructions: Some(merge_developer_instructions(
            app.config.developer_instructions.as_deref(),
        )),
        personality: app.config.personality,
        ephemeral: Some(true),
        persist_extended_history: true,
        ..ThreadStartParams::default()
    }
}

fn btw_thread_fork_params(
    app: &App,
    thread_id: ThreadId,
    app_server: &AppServerSession,
) -> ThreadForkParams {
    ThreadForkParams {
        thread_id: thread_id.to_string(),
        model: Some(app.chat_widget.current_model().to_string()),
        model_provider: (!app_server.is_remote()).then_some(app.config.model_provider_id.clone()),
        cwd: app.btw_thread_cwd(app_server),
        approval_policy: Some(AskForApproval::Never.into()),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(
            app.config.approvals_reviewer,
        )),
        sandbox: Some(SandboxMode::ReadOnly),
        config: config_overrides(app.active_profile.as_deref()),
        developer_instructions: Some(merge_developer_instructions(
            app.config.developer_instructions.as_deref(),
        )),
        ephemeral: true,
        persist_extended_history: true,
        ..ThreadForkParams::default()
    }
}

fn config_overrides(active_profile: Option<&str>) -> Option<HashMap<String, serde_json::Value>> {
    active_profile.map(|profile| {
        HashMap::from([(
            "profile".to_string(),
            serde_json::Value::String(profile.to_string()),
        )])
    })
}

fn merge_developer_instructions(existing: Option<&str>) -> String {
    match existing {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{BTW_DEVELOPER_INSTRUCTIONS}")
        }
        _ => BTW_DEVELOPER_INSTRUCTIONS.to_string(),
    }
}

fn preview_text(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.chars().count() <= PREVIEW_CHAR_LIMIT {
        return trimmed.to_string();
    }

    let preview: String = trimmed.chars().take(PREVIEW_CHAR_LIMIT).collect();
    format!("{preview}\n\n...preview truncated...")
}

fn summarize_message(message: &str) -> String {
    let mut kept = Vec::new();
    let mut used_chars = 0usize;
    for line in message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let next_len = used_chars.saturating_add(line.chars().count());
        if !kept.is_empty() && (kept.len() >= SUMMARY_MAX_LINES || next_len > SUMMARY_MAX_CHARS) {
            break;
        }
        kept.push(line.to_string());
        used_chars = next_len;
    }

    if kept.is_empty() {
        "BTW summary:\n(Empty answer)".to_string()
    } else {
        format!("BTW summary:\n{}", kept.join("\n"))
    }
}

fn full_insert_text(message: &str) -> String {
    format!("BTW discussion:\n{message}")
}

fn last_agent_message_or_error(turn: &codex_app_server_protocol::Turn) -> Result<String, String> {
    super::last_agent_message_for_turn(turn)
        .ok_or_else(|| "Temporary discussion finished without a final answer.".to_string())
}

fn turn_failed_error(turn: &codex_app_server_protocol::Turn) -> Result<String, String> {
    Err(turn
        .error
        .as_ref()
        .map(|error| error.message.clone())
        .unwrap_or_else(|| "Temporary discussion failed without a final error.".to_string()))
}

fn btw_loading_view_params() -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(BTW_DISCUSSION_VIEW_ID),
        title: Some("Temporary BTW discussion".to_string()),
        subtitle: Some("Running a hidden temporary discussion thread.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items: vec![SelectionItem {
            name: "Discard".to_string(),
            description: Some("Cancel and destroy the temporary discussion.".to_string()),
            actions: vec![Box::new(|tx| tx.send(AppEvent::BtwDiscard))],
            dismiss_on_select: true,
            ..Default::default()
        }],
        side_content: Paragraph::new(
            "Codex is answering in a hidden ephemeral thread. Nothing will be written back to the \
             main composer unless you explicitly choose an insert action."
                .to_string(),
        )
        .wrap(Wrap { trim: false })
        .into(),
        side_content_width: SideContentWidth::Half,
        side_content_min_width: 28,
        on_cancel: Some(Box::new(|tx| tx.send(AppEvent::BtwDiscard))),
        ..Default::default()
    }
}

fn btw_result_view_params(message: &str) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(BTW_DISCUSSION_VIEW_ID),
        title: Some("Temporary BTW answer".to_string()),
        subtitle: Some("Choose what to do with the temporary answer.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items: vec![
            SelectionItem {
                name: "Insert Summary".to_string(),
                description: Some("Insert a short summary into the main composer.".to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::BtwInsertSummary))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Insert Full".to_string(),
                description: Some("Insert the full answer into the main composer.".to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::BtwInsertFull))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Discard".to_string(),
                description: Some(
                    "Destroy the temporary discussion and keep the main composer untouched."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::BtwDiscard))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        side_content: Paragraph::new(preview_text(message))
            .wrap(Wrap { trim: false })
            .into(),
        side_content_width: SideContentWidth::Half,
        side_content_min_width: 28,
        on_cancel: Some(Box::new(|tx| tx.send(AppEvent::BtwDiscard))),
        ..Default::default()
    }
}

fn btw_failure_view_params(error: &str) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(BTW_DISCUSSION_VIEW_ID),
        title: Some("Temporary BTW failed".to_string()),
        subtitle: Some("The hidden temporary discussion did not complete.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items: vec![SelectionItem {
            name: "Close".to_string(),
            description: Some("Dismiss this temporary discussion.".to_string()),
            actions: vec![Box::new(|tx| tx.send(AppEvent::BtwDiscard))],
            dismiss_on_select: true,
            ..Default::default()
        }],
        side_content: Paragraph::new(error.to_string())
            .wrap(Wrap { trim: false })
            .into(),
        side_content_width: SideContentWidth::Half,
        side_content_min_width: 28,
        on_cancel: Some(Box::new(|tx| tx.send(AppEvent::BtwDiscard))),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    use super::btw_failure_view_params;
    use super::btw_loading_view_params;
    use super::btw_result_view_params;
    use super::merge_developer_instructions;
    use super::preview_text;
    use super::summarize_message;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
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
    fn btw_loading_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(btw_loading_view_params(), tx);

        assert_snapshot!("btw_loading_popup", render_selection_popup(&view, 92, 20));
    }

    #[test]
    fn btw_result_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            btw_result_view_params(
                "Use a hidden thread to brainstorm tradeoffs, then choose whether to insert the \
                 summary or the full answer back into the main composer.",
            ),
            tx,
        );

        assert_snapshot!("btw_result_popup", render_selection_popup(&view, 92, 28));
    }

    #[test]
    fn btw_failure_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            btw_failure_view_params("`/btw` failed: upstream unavailable"),
            tx,
        );

        assert_snapshot!("btw_failure_popup", render_selection_popup(&view, 92, 20));
    }

    #[test]
    fn summarize_btw_message_keeps_short_prefix_for_insertion() {
        let summary = summarize_message(
            "First point.\n\nSecond point.\nThird point.\nFourth point.\nFifth point.",
        );

        assert_eq!(
            summary,
            "BTW summary:\nFirst point.\nSecond point.\nThird point.\nFourth point."
        );
    }

    #[test]
    fn preview_text_truncates_long_messages() {
        let preview = preview_text(&"a".repeat(1_250));
        assert!(preview.contains("preview truncated"));
    }

    #[test]
    fn merge_developer_instructions_appends_btw_guardrail() {
        assert_eq!(
            merge_developer_instructions(Some("Stay focused.")),
            format!("Stay focused.\n\n{}", super::BTW_DEVELOPER_INSTRUCTIONS)
        );
    }
}
