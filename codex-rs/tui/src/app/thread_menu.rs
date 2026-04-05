use std::sync::Arc;

use super::App;
use super::jump_navigation::build_jump_targets;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell::HistoryCell;

const THREAD_PANEL_VIEW_ID: &str = "thread-actions";
const JUMP_TO_MESSAGE_VIEW_ID: &str = "jump-to-message";

impl App {
    pub(crate) fn open_thread_panel(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(THREAD_PANEL_VIEW_ID);
        let params = thread_panel_params(self.chat_widget.is_task_running(), initial_selected_idx);
        if !self
            .chat_widget
            .replace_selection_view_if_active(THREAD_PANEL_VIEW_ID, params)
        {
            self.chat_widget.show_selection_view(thread_panel_params(
                self.chat_widget.is_task_running(),
                initial_selected_idx,
            ));
        }
    }

    pub(crate) fn open_jump_to_message_panel(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(JUMP_TO_MESSAGE_VIEW_ID);
        let params = jump_to_message_panel_params(&self.transcript_cells, initial_selected_idx);
        if !self
            .chat_widget
            .replace_selection_view_if_active(JUMP_TO_MESSAGE_VIEW_ID, params)
        {
            self.chat_widget
                .show_selection_view(jump_to_message_panel_params(
                    &self.transcript_cells,
                    initial_selected_idx,
                ));
        }
    }
}

fn thread_panel_params(
    task_running: bool,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    let mut items = vec![SelectionItem {
        name: "Fork Current Session".to_string(),
        description: Some("Create a new thread from the current session state.".to_string()),
        selected_description: Some(
            "Fork the current thread into a new session and continue there.".to_string(),
        ),
        actions: vec![Box::new(|tx| tx.send(AppEvent::ForkCurrentSession))],
        dismiss_on_select: true,
        is_disabled: task_running,
        disabled_reason: task_running
            .then_some("Wait for the current task to finish before forking.".to_string()),
        ..Default::default()
    }];

    items.push(SelectionItem {
        name: "Jump To Message".to_string(),
        description: Some(
            "Search committed transcript entries and open the transcript overlay.".to_string(),
        ),
        selected_description: Some(
            "Search the current transcript and jump directly to a committed entry.".to_string(),
        ),
        actions: vec![Box::new(|tx| tx.send(AppEvent::OpenJumpToMessagePanel))],
        dismiss_on_select: false,
        ..Default::default()
    });

    items.push(SelectionItem {
        name: "Undo Last User Message".to_string(),
        description: Some("Restore the last sent input and roll back one turn.".to_string()),
        selected_description: Some(
            "Restore the latest user input to the composer and rewind one committed turn."
                .to_string(),
        ),
        actions: vec![Box::new(|tx| tx.send(AppEvent::UndoLastUserMessage))],
        dismiss_on_select: true,
        is_disabled: task_running,
        disabled_reason: task_running
            .then_some("Wait for the current task to finish before undoing a turn.".to_string()),
        ..Default::default()
    });

    SelectionViewParams {
        view_id: Some(THREAD_PANEL_VIEW_ID),
        title: Some("Thread".to_string()),
        subtitle: Some("Fork, rewind, or jump within the current conversation.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        initial_selected_idx,
        ..Default::default()
    }
}

fn jump_to_message_panel_params(
    transcript_cells: &[Arc<dyn HistoryCell>],
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    let targets = build_jump_targets(transcript_cells);
    let subtitle = if targets.is_empty() {
        Some("No committed transcript entries are available yet.".to_string())
    } else {
        Some(format!(
            "{} committed transcript entr{} available.",
            targets.len(),
            if targets.len() == 1 {
                "y is"
            } else {
                "ies are"
            },
        ))
    };
    let items = if targets.is_empty() {
        vec![SelectionItem {
            name: "Nothing to jump to yet".to_string(),
            description: Some(
                "Send a message or wait for a response before using Jump To Message.".to_string(),
            ),
            is_disabled: true,
            ..Default::default()
        }]
    } else {
        targets
            .into_iter()
            .map(|target| {
                let cell_index = target.cell_index;
                let search_value = target.search_value();
                let name = target.title;
                let description = target.preview;
                SelectionItem {
                    name,
                    description: Some(description),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::JumpToTranscriptCell { cell_index });
                    })],
                    dismiss_on_select: true,
                    search_value: Some(search_value),
                    ..Default::default()
                }
            })
            .collect()
    };

    SelectionViewParams {
        view_id: Some(JUMP_TO_MESSAGE_VIEW_ID),
        title: Some("Jump To Message".to_string()),
        subtitle,
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search committed transcript".to_string()),
        initial_selected_idx,
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
    use std::sync::Arc;
    use tokio::sync::mpsc::unbounded_channel;

    use super::jump_to_message_panel_params;
    use super::thread_panel_params;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
    use crate::history_cell::AgentMessageCell;
    use crate::history_cell::HistoryCell;
    use crate::history_cell::UserHistoryCell;
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
    fn thread_panel_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(thread_panel_params(/*task_running*/ false, None), tx);

        assert_snapshot!("thread_panel_popup", render_selection_popup(&view, 92, 20));
    }

    #[test]
    fn thread_panel_disables_mutations_while_task_running() {
        let params = thread_panel_params(/*task_running*/ true, None);

        assert_eq!(params.items[0].is_disabled, true);
        assert_eq!(params.items[1].is_disabled, false);
        assert_eq!(params.items[2].is_disabled, true);
    }

    #[test]
    fn jump_to_message_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let cells = vec![
            Arc::new(UserHistoryCell {
                message: "How do I keep the retry chain intact?".to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn HistoryCell>,
            Arc::new(AgentMessageCell::new(
                vec!["Classify 503 as fallback-eligible before surfacing the final error.".into()],
                /*is_first_line*/ true,
            )) as Arc<dyn HistoryCell>,
        ];
        let view = ListSelectionView::new(jump_to_message_panel_params(&cells, None), tx);

        assert_snapshot!(
            "jump_to_message_popup",
            render_selection_popup(&view, 92, 22)
        );
    }

    #[test]
    fn jump_to_message_empty_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let cells: Vec<Arc<dyn HistoryCell>> = Vec::new();
        let view = ListSelectionView::new(jump_to_message_panel_params(&cells, None), tx);

        assert_snapshot!(
            "jump_to_message_empty_popup",
            render_selection_popup(&view, 92, 18)
        );
    }
}
