use super::App;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::render::renderable::ColumnRenderable;
use codex_core::CodexThread;
use codex_core::RolloutRecorder;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReadOnlyAccess;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::path::Path;
use std::sync::Arc;
use tokio::task::JoinHandle;

const BTW_DISCUSSION_VIEW_ID: &str = "btw_discussion";
const BTW_CONTEXT_BUDGET_TOKENS: usize = 2_000;
const BTW_DEVELOPER_INSTRUCTIONS: &str = concat!(
    "This is a hidden `/btw` discussion thread. ",
    "Treat it as a temporary scratchpad that must not mutate the workspace or persistent state. ",
    "Do not write files, apply patches, spawn agents, or perform side-effectful actions. ",
    "If you need to inspect local context, keep it read-only and concise. ",
    "Your answer will be shown to the user in a temporary confirmation view and may be inserted ",
    "back into the main composer."
);

pub(crate) struct BtwSessionState {
    pub(crate) thread_id: ThreadId,
    pub(crate) thread: Arc<CodexThread>,
    pub(crate) listener_handle: JoinHandle<()>,
    pub(crate) final_message: Option<String>,
}

impl App {
    pub(crate) async fn start_btw_discussion(&mut self, prompt: String) {
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

        let mut btw_config = self.config.clone();
        btw_config.ephemeral = true;
        btw_config.include_apply_patch_tool = false;
        if let Err(err) = btw_config
            .permissions
            .approval_policy
            .set(AskForApproval::Never)
        {
            self.chat_widget
                .add_error_message(format!("Failed to configure `/btw` approvals: {err}"));
            return;
        }
        if let Err(err) = btw_config
            .permissions
            .sandbox_policy
            .set(SandboxPolicy::ReadOnly {
                access: ReadOnlyAccess::default(),
                network_access: false,
            })
        {
            self.chat_widget
                .add_error_message(format!("Failed to configure `/btw` sandbox: {err}"));
            return;
        }
        btw_config.developer_instructions = Some(merge_developer_instructions(
            btw_config.developer_instructions.take(),
        ));

        let initial_history =
            build_btw_initial_history(self.chat_widget.rollout_path().as_deref()).await;
        self.open_btw_loading_panel();

        let new_thread = match self
            .server
            .start_thread_with_history_and_source(
                btw_config,
                initial_history,
                SessionSource::SubAgent(SubAgentSource::Other("btw".to_string())),
            )
            .await
        {
            Ok(new_thread) => new_thread,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to start `/btw`: {err}"));
                return;
            }
        };

        let thread_id = new_thread.thread_id;
        let thread = new_thread.thread;
        let app_event_tx = self.app_event_tx.clone();
        let listener_thread = Arc::clone(&thread);
        let listener_handle = tokio::spawn(async move {
            let mut last_agent_message = None;
            loop {
                match listener_thread.next_event().await {
                    Ok(event) => match event.msg {
                        EventMsg::ItemCompleted(item_completed) => {
                            if let TurnItem::AgentMessage(message) = item_completed.item {
                                let text = message
                                    .content
                                    .into_iter()
                                    .map(|content| match content {
                                        AgentMessageContent::Text { text } => text,
                                    })
                                    .collect::<String>();
                                if !text.trim().is_empty() {
                                    last_agent_message = Some(text);
                                }
                            }
                        }
                        EventMsg::TurnComplete(turn_complete) => {
                            let message = turn_complete.last_agent_message.or(last_agent_message);
                            app_event_tx.send(AppEvent::BtwCompleted {
                                thread_id,
                                result: message.ok_or_else(|| {
                                    "Temporary discussion finished without a final answer."
                                        .to_string()
                                }),
                            });
                            break;
                        }
                        EventMsg::Error(error) => {
                            app_event_tx.send(AppEvent::BtwCompleted {
                                thread_id,
                                result: Err(error.message),
                            });
                            break;
                        }
                        EventMsg::ShutdownComplete => {
                            app_event_tx.send(AppEvent::BtwCompleted {
                                thread_id,
                                result: Err("Temporary discussion closed before a final answer."
                                    .to_string()),
                            });
                            break;
                        }
                        _ => {}
                    },
                    Err(err) => {
                        app_event_tx.send(AppEvent::BtwCompleted {
                            thread_id,
                            result: Err(format!("Temporary discussion failed: {err}")),
                        });
                        break;
                    }
                }
            }
        });

