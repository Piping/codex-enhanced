use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use crate::app_event::AppEvent;
use crate::app_event::WorkflowControlsDestination;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionAction;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::tui;

use super::App;
use super::editor_helpers::ExternalEditorErrorTarget;
use super::workflow_definition::WorkflowControlsAction;
use super::workflow_definition::WorkflowControlsItem;
use super::workflow_definition::WorkflowControlsMenu;
use super::workflow_definition::WorkflowJobEditableField;
use super::workflow_definition::WorkflowTriggerEditableField;
use super::workflow_definition::WorkflowTriggerType;
use super::workflow_definition::create_default_workflow_template;
use super::workflow_definition::cycle_job_context;
use super::workflow_definition::cycle_job_response;
use super::workflow_definition::job_field_seed;
use super::workflow_definition::set_trigger_type;
use super::workflow_definition::toggle_job_enabled;
use super::workflow_definition::toggle_trigger_enabled;
use super::workflow_definition::trigger_field_seed;
use super::workflow_definition::workflow_context_label;
use super::workflow_definition::workflow_file_controls_menu;
use super::workflow_definition::workflow_job_controls_menu;
use super::workflow_definition::workflow_job_field_label;
use super::workflow_definition::workflow_jobs_controls_menu;
use super::workflow_definition::workflow_manual_trigger_controls_menu;
use super::workflow_definition::workflow_manual_triggers_controls_menu;
use super::workflow_definition::workflow_response_label;
use super::workflow_definition::workflow_root_controls_menu;
use super::workflow_definition::workflow_trigger_field_label;
use super::workflow_definition::workflow_trigger_type_controls_menu;
use super::workflow_definition::workflow_trigger_type_label;
use super::workflow_definition::write_job_field;
use super::workflow_definition::write_trigger_field;

const WORKFLOW_CONTROLS_VIEW_ID: &str = "workflow-controls";

impl App {
    pub(crate) fn open_workflow_controls_popup(&mut self) {
        self.open_workflow_control_view(WorkflowControlsDestination::Root);
    }

    pub(crate) fn open_workflow_control_view(&mut self, destination: WorkflowControlsDestination) {
        self.open_selection_popup_for_view(
            WORKFLOW_CONTROLS_VIEW_ID,
            |app, active_selected_idx| match destination {
                WorkflowControlsDestination::Root => {
                    app.workflow_root_popup_params(active_selected_idx)
                }
                WorkflowControlsDestination::File { ref workflow_path } => {
                    app.workflow_file_popup_params(workflow_path.as_path(), Some(0))
                }
                WorkflowControlsDestination::Jobs { ref workflow_path } => {
                    app.workflow_jobs_popup_params(workflow_path.as_path(), Some(0))
                }
                WorkflowControlsDestination::Job {
                    ref workflow_path,
                    ref job_name,
                } => app.workflow_job_popup_params(workflow_path.as_path(), job_name, Some(0)),
                WorkflowControlsDestination::ManualTriggers { ref workflow_path } => {
                    app.workflow_manual_triggers_popup_params(workflow_path.as_path(), Some(0))
                }
                WorkflowControlsDestination::ManualTrigger {
                    ref workflow_path,
                    ref trigger_id,
                } => app.workflow_manual_trigger_popup_params(
                    workflow_path.as_path(),
                    trigger_id,
                    Some(0),
                ),
                WorkflowControlsDestination::TriggerType {
                    ref workflow_path,
                    ref trigger_id,
                } => app.workflow_trigger_type_popup_params(
                    workflow_path.as_path(),
                    trigger_id,
                    Some(0),
                ),
            },
        );
    }

    pub(crate) fn refresh_workflow_controls_if_active(&mut self) {
        let _ = self
            .refresh_selection_popup_if_active(WORKFLOW_CONTROLS_VIEW_ID, |app, selected_idx| {
                app.workflow_root_popup_params(Some(selected_idx))
            });
    }

    pub(crate) fn start_manual_workflow_trigger_from_ui(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        trigger_id: String,
    ) -> Arc<dyn HistoryCell> {
        let cell = self.start_manual_workflow_trigger_run(app_server, workflow_name, trigger_id);
        self.refresh_workflow_controls_if_active();
        cell
    }

    pub(crate) fn start_manual_workflow_job_from_ui(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        job_name: String,
    ) -> Arc<dyn HistoryCell> {
        let cell = self.start_manual_workflow_job_run(app_server, workflow_name, job_name);
        self.refresh_workflow_controls_if_active();
        cell
    }

