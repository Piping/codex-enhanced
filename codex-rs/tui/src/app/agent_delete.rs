use super::App;
use crate::app_event::AppEvent;
use crate::app_event::ThreadEvent;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::multi_agents::agent_picker_status_dot_spans;
use crate::tui;
use codex_protocol::ThreadId;

const DELETE_AGENT_PICKER_VIEW_ID: &str = "delete-agent-picker";
const DELETE_AGENT_CONFIRMATION_VIEW_ID: &str = "delete-agent-confirmation";

impl App {
    pub(crate) async fn open_delete_agent_picker(&mut self, app_server: &mut AppServerSession) {
        let mut thread_ids = self.agent_navigation.tracked_thread_ids();
        for thread_id in self.thread_event_channels.keys().copied() {
            if !thread_ids.contains(&thread_id) {
                thread_ids.push(thread_id);
            }
        }
        for thread_id in thread_ids {
            let _ = self
                .refresh_agent_picker_thread_liveness(
                    app_server,
                    thread_id,
                    super::ThreadLivenessRefreshMode::Picker,
                )
                .await;
        }

        if !self
            .agent_navigation
            .ordered_threads()
            .into_iter()
            .any(|(thread_id, entry)| self.primary_thread_id != Some(thread_id) && !entry.is_closed)
        {
            self.chat_widget.add_info_message(
                "No open agents available to delete.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        self.open_selection_popup_for_view(DELETE_AGENT_PICKER_VIEW_ID, delete_agent_picker_params);
    }

    pub(crate) fn open_delete_agent_confirmation(&mut self, thread_id: ThreadId) {
        if self.primary_thread_id == Some(thread_id) {
            self.chat_widget
                .add_error_message("The main thread cannot be deleted.".to_string());
            return;
        }

        let Some(entry) = self.agent_navigation.get(&thread_id) else {
            self.chat_widget
                .add_error_message(format!("Agent thread {thread_id} is no longer available."));
            return;
        };
        if entry.is_closed {
            self.chat_widget.add_error_message(format!(
                "{} is already closed and cannot be deleted from live agent state.",
                self.thread_label(thread_id)
            ));
            return;
        }

        let label = self.thread_label(thread_id);
        self.chat_widget
            .show_selection_view(delete_agent_confirmation_params(thread_id, &label));
    }

    pub(crate) async fn archive_agent_thread(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
    ) {
        if self.primary_thread_id == Some(thread_id) {
            self.chat_widget
                .add_error_message("The main thread cannot be deleted.".to_string());
            return;
        }

        let Some(entry) = self.agent_navigation.get(&thread_id) else {
            self.chat_widget
                .add_error_message(format!("Agent thread {thread_id} is no longer available."));
            return;
        };
        if entry.is_closed {
            self.chat_widget.add_error_message(format!(
                "{} is already closed and cannot be deleted from live agent state.",
                self.thread_label(thread_id)
            ));
            return;
        }

        let label = self.thread_label(thread_id);
        if self.current_displayed_thread_id() == Some(thread_id) {
            let Some(primary_thread_id) = self.primary_thread_id else {
                self.chat_widget.add_error_message(format!(
                    "Cannot delete {label} because the main thread is unavailable."
                ));
                return;
            };
            if let Err(err) = self
                .select_agent_thread(tui, app_server, primary_thread_id)
                .await
            {
                self.chat_widget
                    .add_error_message(format!("Failed to switch away from {label}: {err}"));
                return;
            }
        }

        self.finish_archiving_agent_thread(app_server, thread_id, label)
            .await;
    }

    async fn finish_archiving_agent_thread(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
        label: String,
    ) {
        if let Err(err) = app_server.thread_archive(thread_id).await {
            self.chat_widget
                .add_error_message(format!("Failed to archive {label}: {err}"));
            return;
        }

        if self.thread_event_channels.contains_key(&thread_id)
            && let Err(err) = app_server.thread_unsubscribe(thread_id).await
        {
            tracing::warn!("failed to unsubscribe archived thread {thread_id}: {err}");
        }

        self.forget_live_subagent_thread(thread_id).await;
        self.chat_widget
            .add_info_message(format!("Archived {label}."), /*hint*/ None);
    }
}

fn delete_agent_picker_params(
    app: &App,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    let mut selected_idx = initial_selected_idx.unwrap_or(0);
    let mut items: Vec<SelectionItem> = Vec::new();

    for (thread_id, entry) in app.agent_navigation.ordered_threads() {
        if app.primary_thread_id == Some(thread_id) || entry.is_closed {
            continue;
        }
        if initial_selected_idx.is_none() && app.active_thread_id == Some(thread_id) {
            selected_idx = items.len();
        }

        let name = app.thread_label(thread_id);
        let uuid = thread_id.to_string();
        items.push(SelectionItem {
            name: name.clone(),
            name_prefix_spans: agent_picker_status_dot_spans(/*is_closed*/ false),
            description: Some(uuid.clone()),
            is_current: app.active_thread_id == Some(thread_id),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::Thread(ThreadEvent::OpenDeleteAgentConfirmation {
                    thread_id,
                }));
            })],
            dismiss_on_select: false,
            dismiss_parent_on_child_accept: true,
            search_value: Some(format!("{name} {uuid}")),
            ..Default::default()
        });
    }

    let has_items = !items.is_empty();
    SelectionViewParams {
        view_id: Some(DELETE_AGENT_PICKER_VIEW_ID),
        title: Some("Delete Agent".to_string()),
        subtitle: Some("Select an open agent thread to archive.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search open agents".to_string()),
        initial_selected_idx: has_items.then_some(selected_idx),
        ..Default::default()
    }
}

