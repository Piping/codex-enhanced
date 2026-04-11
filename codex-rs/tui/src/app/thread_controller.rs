use ratatui::style::Stylize;
use ratatui::text::Line;

use super::App;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::pager_overlay::Overlay;
use crate::tui;

pub(super) struct ThreadController;

impl ThreadController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::OpenThreadPanel => {
                app.open_thread_panel();
            }
            AppEvent::OpenJumpToMessagePanel => {
                app.open_jump_to_message_panel();
            }
            AppEvent::JumpToTranscriptCell { cell_index } => {
                app.reset_backtrack_state();
                app.backtrack.overlay_preview_active = false;
                if !matches!(app.overlay, Some(Overlay::Transcript(_))) {
                    app.open_transcript_overlay(tui);
                }
                if let Some(Overlay::Transcript(overlay)) = &mut app.overlay
                    && cell_index < app.transcript_cells.len()
                {
                    overlay.set_highlight_cell(Some(cell_index));
                    tui.frame_requester().schedule_frame();
                }
            }
            AppEvent::ForkCurrentSession => {
                app.session_telemetry.counter(
                    "codex.thread.fork",
                    /*inc*/ 1,
                    &[("source", "slash_command")],
                );
                let summary = super::session_summary(
                    app.chat_widget.token_usage(),
                    app.chat_widget.thread_id(),
                    app.chat_widget.thread_name(),
                );
                app.chat_widget
                    .add_plain_history_lines(vec!["/fork".magenta().into()]);
                if let Some(thread_id) = app.chat_widget.thread_id() {
                    app.refresh_in_memory_config_from_disk_best_effort("forking the thread")
                        .await;
                    match app_server.fork_thread(app.config.clone(), thread_id).await {
                        Ok(forked) => {
                            app.shutdown_current_thread(app_server).await;
                            match app
                                .replace_chat_widget_with_app_server_thread(tui, app_server, forked)
                                .await
                            {
                                Ok(()) => {
                                    if let Some(summary) = summary {
                                        let mut lines: Vec<Line<'static>> =
                                            vec![summary.usage_line.clone().into()];
                                        if let Some(command) = summary.resume_command {
                                            let spans = vec![
                                                "To continue this session, run ".into(),
                                                command.cyan(),
                                            ];
                                            lines.push(spans.into());
                                        }
                                        app.chat_widget.add_plain_history_lines(lines);
                                    }
                                }
                                Err(err) => {
                                    app.chat_widget.add_error_message(format!(
                                        "Failed to attach to forked app-server thread: {err}"
                                    ));
                                }
                            }
                        }
                        Err(err) => {
                            app.chat_widget.add_error_message(format!(
                                "Failed to fork current session through the app server: {err}"
                            ));
                        }
                    }
                } else {
                    app.chat_widget.add_error_message(
                        "A thread must contain at least one turn before it can be forked."
                            .to_string(),
                    );
                }

                tui.frame_requester().schedule_frame();
            }
            AppEvent::UndoLastUserMessage => {
                app.undo_last_user_message();
            }
            _ => unreachable!("non-thread event passed to thread controller"),
        }
    }
}