    pub(crate) async fn create_default_workflow_template_from_ui(&mut self, tui: &mut tui::Tui) {
        match create_default_workflow_template(self.config.cwd.as_path()) {
            Ok(workflow_path) => {
                self.chat_widget.add_info_message(
                    format!("Created workflow template at {}.", workflow_path.display()),
                    /*hint*/ None,
                );
                self.edit_workflow_file_from_ui(
                    tui,
                    workflow_path.clone(),
                    WorkflowControlsDestination::File { workflow_path },
                )
                .await;
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) async fn edit_workflow_file_from_ui(
        &mut self,
        tui: &mut tui::Tui,
        workflow_path: PathBuf,
        reopen: WorkflowControlsDestination,
    ) {
        if self
            .edit_file_with_external_editor(
                tui,
                ExternalEditorErrorTarget::History,
                workflow_path.as_path(),
            )
            .await
            .is_ok()
        {
            self.open_workflow_control_view(reopen);
        }
    }

    pub(crate) async fn edit_workflow_job_field_from_ui(
        &mut self,
        tui: &mut tui::Tui,
        workflow_path: PathBuf,
        job_name: String,
        field: WorkflowJobEditableField,
    ) {
        let seed = match job_field_seed(workflow_path.as_path(), &job_name, field) {
            Ok(seed) => seed,
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
                return;
            }
        };
        let suffix = match field {
            WorkflowJobEditableField::Needs => ".yaml",
            WorkflowJobEditableField::Steps => ".yaml",
        };
        let Ok(updated) = self
            .edit_seed_with_external_editor(tui, ExternalEditorErrorTarget::History, &seed, suffix)
            .await
        else {
            return;
        };
        match write_job_field(workflow_path.as_path(), &job_name, field, &updated) {
            Ok(()) => {
                self.chat_widget.add_info_message(
                    format!(
                        "Updated `{}` for workflow job `{job_name}`.",
                        workflow_job_field_label(field)
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::Job {
                    workflow_path,
                    job_name,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) async fn edit_workflow_trigger_field_from_ui(
        &mut self,
        tui: &mut tui::Tui,
        workflow_path: PathBuf,
        trigger_id: String,
        field: WorkflowTriggerEditableField,
    ) {
        let seed = match trigger_field_seed(workflow_path.as_path(), &trigger_id, field) {
            Ok(seed) => seed,
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
                return;
            }
        };
        let suffix = if matches!(field, WorkflowTriggerEditableField::Jobs) {
            ".yaml"
        } else {
            ".txt"
        };
        let Ok(updated) = self
            .edit_seed_with_external_editor(tui, ExternalEditorErrorTarget::History, &seed, suffix)
            .await
        else {
            return;
        };
        match write_trigger_field(workflow_path.as_path(), &trigger_id, field, &updated) {
            Ok(next_trigger_id) => {
                self.chat_widget.add_info_message(
                    format!(
                        "Updated `{}` for workflow trigger `{next_trigger_id}`.",
                        workflow_trigger_field_label(field)
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::ManualTrigger {
                    workflow_path,
                    trigger_id: next_trigger_id,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) fn toggle_workflow_job_enabled_from_ui(
        &mut self,
        workflow_path: PathBuf,
        job_name: String,
    ) {
        match toggle_job_enabled(workflow_path.as_path(), &job_name) {
            Ok(enabled) => {
                self.chat_widget.add_info_message(
                    format!(
                        "{} workflow job `{job_name}`.",
                        if enabled { "Enabled" } else { "Disabled" }
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::Job {
                    workflow_path,
                    job_name,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) fn toggle_workflow_trigger_enabled_from_ui(
        &mut self,
        workflow_path: PathBuf,
        trigger_id: String,
    ) {
        match toggle_trigger_enabled(workflow_path.as_path(), &trigger_id) {
            Ok(enabled) => {
                self.chat_widget.add_info_message(
                    format!(
                        "{} workflow trigger `{trigger_id}`.",
                        if enabled { "Enabled" } else { "Disabled" }
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::ManualTrigger {
                    workflow_path,
                    trigger_id,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) fn set_workflow_trigger_type_from_ui(
        &mut self,
        workflow_path: PathBuf,
        trigger_id: String,
        trigger_type: WorkflowTriggerType,
    ) {
        match set_trigger_type(workflow_path.as_path(), &trigger_id, trigger_type) {
            Ok(next_trigger_id) => {
                self.chat_widget.add_info_message(
                    format!(
                        "Workflow trigger `{next_trigger_id}` now uses {}.",
                        workflow_trigger_type_label(trigger_type)
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::ManualTrigger {
                    workflow_path,
                    trigger_id: next_trigger_id,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) fn cycle_workflow_job_context_from_ui(
        &mut self,
        workflow_path: PathBuf,
        job_name: String,
    ) {
        match cycle_job_context(workflow_path.as_path(), &job_name) {
            Ok(context) => {
                self.chat_widget.add_info_message(
                    format!(
                        "Workflow job `{job_name}` now uses {} context.",
                        workflow_context_label(context)
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::Job {
                    workflow_path,
                    job_name,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    pub(crate) fn cycle_workflow_job_response_from_ui(
        &mut self,
        workflow_path: PathBuf,
        job_name: String,
    ) {
        match cycle_job_response(workflow_path.as_path(), &job_name) {
            Ok(response) => {
                self.chat_widget.add_info_message(
                    format!(
                        "Workflow job `{job_name}` now delivers a {} response.",
                        workflow_response_label(response)
                    ),
                    /*hint*/ None,
                );
                self.open_workflow_control_view(WorkflowControlsDestination::Job {
                    workflow_path,
                    job_name,
                });
            }
            Err(err) => {
                self.chat_widget
                    .add_to_history(history_cell::new_error_event(err));
            }
        }
    }

    fn workflow_root_popup_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let running_labels = self.background_workflow_labels();
        let queued_labels = self.queued_trigger_labels();
        workflow_menu_popup_params(
            workflow_root_controls_menu(self.config.cwd.as_path(), &running_labels, &queued_labels),
            initial_selected_idx,
        )
    }

    fn workflow_file_popup_params(
        &self,
        workflow_path: &Path,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        match workflow_file_controls_menu(self.config.cwd.as_path(), workflow_path) {
            Ok(menu) => workflow_menu_popup_params(menu, initial_selected_idx),
            Err(err) => workflow_error_popup_params(
                "Workflow",
                "Failed to open workflow file.",
                err,
                workflow_back_item(WorkflowControlsDestination::Root),
                initial_selected_idx,
            ),
        }
    }

    fn workflow_jobs_popup_params(
        &self,
        workflow_path: &Path,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let running_labels = self.background_workflow_labels();
        let queued_labels = self.queued_trigger_labels();
        match workflow_jobs_controls_menu(
            self.config.cwd.as_path(),
            workflow_path,
            &running_labels,
            &queued_labels,
        ) {
            Ok(menu) => workflow_menu_popup_params(menu, initial_selected_idx),
            Err(err) => workflow_error_popup_params(
                "Workflow Jobs",
                "Failed to load workflow jobs.",
                err,
                workflow_back_item(WorkflowControlsDestination::Root),
                initial_selected_idx,
            ),
        }
    }

    fn workflow_manual_triggers_popup_params(
        &self,
        workflow_path: &Path,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let running_labels = self.background_workflow_labels();
        let queued_labels = self.queued_trigger_labels();
        match workflow_manual_triggers_controls_menu(
            self.config.cwd.as_path(),
            workflow_path,
            &running_labels,
            &queued_labels,
        ) {
            Ok(menu) => workflow_menu_popup_params(menu, initial_selected_idx),
            Err(err) => workflow_error_popup_params(
                "Workflow Triggers",
                "Failed to load workflow triggers.",
                err,
                workflow_back_item(WorkflowControlsDestination::Root),
                initial_selected_idx,
            ),
        }
    }

    fn workflow_manual_trigger_popup_params(
        &self,
        workflow_path: &Path,
        trigger_id: &str,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        match workflow_manual_trigger_controls_menu(
            self.config.cwd.as_path(),
            workflow_path,
            trigger_id,
        ) {
            Ok(menu) => workflow_menu_popup_params(menu, initial_selected_idx),
            Err(err) => workflow_error_popup_params(
                "Workflow Trigger",
                "Failed to load workflow trigger.",
                err,
                workflow_back_item(WorkflowControlsDestination::Root),
                initial_selected_idx,
            ),
        }
    }

    fn workflow_trigger_type_popup_params(
        &self,
        workflow_path: &Path,
        trigger_id: &str,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        match workflow_trigger_type_controls_menu(
            self.config.cwd.as_path(),
            workflow_path,
            trigger_id,
        ) {
            Ok(menu) => workflow_menu_popup_params(menu, initial_selected_idx),
            Err(err) => workflow_error_popup_params(
                "Trigger Type",
                "Failed to load workflow trigger type picker.",
                err,
                workflow_back_item(WorkflowControlsDestination::Root),
                initial_selected_idx,
            ),
        }
    }

    fn workflow_job_popup_params(
        &self,
        workflow_path: &Path,
        job_name: &str,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        match workflow_job_controls_menu(self.config.cwd.as_path(), workflow_path, job_name) {
            Ok(menu) => workflow_menu_popup_params(menu, initial_selected_idx),
            Err(err) => workflow_error_popup_params(
                "Workflow Job",
                "Failed to load workflow job.",
                err,
                workflow_back_item(WorkflowControlsDestination::Root),
                initial_selected_idx,
            ),
        }
    }
}

fn workflow_back_item(destination: WorkflowControlsDestination) -> SelectionItem {
    SelectionItem {
        name: "Back".to_string(),
        description: Some("Return to the previous workflow menu.".to_string()),
        selected_description: Some("Return to the previous workflow menu.".to_string()),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenWorkflowControlView {
                destination: destination.clone(),
            });
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn workflow_error_popup_params(
    title: &str,
    subtitle: &str,
    err: String,
    back_item: SelectionItem,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
        title: Some(title.to_string()),
        subtitle: Some(subtitle.to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items: vec![
            back_item,
            SelectionItem {
                name: "Workflow Error".to_string(),
                description: Some(err),
                selected_description: Some(
                    "Fix the workflow YAML, then reopen this menu.".to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            },
        ],
        is_searchable: true,
        search_placeholder: Some("Type to search workflow actions".to_string()),
        initial_selected_idx,
        ..Default::default()
    }
}

fn workflow_menu_popup_params(
    menu: WorkflowControlsMenu,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
        title: Some(menu.title),
        subtitle: Some(menu.subtitle),
        footer_hint: Some(standard_popup_hint_line()),
        items: menu
            .items
            .into_iter()
            .map(workflow_controls_selection_item)
            .collect(),
        is_searchable: true,
        search_placeholder: Some(menu.search_placeholder),
        initial_selected_idx,
        ..Default::default()
    }
}

fn workflow_controls_selection_item(item: WorkflowControlsItem) -> SelectionItem {
    SelectionItem {
        name: item.name,
        description: item.description,
        selected_description: item.selected_description,
        is_disabled: item.is_disabled,
        actions: workflow_controls_actions(item.action),
        dismiss_on_select: false,
        search_value: item.search_value,
        ..Default::default()
    }
}

fn workflow_controls_actions(action: Option<WorkflowControlsAction>) -> Vec<SelectionAction> {
    match action {
        Some(WorkflowControlsAction::ShowBackgroundTasks) => {
            vec![Box::new(|tx| {
                tx.send(AppEvent::ShowWorkflowBackgroundTasks)
            })]
        }
        Some(WorkflowControlsAction::CreateDefaultWorkflowTemplate) => {
            vec![Box::new(|tx| {
                tx.send(AppEvent::CreateDefaultWorkflowTemplate)
            })]
        }
        Some(WorkflowControlsAction::EditWorkflowFile {
            workflow_path,
            reopen,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::EditWorkflowFile {
                workflow_path: workflow_path.clone(),
                reopen: reopen.clone(),
            });
        })],
        Some(WorkflowControlsAction::OpenDestination(destination)) => vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenWorkflowControlView {
                destination: destination.clone(),
            });
        })],
        Some(WorkflowControlsAction::StartManualWorkflowTrigger {
            workflow_name,
            trigger_id,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::StartManualWorkflowTrigger {
                workflow_name: workflow_name.clone(),
                trigger_id: trigger_id.clone(),
            });
        })],
        Some(WorkflowControlsAction::StartManualWorkflowJob {
            workflow_name,
            job_name,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::StartManualWorkflowJob {
                workflow_name: workflow_name.clone(),
                job_name: job_name.clone(),
            });
        })],
        Some(WorkflowControlsAction::ToggleWorkflowTriggerEnabled {
            workflow_path,
            trigger_id,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::ToggleWorkflowTriggerEnabled {
                workflow_path: workflow_path.clone(),
                trigger_id: trigger_id.clone(),
            });
        })],
        Some(WorkflowControlsAction::ToggleWorkflowJobEnabled {
            workflow_path,
            job_name,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::ToggleWorkflowJobEnabled {
                workflow_path: workflow_path.clone(),
                job_name: job_name.clone(),
            });
        })],
        Some(WorkflowControlsAction::SetWorkflowTriggerType {
            workflow_path,
            trigger_id,
            trigger_type,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::SetWorkflowTriggerType {
                workflow_path: workflow_path.clone(),
                trigger_id: trigger_id.clone(),
                trigger_type,
            });
        })],
        Some(WorkflowControlsAction::CycleWorkflowJobContext {
            workflow_path,
            job_name,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::CycleWorkflowJobContext {
                workflow_path: workflow_path.clone(),
                job_name: job_name.clone(),
            });
        })],
        Some(WorkflowControlsAction::CycleWorkflowJobResponse {
            workflow_path,
            job_name,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::CycleWorkflowJobResponse {
                workflow_path: workflow_path.clone(),
                job_name: job_name.clone(),
            });
        })],
        Some(WorkflowControlsAction::EditWorkflowJobField {
            workflow_path,
            job_name,
            field,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::EditWorkflowJobField {
                workflow_path: workflow_path.clone(),
                job_name: job_name.clone(),
                field,
            });
        })],
        Some(WorkflowControlsAction::EditWorkflowTriggerField {
            workflow_path,
            trigger_id,
            field,
        }) => vec![Box::new(move |tx| {
            tx.send(AppEvent::EditWorkflowTriggerField {
                workflow_path: workflow_path.clone(),
                trigger_id: trigger_id.clone(),
                field,
            });
        })],
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatwidget::tests::render_bottom_popup;
    use crate::test_support::PathBufExt;
    use tempfile::tempdir;

    fn write_test_manual_workflow(workspace_cwd: &Path) {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: manual
    id: review_backlog
    jobs: [summarize]
  - type: manual
    id: triage
    enabled: false
    jobs: [notify]
  - type: interval
    id: pulse
    every: 30m
    jobs: [summarize]

jobs:
  summarize:
    context: embed
    steps:
      - prompt: |
          summarize the backlog
  notify:
    context: ephemeral
    response: user
    needs: [summarize]
    steps:
      - prompt: |
          send workflow update
"#,
        )
        .unwrap();
    }

    fn write_test_disabled_job_workflow(workspace_cwd: &Path) {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: manual
    id: review_backlog
    jobs: [notify]

jobs:
  notify:
    enabled: false
    context: ephemeral
    response: user
    steps:
      - prompt: |
          send workflow update
"#,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn workflow_root_popup_shows_create_template_when_empty() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();

        app.open_workflow_control_view(WorkflowControlsDestination::Root);
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_root_popup_empty", popup);
    }

    #[tokio::test]
    async fn workflow_file_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::File { workflow_path });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_file_popup", popup);
    }

    #[tokio::test]
    async fn workflow_root_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());

        app.open_workflow_control_view(WorkflowControlsDestination::Root);
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_root_popup", popup);
    }

    #[tokio::test]
    async fn workflow_jobs_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::Jobs { workflow_path });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_jobs_popup", popup);
    }

    #[tokio::test]
    async fn workflow_job_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::Job {
            workflow_path,
            job_name: "notify".to_string(),
        });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_job_popup", popup);
    }

    #[tokio::test]
    async fn disabled_job_popup_still_offers_run_now() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_disabled_job_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::Job {
            workflow_path,
            job_name: "notify".to_string(),
        });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        assert!(popup.contains("Run Now             Run this job immediately"));
        assert!(!popup.contains("This job is disabled."));
    }

    #[tokio::test]
    async fn workflow_manual_triggers_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::ManualTriggers {
            workflow_path,
        });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_manual_triggers_popup", popup);
    }

    #[tokio::test]
    async fn workflow_manual_trigger_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::ManualTrigger {
            workflow_path,
            trigger_id: "review_backlog".to_string(),
        });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_manual_trigger_popup", popup);
    }

    #[tokio::test]
    async fn workflow_trigger_type_popup_snapshot() {
        let mut app = super::super::tests::make_test_app().await;
        let dir = tempdir().unwrap();
        app.config.cwd = dir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path());
        let workflow_path = app
            .config
            .cwd
            .as_path()
            .join(".codex/workflows/manual.yaml");

        app.open_workflow_control_view(WorkflowControlsDestination::TriggerType {
            workflow_path,
            trigger_id: "pulse".to_string(),
        });
        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        insta::assert_snapshot!("workflow_trigger_type_popup", popup);
    }
}
