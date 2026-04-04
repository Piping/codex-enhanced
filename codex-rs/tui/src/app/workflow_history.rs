use super::App;
use crate::app_event::AppEvent;
use crate::history_cell;
use crate::history_cell::AgentMessageCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::PlainHistoryCell;
use crate::markdown::append_markdown;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use ratatui::text::Line;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct WorkflowHistoryState {
    pub(super) thread_history_cells: HashMap<ThreadId, Vec<Arc<dyn HistoryCell>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowReplySource {
    workflow_id: String,
    action: Option<String>,
}

impl WorkflowReplySource {
    pub(crate) fn new(workflow_id: String, action: Option<String>) -> Self {
        Self {
            workflow_id,
            action,
        }
    }

    pub(crate) fn hint(&self) -> String {
        match self
            .action
            .as_deref()
            .map(str::trim)
            .filter(|action| !action.is_empty())
        {
            Some(action) => format!("{} · {}", self.workflow_id, workflow_prompt_prefix(action)),
            None => self.workflow_id.clone(),
        }
    }
}

impl App {
    pub(crate) fn queue_workflow_history_replay_for_thread(&self, thread_id: ThreadId) {
        if self
            .workflow_history
            .thread_history_cells
            .contains_key(&thread_id)
        {
            self.app_event_tx
                .send(AppEvent::ReplayWorkflowHistory { thread_id });
        }
    }

    pub(crate) fn replay_workflow_history_cells_for_thread(
        &mut self,
        thread_id: ThreadId,
        width: u16,
    ) -> Vec<Line<'static>> {
        let Some(cells) = self.workflow_history.thread_history_cells.get(&thread_id) else {
            return Vec::new();
        };

        let mut rendered = Vec::new();
        for cell in cells {
            self.transcript_cells.push(cell.clone());
            let mut display = cell.display_lines(width);
            if display.is_empty() {
                continue;
            }
            if !cell.is_stream_continuation() {
                if self.has_emitted_history_lines {
                    display.insert(0, Line::default());
                } else {
                    self.has_emitted_history_lines = true;
                }
            }
            rendered.extend(display);
        }
        rendered
    }

    pub(crate) fn record_workflow_history_cell(
        &mut self,
        thread_id: ThreadId,
        cell: Arc<dyn HistoryCell>,
    ) -> Option<Arc<dyn HistoryCell>> {
        self.workflow_history
            .thread_history_cells
            .entry(thread_id)
            .or_default()
            .push(cell.clone());
        (self.active_thread_id == Some(thread_id)).then_some(cell)
    }

    pub(crate) fn queue_workflow_followup_to_primary(
        &mut self,
        text: String,
        source: WorkflowReplySource,
    ) -> Option<Arc<dyn HistoryCell>> {
        let Some(primary_thread_id) = self.primary_thread_id else {
            self.chat_widget.add_error_message(
                "Failed to find the main thread for background follow-up.".to_string(),
            );
            return None;
        };

        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }

        let origin_cell: Arc<dyn HistoryCell> = Arc::new(workflow_info_cell(&source));
        let visible_cell = self.record_workflow_history_cell(primary_thread_id, origin_cell);
        let Some(op) = self.workflow_followup_user_turn(trimmed) else {
            self.chat_widget.add_error_message(
                "Failed to build the main-thread follow-up for the workflow.".to_string(),
            );
            return visible_cell;
        };
        self.app_event_tx.send(AppEvent::SubmitThreadOp {
            thread_id: primary_thread_id,
            op,
        });
        visible_cell
    }

    fn workflow_followup_user_turn(&self, text: String) -> Option<Op> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }

        let session = self.primary_session_configured.as_ref();
        let cwd = session
            .map(|session| session.cwd.clone())
            .unwrap_or_else(|| self.config.cwd.to_path_buf());
        let approval_policy = session
            .map(|session| session.approval_policy)
            .unwrap_or_else(|| self.config.permissions.approval_policy.value());
        let approvals_reviewer = session.map(|session| session.approvals_reviewer);
        let sandbox_policy = session
            .map(|session| session.sandbox_policy.clone())
            .unwrap_or_else(|| self.config.permissions.sandbox_policy.get().clone());
        let model = session
            .map(|session| session.model.clone())
            .filter(|model| !model.trim().is_empty())
            .or_else(|| self.config.model.clone())
            .unwrap_or_else(|| self.chat_widget.current_model().to_string());
        let effort = session.and_then(|session| session.reasoning_effort);
        let service_tier = session.and_then(|session| session.service_tier.map(Some));

        Some(Op::UserTurn {
            items: vec![UserInput::Text {
                text,
                text_elements: Vec::new(),
            }],
            cwd,
            approval_policy,
            approvals_reviewer,
            sandbox_policy,
            model,
            effort,
            summary: None,
            service_tier,
            final_output_json_schema: None,
            collaboration_mode: None,
            personality: self.config.personality,
        })
    }
}

pub(crate) fn workflow_result_cell(message: &str, cwd: &Path) -> AgentMessageCell {
    let mut rendered = vec![Line::default()];
    append_markdown(message, /*width*/ None, Some(cwd), &mut rendered);
    AgentMessageCell::new(rendered, /*is_first_line*/ false)
}

fn workflow_info_cell(source: &WorkflowReplySource) -> PlainHistoryCell {
    history_cell::new_info_event("Workflow reply".to_string(), Some(source.hint()))
}

fn workflow_prompt_prefix(prompt: &str) -> String {
    let prefix = prompt.chars().take(48).collect::<String>();
    if prompt.chars().count() > 48 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowReplySource;
    use pretty_assertions::assert_eq;

    #[test]
    fn workflow_reply_source_hint_prefers_action_when_present() {
        let source = WorkflowReplySource::new(
            "director/review:summary".to_string(),
            Some("summarize it".to_string()),
        );
        assert_eq!(
            source.hint(),
            "director/review:summary · summarize it".to_string()
        );
    }
}