fn delete_agent_confirmation_params(thread_id: ThreadId, label: &str) -> SelectionViewParams {
    let label = label.to_string();
    SelectionViewParams {
        view_id: Some(DELETE_AGENT_CONFIRMATION_VIEW_ID),
        title: Some("Archive Agent".to_string()),
        subtitle: Some(format!(
            "{label} will be removed from the live agent picker."
        )),
        footer_hint: Some(standard_popup_hint_line()),
        items: vec![
            SelectionItem {
                name: "Archive Agent".to_string(),
                description: Some("Stop tracking this open agent thread in Codex.".to_string()),
                selected_description: Some(format!("Archive {label} now.")),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::Thread(ThreadEvent::ArchiveAgentThread {
                        thread_id,
                    }));
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Cancel".to_string(),
                description: Some("Keep the agent thread open.".to_string()),
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::make_test_app;
    use super::delete_agent_confirmation_params;
    use super::delete_agent_picker_params;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
    use crate::multi_agents::AgentPickerThreadEntry;
    use crate::multi_agents::format_agent_picker_item_name;
    use crate::render::renderable::Renderable;
    use codex_protocol::ThreadId;
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

    #[tokio::test]
    async fn delete_agent_picker_popup_snapshot() {
        let mut app = make_test_app().await;
        let primary_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("thread id");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000123").expect("thread id");

        app.primary_thread_id = Some(primary_thread_id);
        app.active_thread_id = Some(agent_thread_id);
        app.agent_navigation.upsert(
            primary_thread_id,
            /*agent_nickname*/ None,
            /*agent_role*/ None,
            /*is_closed*/ false,
        );
        app.agent_navigation.upsert(
            agent_thread_id,
            Some("Scout".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );

        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            delete_agent_picker_params(&app, /*initial_selected_idx*/ None),
            tx,
            crate::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!(
            "delete_agent_picker_popup",
            render_selection_popup(&view, /*width*/ 92, /*height*/ 18)
        );
    }

    #[test]
    fn delete_agent_confirmation_popup_snapshot() {
        let thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000123").expect("thread id");
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            delete_agent_confirmation_params(thread_id, "Scout [worker]"),
            tx,
            crate::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!(
            "delete_agent_confirmation_popup",
            render_selection_popup(&view, /*width*/ 92, /*height*/ 16)
        );
    }

    #[tokio::test]
    async fn delete_agent_picker_params_excludes_primary_and_closed_threads() {
        let primary_thread_id = ThreadId::new();
        let open_thread_id = ThreadId::new();
        let closed_thread_id = ThreadId::new();
        let mut app = make_test_app().await;
        app.primary_thread_id = Some(primary_thread_id);
        app.active_thread_id = Some(open_thread_id);
        app.agent_navigation.upsert(
            primary_thread_id,
            /*agent_nickname*/ None,
            /*agent_role*/ None,
            /*is_closed*/ false,
        );
        app.agent_navigation.upsert(
            open_thread_id,
            Some("Scout".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );
        app.agent_navigation.upsert(
            closed_thread_id,
            Some("Closed".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ true,
        );

        let params = delete_agent_picker_params(&app, /*initial_selected_idx*/ None);

        assert_eq!(params.items.len(), 1);
        assert_eq!(
            params.items[0].name,
            format_agent_picker_item_name(Some("Scout"), Some("worker"), /*is_primary*/ false)
        );
        assert_eq!(
            app.agent_navigation.get(&closed_thread_id),
            Some(&AgentPickerThreadEntry {
                agent_nickname: Some("Closed".to_string()),
                agent_role: Some("worker".to_string()),
                is_closed: true,
            })
        );
    }
}
