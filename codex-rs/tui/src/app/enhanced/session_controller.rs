use color_eyre::eyre::Result;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::App;
use super::AppRunControl;
use super::ExitReason;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::cwd_prompt::CwdPromptAction;
use crate::resume_picker::SessionSelection;
use crate::tui;

pub(super) struct SessionController;

impl SessionController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) -> Result<Option<AppRunControl>> {
        match event {
            AppEvent::NewSession => {
                app.start_fresh_session_with_summary_hint(tui, app_server)
                    .await;
            }
            AppEvent::ClearUi => {
                app.clear_terminal_ui(tui, /*redraw_header*/ false)?;
                app.reset_app_ui_state_after_clear();

                app.start_fresh_session_with_summary_hint(tui, app_server)
                    .await;
            }
            AppEvent::OpenResumePicker => {
                let picker_app_server = match crate::start_app_server_for_picker(
                    &app.config,
                    &match app.remote_app_server_url.clone() {
                        Some(websocket_url) => crate::AppServerTarget::Remote {
                            websocket_url,
                            auth_token: app.remote_app_server_auth_token.clone(),
                        },
                        None => crate::AppServerTarget::Embedded,
                    },
                )
                .await
                {
                    Ok(app_server) => app_server,
                    Err(err) => {
                        app.chat_widget.add_error_message(format!(
                            "Failed to start TUI session picker: {err}"
                        ));
                        return Ok(None);
                    }
                };
                match crate::resume_picker::run_resume_picker_with_app_server(
                    tui,
                    &app.config,
                    crate::resume_picker::SessionPickerOrder::LocalGroupFirst,
                    crate::resume_picker::SessionPickerProviderScope::AllProviders,
                    /*include_non_interactive*/ true,
                    picker_app_server,
                )
                .await?
                {
                    SessionSelection::Resume(target_session) => {
                        let current_cwd = app.config.cwd.to_path_buf();
                        let resume_cwd = if app.remote_app_server_url.is_some() {
                            current_cwd.clone()
                        } else {
                            match crate::resolve_cwd_for_resume_or_fork(
                                tui,
                                &app.config,
                                &current_cwd,
                                target_session.thread_id,
                                target_session.path.as_deref(),
                                CwdPromptAction::Resume,
                                /*allow_prompt*/ true,
                            )
                            .await?
                            {
                                crate::ResolveCwdOutcome::Continue(Some(cwd)) => cwd,
                                crate::ResolveCwdOutcome::Continue(None) => current_cwd.clone(),
                                crate::ResolveCwdOutcome::Exit => {
                                    return Ok(Some(AppRunControl::Exit(
                                        ExitReason::UserRequested,
                                    )));
                                }
                            }
                        };
                        let mut resume_config = match app
                            .rebuild_config_for_resume_or_fallback(&current_cwd, resume_cwd)
                            .await
                        {
                            Ok(cfg) => cfg,
                            Err(err) => {
                                app.chat_widget.add_error_message(format!(
                                    "Failed to rebuild configuration for resume: {err}"
                                ));
                                return Ok(None);
                            }
                        };
                        app.apply_runtime_policy_overrides(&mut resume_config);
                        let summary = super::session_summary(
                            app.chat_widget.token_usage(),
                            app.chat_widget.thread_id(),
                            app.chat_widget.thread_name(),
                        );
                        match app_server
                            .resume_thread(resume_config.clone(), target_session.thread_id)
                            .await
                        {
                            Ok(resumed) => {
                                app.shutdown_current_thread(app_server).await;
                                app.config = resume_config;
                                tui.set_notification_method(app.config.tui_notification_method);
                                app.file_search
                                    .update_search_dir(app.config.cwd.to_path_buf());
                                match app
                                    .replace_chat_widget_with_app_server_thread(
                                        tui, app_server, resumed,
                                    )
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
                                            "Failed to attach to resumed app-server thread: {err}"
                                        ));
                                    }
                                }
                            }
                            Err(err) => {
                                let path_display = target_session.display_label();
                                app.chat_widget.add_error_message(format!(
                                    "Failed to resume session from {path_display}: {err}"
                                ));
                            }
                        }
                    }
                    SessionSelection::Exit
                    | SessionSelection::StartFresh
                    | SessionSelection::Fork(_) => {}
                }

                // Leaving alt-screen may blank the inline viewport; force a redraw either way.
                tui.frame_requester().schedule_frame();
            }
            _ => unreachable!("non-session event passed to session controller"),
        }

        Ok(None)
    }
}