        self.btw_session = Some(BtwSessionState {
            thread_id,
            thread: Arc::clone(&thread),
            listener_handle,
            final_message: None,
        });

        let op = Op::UserInput {
            items: vec![codex_protocol::user_input::UserInput::Text {
                text: trimmed_prompt.to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        };
        if let Err(err) = thread.submit(op).await {
            self.chat_widget
                .add_error_message(format!("Failed to submit `/btw`: {err}"));
            self.cleanup_btw_session();
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
                self.open_btw_result_panel(&message);
            }
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("`/btw` failed: {err}"));
                self.cleanup_btw_session();
            }
        }
    }

    pub(crate) fn insert_btw_summary(&mut self) {
        let Some(message) = self
            .btw_session
            .as_ref()
            .and_then(|session| session.final_message.as_deref())
        else {
            self.chat_widget
                .add_error_message("`/btw` summary is unavailable.".to_string());
            self.cleanup_btw_session();
            return;
        };

        let summary = summarize_btw_message(message);
        self.insert_btw_text(summary, "Inserted `/btw` summary into the composer.");
    }

    pub(crate) fn insert_btw_full(&mut self) {
        let Some(message) = self
            .btw_session
            .as_ref()
            .and_then(|session| session.final_message.as_deref())
        else {
            self.chat_widget
                .add_error_message("`/btw` answer is unavailable.".to_string());
            self.cleanup_btw_session();
            return;
        };

        let text = format!("BTW discussion:\n{message}");
        self.insert_btw_text(text, "Inserted `/btw` answer into the composer.");
    }

    pub(crate) fn discard_btw_session(&mut self) {
        self.chat_widget.add_info_message(
            "Discarded `/btw` discussion.".to_string(),
            /*hint*/ None,
        );
        self.cleanup_btw_session();
    }

    fn insert_btw_text(&mut self, text: String, confirmation: &str) {
        if !self
            .chat_widget
            .composer_text_with_pending()
            .trim()
            .is_empty()
        {
            self.chat_widget.insert_str("\n\n");
        }
        self.chat_widget.insert_str(&text);
        self.chat_widget
            .add_info_message(confirmation.to_string(), /*hint*/ None);
        self.cleanup_btw_session();
    }

    fn cleanup_btw_session(&mut self) {
        let Some(session) = self.btw_session.take() else {
            return;
        };

        session.listener_handle.abort();
        let thread = session.thread;
        let thread_id = session.thread_id;
        let server = Arc::clone(&self.server);
        tokio::spawn(async move {
            let _ = thread.shutdown_and_wait().await;
            let _ = server.remove_thread(&thread_id).await;
        });
    }

    fn open_btw_loading_panel(&mut self) {
        self.chat_widget
            .show_selection_view(btw_loading_view_params());
    }

    fn open_btw_result_panel(&mut self, message: &str) {
        self.chat_widget
            .show_selection_view(btw_result_view_params(message));
    }
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
        side_content: ColumnRenderable::with(vec![
            Box::new("Temporary `/btw` discussion".to_string())
                as Box<dyn crate::render::renderable::Renderable>,
            Box::new(
                Paragraph::new(
                    "Codex is answering in a hidden ephemeral thread. Nothing will be written back \
                     to the main thread unless you explicitly choose an insert action."
                        .to_string(),
                )
                .wrap(Wrap { trim: false }),
            ) as Box<dyn crate::render::renderable::Renderable>,
        ])
        .into(),
        on_cancel: Some(Box::new(|tx| tx.send(AppEvent::BtwDiscard))),
        ..Default::default()
    }
}

