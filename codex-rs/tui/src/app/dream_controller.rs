use super::App;
use super::feature_dispatch::FeatureDispatchFuture;
use super::feature_dispatch::FeatureDispatchOutcome;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::tui;
use ratatui::style::Stylize;
use ratatui::text::Line;

pub(super) struct DreamController;

impl DreamController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::DreamSession => {
                let Some(thread_id) = app.active_thread_id else {
                    app.chat_widget
                        .add_error_message("No active thread is available for /dream.".to_string());
                    return;
                };
                let dream = match app_server.thread_dream_start(thread_id).await {
                    Ok(dream) => dream,
                    Err(err) => {
                        app.chat_widget
                            .add_error_message(format!("Failed to complete /dream: {err:#}"));
                        return;
                    }
                };
                app.start_fresh_session_with_summary_hint(tui, app_server)
                    .await;
                let mut lines: Vec<Line<'static>> = vec![
                    "Dream retrospective updated repo memory.".green().into(),
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
                lines.push("Next session hint:".into());
                lines.extend(
                    dream
                        .next_session_hint
                        .lines()
                        .map(|line| line.to_string().into()),
                );
                app.chat_widget.add_plain_history_lines(lines);
            }
            _ => unreachable!("non-dream event passed to dream controller"),
        }
    }
}

pub(super) fn matches_event(event: &AppEvent) -> bool {
    matches!(event, AppEvent::DreamSession)
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
