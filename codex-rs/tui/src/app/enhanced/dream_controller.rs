use super::App;
use super::feature_dispatch::FeatureDispatchFuture;
use super::feature_dispatch::FeatureDispatchOutcome;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::tui;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadDreamStartParams;
use codex_app_server_protocol::ThreadDreamStartResponse;
use codex_protocol::ThreadId;
use ratatui::style::Stylize;
use ratatui::text::Line;
use uuid::Uuid;

pub(super) struct DreamController;

impl DreamController {
    pub(super) fn begin(app: &mut App, app_server: &AppServerSession) {
        let Some(thread_id) = app.active_thread_id else {
            app.chat_widget
                .add_error_message("No active thread is available for /dream.".to_string());
            return;
        };

        if app.pending_dream_thread_id.is_some() {
            app.chat_widget
                .add_error_message("A /dream retrospective is already running.".to_string());
            return;
        }

        app.pending_dream_thread_id = Some(thread_id);
        app.chat_widget.add_info_message(
            "Running /dream retrospective in the background.".to_string(),
            Some(
                "Stay on this thread to start a fresh session automatically when it completes."
                    .to_string(),
            ),
        );

        let request_handle = app_server.request_handle();
        let app_event_tx = app.app_event_tx.clone();
        tokio::spawn(async move {
            let result = request_handle
                .request_typed(ClientRequest::ThreadDreamStart {
                    request_id: dream_request_id(),
                    params: ThreadDreamStartParams {
                        thread_id: thread_id.to_string(),
                    },
                })
                .await
                .map_err(|err| format!("thread/dream/start failed in TUI: {err}"));
            app_event_tx.send(AppEvent::DreamSessionCompleted {
                thread_id,
                result: Box::new(result),
            });
        });
    }

    async fn finish(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
        result: Result<ThreadDreamStartResponse, String>,
    ) {
        if app.pending_dream_thread_id != Some(thread_id) {
            return;
        }
        app.pending_dream_thread_id = None;

        let dream = match result {
            Ok(dream) => dream,
            Err(err) => {
                app.chat_widget
                    .add_error_message(format!("Failed to complete /dream: {err}"));
                return;
            }
        };

        if app.active_thread_id == Some(thread_id) {
            app.start_fresh_session_with_summary_hint(tui, app_server)
                .await;
            app.chat_widget
                .add_plain_history_lines(dream_completion_lines(
                    dream,
                    "Dream retrospective updated repo memory.".green().into(),
                    None,
                ));
            return;
        }

        app.chat_widget.add_plain_history_lines(dream_completion_lines(
            dream,
            "Dream retrospective finished, but TUI kept the current session."
                .bold()
                .into(),
            Some(
                "The active thread changed before /dream completed, so no fresh session was started."
                    .to_string(),
            ),
        ));
    }

    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::DreamSession => Self::begin(app, app_server),
            AppEvent::DreamSessionCompleted { thread_id, result } => {
                Self::finish(app, tui, app_server, thread_id, *result).await;
            }
            _ => unreachable!("non-dream event passed to dream controller"),
        }
    }
}

pub(super) fn matches_event(event: &AppEvent) -> bool {
    matches!(
        event,
        AppEvent::DreamSession | AppEvent::DreamSessionCompleted { .. }
    )
}

pub(super) fn dispatch<'a>(
    app: &'a mut App,
    tui: &'a mut tui::Tui,
    app_server: &'a mut AppServerSession,
    event: AppEvent,
) -> FeatureDispatchFuture<'a> {
    Box::pin(async move {
        DreamController::handle(app, tui, app_server, event).await;
        Ok(FeatureDispatchOutcome::Handled)
    })
}

fn dream_request_id() -> RequestId {
    RequestId::String(format!("dream-{}", Uuid::new_v4()))
}

fn dream_completion_lines(
    dream: ThreadDreamStartResponse,
    heading: Line<'static>,
    note: Option<String>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = vec![
        heading,
        format!("Memory root: {}", dream.memory_root).into(),
        format!("Retrospective: {}", dream.retrospective_path).into(),
        format!("AGENTS.md: {}", dream.updated_agents_path).into(),
    ];
    if dream.updated_skill_paths.is_empty() {
        lines.push("Skills: none updated".dim().into());
    } else {
        lines.push("Updated skills:".into());
        lines.extend(
            dream
                .updated_skill_paths
                .into_iter()
                .map(|path| format!("  - {path}").into()),
        );
    }
    if let Some(note) = note {
        lines.push(note.dim().into());
    }
    lines.push("Next session hint:".into());
    lines.extend(
        dream
            .next_session_hint
            .lines()
            .map(|line| line.to_string().into()),
    );
    lines
}