fn btw_result_view_params(message: &str) -> SelectionViewParams {
    let preview = btw_preview_text(message);
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
                    "Destroy the temporary discussion and keep the main thread untouched."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::BtwDiscard))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        side_content: Paragraph::new(preview).wrap(Wrap { trim: false }).into(),
        on_cancel: Some(Box::new(|tx| tx.send(AppEvent::BtwDiscard))),
        ..Default::default()
    }
}

fn merge_developer_instructions(existing: Option<String>) -> String {
    match existing {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{BTW_DEVELOPER_INSTRUCTIONS}")
        }
        _ => BTW_DEVELOPER_INSTRUCTIONS.to_string(),
    }
}

async fn build_btw_initial_history(rollout_path: Option<&Path>) -> InitialHistory {
    let Some(rollout_path) = rollout_path else {
        return InitialHistory::New;
    };
    let Ok(history) = RolloutRecorder::get_rollout_history(rollout_path).await else {
        return InitialHistory::New;
    };
    let items = history.get_rollout_items();
    if items.is_empty() {
        return InitialHistory::New;
    }

    let session_meta = items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(_) => Some(item.clone()),
        _ => None,
    });
    let latest_turn_context_index = items
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, item)| matches!(item, RolloutItem::TurnContext(_)).then_some(index));
    let latest_turn_context = latest_turn_context_index.map(|index| items[index].clone());

    let mut used_tokens = 0usize;
    let mut selected_tail = Vec::new();
    for (index, item) in items.iter().enumerate().rev() {
        if matches!(item, RolloutItem::SessionMeta(_)) {
            continue;
        }
        if Some(index) == latest_turn_context_index {
            continue;
        }

        let item_tokens = approx_rollout_item_tokens(item);
        if !selected_tail.is_empty()
            && used_tokens.saturating_add(item_tokens) > BTW_CONTEXT_BUDGET_TOKENS
        {
            break;
        }
        used_tokens = used_tokens.saturating_add(item_tokens);
        selected_tail.push(item.clone());
    }
    selected_tail.reverse();

    let mut selected = Vec::new();
    if let Some(session_meta) = session_meta {
        selected.push(session_meta);
    }
    if let Some(turn_context) = latest_turn_context {
        selected.push(turn_context);
    }
    selected.extend(selected_tail);

    if selected.is_empty() {
        InitialHistory::New
    } else {
        InitialHistory::Forked(selected)
    }
}

fn approx_rollout_item_tokens(item: &RolloutItem) -> usize {
    serde_json::to_string(item)
        .ok()
        .map(|text| text.len().saturating_add(3) / 4)
        .unwrap_or(0)
}

fn btw_preview_text(message: &str) -> String {
    const PREVIEW_CHAR_LIMIT: usize = 1_200;

    let trimmed = message.trim();
    if trimmed.chars().count() <= PREVIEW_CHAR_LIMIT {
        return trimmed.to_string();
    }

    let preview: String = trimmed.chars().take(PREVIEW_CHAR_LIMIT).collect();
    format!("{preview}\n\n…preview truncated…")
}

fn summarize_btw_message(message: &str) -> String {
    const MAX_LINES: usize = 4;
    const MAX_CHARS: usize = 500;

    let mut kept = Vec::new();
    let mut used_chars = 0usize;
    for line in message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let next_len = used_chars.saturating_add(line.chars().count());
        if !kept.is_empty() && (kept.len() >= MAX_LINES || next_len > MAX_CHARS) {
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

#[cfg(test)]
mod tests {
    use super::btw_loading_view_params;
    use super::btw_result_view_params;
    use super::summarize_btw_message;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::list_selection_view::ListSelectionView;
    use crate::render::renderable::Renderable;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

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
    fn summarize_btw_message_keeps_short_prefix_for_insertion() {
        let summary = summarize_btw_message(
            "First point.\n\nSecond point.\nThird point.\nFourth point.\nFifth point.",
        );

        assert_eq!(
            summary,
            "BTW summary:\nFirst point.\nSecond point.\nThird point.\nFourth point."
        );
    }
}
