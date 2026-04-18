use super::App;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::insert_history::ScrollbackWrapMode;
use crate::tui;

pub(super) struct WorkflowController;

impl WorkflowController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::OpenWorkflowControls => {
                app.open_workflow_controls_popup();
            }
            AppEvent::OpenWorkflowControlView { destination } => {
                app.open_workflow_control_view(destination);
            }
            AppEvent::CreateDefaultWorkflowTemplate => {
                app.create_default_workflow_template_from_ui(tui).await;
            }
            AppEvent::EditWorkflowFile {
                workflow_path,
                reopen,
            } => {
                app.edit_workflow_file_from_ui(tui, workflow_path, reopen)
                    .await;
            }
            AppEvent::ToggleWorkflowTriggerEnabled {
                workflow_path,
                trigger_id,
            } => {
                app.toggle_workflow_trigger_enabled_from_ui(workflow_path, trigger_id);
            }
            AppEvent::ToggleWorkflowJobEnabled {
                workflow_path,
                job_name,
            } => {
                app.toggle_workflow_job_enabled_from_ui(workflow_path, job_name);
            }
            AppEvent::CycleWorkflowJobContext {
                workflow_path,
                job_name,
            } => {
                app.cycle_workflow_job_context_from_ui(workflow_path, job_name);
            }
            AppEvent::CycleWorkflowJobResponse {
                workflow_path,
                job_name,
            } => {
                app.cycle_workflow_job_response_from_ui(workflow_path, job_name);
            }
            AppEvent::EditWorkflowJobField {
                workflow_path,
                job_name,
                field,
            } => {
                app.edit_workflow_job_field_from_ui(tui, workflow_path, job_name, field)
                    .await;
            }
            AppEvent::SetWorkflowTriggerType {
                workflow_path,
                trigger_id,
                trigger_type,
            } => {
                app.set_workflow_trigger_type_from_ui(workflow_path, trigger_id, trigger_type);
            }
            AppEvent::EditWorkflowTriggerField {
                workflow_path,
                trigger_id,
                field,
            } => {
                app.edit_workflow_trigger_field_from_ui(tui, workflow_path, trigger_id, field)
                    .await;
            }
            AppEvent::WorkflowWorkspaceFilesChanged { changed_paths } => {
                let relevant_paths = changed_paths
                    .into_iter()
                    .filter(|path| {
                        super::workflow_file_watch::is_relevant_workspace_change(
                            app.config.cwd.as_path(),
                            path.as_path(),
                        )
                    })
                    .collect::<Vec<_>>();
                if !relevant_paths.is_empty() {
                    let cells = app.handle_workspace_file_changes_for_workflows(
                        app_server,
                        relevant_paths.as_slice(),
                    );
                    for cell in cells {
                        app.insert_visible_history_cell(tui, cell);
                    }
                }
            }
            AppEvent::StartManualWorkflowTrigger {
                workflow_name,
                trigger_id,
            } => {
                let cell = app.start_manual_workflow_trigger_from_ui(
                    app_server,
                    workflow_name,
                    trigger_id,
                );
                app.insert_visible_history_cell(tui, cell);
            }
            AppEvent::StartManualWorkflowJob {
                workflow_name,
                job_name,
            } => {
                let cell =
                    app.start_manual_workflow_job_from_ui(app_server, workflow_name, job_name);
                app.insert_visible_history_cell(tui, cell);
            }
            AppEvent::ShowWorkflowBackgroundTasks => {
                app.chat_widget.add_ps_output();
            }
            AppEvent::ReplayWorkflowHistory { thread_id } => {
                if app.active_thread_id == Some(thread_id) {
                    let lines = app.replay_workflow_history_cells_for_thread(
                        thread_id,
                        tui.terminal.last_known_screen_size.width,
                    );
                    if app.overlay.is_some() {
                        app.deferred_history_lines
                            .push((lines, ScrollbackWrapMode::Adaptive));
                    } else if !lines.is_empty() {
                        tui.insert_history_lines(lines);
                    }
                }
            }
            AppEvent::BackgroundWorkflowRunCompleted { run_id, result } => {
                let cells = app
                    .finish_background_workflow_run(app_server, run_id, *result)
                    .await;
                for cell in cells {
                    app.insert_visible_history_cell(tui, cell);
                }
            }
            _ => unreachable!("non-workflow event passed to workflow controller"),
        }
    }
}
