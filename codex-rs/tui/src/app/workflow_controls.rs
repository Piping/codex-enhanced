use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use crate::app_event::AppEvent;
use crate::app_event::WorkflowControlsDestination;
use crate::app_event::WorkflowJobEditableField;
use crate::app_event::WorkflowTriggerEditableField;
use crate::app_event::WorkflowTriggerType;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::tui;

use super::App;
use super::editor_helpers::ExternalEditorErrorTarget;
use super::workflow_definition::LoadedWorkflowFile;
use super::workflow_definition::LoadedWorkflowJob;
use super::workflow_definition::LoadedWorkflowRegistry;
use super::workflow_definition::WorkflowContextMode;
use super::workflow_definition::WorkflowResponseMode;
use super::workflow_definition::WorkflowTriggerKind;
use super::workflow_definition::load_workflow_registry;
use super::workflow_editor;

const WORKFLOW_CONTROLS_VIEW_ID: &str = "workflow-controls";

#[derive(Clone)]
struct WorkflowMenuState {
    files: Vec<WorkflowFileSummary>,
    registry_error: Option<String>,
}

#[derive(Clone)]
struct WorkflowFileSummary {
    workflow_path: PathBuf,
    workflow_name: Option<String>,
    display_name: String,
    filename: String,
    jobs: Vec<String>,
    triggers: Vec<WorkflowTriggerSummary>,
    trigger_count: usize,
    job_count: usize,
}

#[derive(Clone)]
struct WorkflowTriggerSummary {
    id: String,
    enabled: bool,
    kind: WorkflowTriggerKind,
}

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
        match workflow_editor::create_default_workflow_template(self.config.cwd.as_path()) {
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
        let seed = match workflow_editor::job_field_seed(workflow_path.as_path(), &job_name, field)
        {
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
        match workflow_editor::write_job_field(workflow_path.as_path(), &job_name, field, &updated)
        {
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
        let seed = match workflow_editor::trigger_field_seed(
            workflow_path.as_path(),
            &trigger_id,
            field,
        ) {
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
        match workflow_editor::write_trigger_field(
            workflow_path.as_path(),
            &trigger_id,
            field,
            &updated,
        ) {
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
        match workflow_editor::toggle_job_enabled(workflow_path.as_path(), &job_name) {
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
        match workflow_editor::toggle_trigger_enabled(workflow_path.as_path(), &trigger_id) {
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
        match workflow_editor::set_trigger_type(workflow_path.as_path(), &trigger_id, trigger_type)
        {
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
        match workflow_editor::cycle_job_context(workflow_path.as_path(), &job_name) {
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
        match workflow_editor::cycle_job_response(workflow_path.as_path(), &job_name) {
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
        let state = workflow_menu_state(self.config.cwd.as_path());

        let mut items = vec![SelectionItem {
            name: "Background Tasks".to_string(),
            description: Some(workflow_status_summary(&running_labels, &queued_labels)),
            selected_description: Some(
                "Insert a background task snapshot into the transcript. /ps shows the same live workflow state."
                    .to_string(),
            ),
            actions: vec![Box::new(|tx| tx.send(AppEvent::ShowWorkflowBackgroundTasks))],
            dismiss_on_select: false,
            ..Default::default()
        }];

        match state {
            Ok(state) => {
                if let Some(err) = state.registry_error {
                    items.push(SelectionItem {
                        name: "Workflow Registry Error".to_string(),
                        description: Some(err),
                        selected_description: Some(
                            "Structured workflow actions are unavailable until the YAML parses again, but you can still open files below."
                                .to_string(),
                        ),
                        is_disabled: true,
                        ..Default::default()
                    });
                }

                if state.files.is_empty() {
                    items.push(SelectionItem {
                        name: "Create workflow.yaml".to_string(),
                        description: Some(
                            "Create a starter template under .codex/workflows and open it in your editor."
                                .to_string(),
                        ),
                        selected_description: Some(
                            "Create a default workflow template, then open it in your configured editor."
                                .to_string(),
                        ),
                        actions: vec![Box::new(|tx| tx.send(AppEvent::CreateDefaultWorkflowTemplate))],
                        dismiss_on_select: false,
                        ..Default::default()
                    });
                } else {
                    for file in state.files {
                        let workflow_prefix = file.display_name.clone();
                        let workflow_filename = file.filename.clone();

                        items.push(SelectionItem {
                            name: format!("{workflow_prefix} - edit yaml"),
                            description: Some(format!("Open {workflow_filename} in your editor.")),
                            selected_description: Some(match file.workflow_name {
                                Some(_) => {
                                    "Open the real workflow YAML file in your external editor."
                                        .to_string()
                                }
                                None => {
                                    "Open this workflow file in your editor so you can fix the YAML."
                                        .to_string()
                                }
                            }),
                            search_value: Some(format!(
                                "{} {} edit workflow yaml",
                                workflow_prefix.to_ascii_lowercase(),
                                workflow_filename.to_ascii_lowercase()
                            )),
                            actions: vec![Box::new({
                                let workflow_path = file.workflow_path.clone();
                                move |tx| {
                                    tx.send(AppEvent::EditWorkflowFile {
                                        workflow_path: workflow_path.clone(),
                                        reopen: WorkflowControlsDestination::Root,
                                    });
                                }
                            })],
                            dismiss_on_select: false,
                            ..Default::default()
                        });

                        if file.workflow_name.is_some() {
                            for job_name in &file.jobs {
                                items.push(SelectionItem {
                                    name: format!("{workflow_prefix} - job - {job_name}"),
                                    description: Some("Workflow job".to_string()),
                                    selected_description: Some(
                                        "Open this job directly. From there you can run it, toggle it, and edit its fields."
                                            .to_string(),
                                    ),
                                    search_value: Some(format!(
                                        "{} {} {} job",
                                        workflow_prefix.to_ascii_lowercase(),
                                        workflow_filename.to_ascii_lowercase(),
                                        job_name.to_ascii_lowercase()
                                    )),
                                    actions: vec![Box::new({
                                        let workflow_path = file.workflow_path.clone();
                                        let job_name = job_name.clone();
                                        move |tx| {
                                            tx.send(AppEvent::OpenWorkflowControlView {
                                                destination: WorkflowControlsDestination::Job {
                                                    workflow_path: workflow_path.clone(),
                                                    job_name: job_name.clone(),
                                                },
                                            });
                                        }
                                    })],
                                    dismiss_on_select: false,
                                    ..Default::default()
                                });
                            }

                            for trigger in &file.triggers {
                                items.push(SelectionItem {
                                    name: format!("{workflow_prefix} - trigger - {}", trigger.id),
                                    description: Some(format!(
                                        "{} · {}",
                                        workflow_trigger_kind_display(&trigger.kind),
                                        if trigger.enabled { "Enabled" } else { "Disabled" }
                                    )),
                                    selected_description: Some(
                                        "Open this trigger directly. From there you can run it, toggle it, change its type, and edit its parameters."
                                            .to_string(),
                                    ),
                                    search_value: Some(format!(
                                        "{} {} {} trigger {}",
                                        workflow_prefix.to_ascii_lowercase(),
                                        workflow_filename.to_ascii_lowercase(),
                                        trigger.id.to_ascii_lowercase(),
                                        workflow_trigger_kind_display(&trigger.kind).to_ascii_lowercase()
                                    )),
                                    actions: vec![Box::new({
                                        let workflow_path = file.workflow_path.clone();
                                        let trigger_id = trigger.id.clone();
                                        move |tx| {
                                            tx.send(AppEvent::OpenWorkflowControlView {
                                                destination:
                                                    WorkflowControlsDestination::ManualTrigger {
                                                        workflow_path: workflow_path.clone(),
                                                        trigger_id: trigger_id.clone(),
                                                    },
                                            });
                                        }
                                    })],
                                    dismiss_on_select: false,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }
            Err(err) => {
                items.push(SelectionItem {
                    name: "Workflow Registry Error".to_string(),
                    description: Some(err),
                    selected_description: Some(
                        "Fix the workflow files under .codex/workflows, then reopen /workflow."
                            .to_string(),
                    ),
                    is_disabled: true,
                    ..Default::default()
                });
            }
        }

        SelectionViewParams {
            view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
            title: Some("Workflow".to_string()),
            subtitle: Some("Manage workflow files, jobs, and triggers directly.".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: true,
            search_placeholder: Some("Type to search workflows".to_string()),
            initial_selected_idx,
            ..Default::default()
        }
    }

    fn workflow_file_popup_params(
        &self,
        workflow_path: &Path,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        match workflow_file_view_state(self.config.cwd.as_path(), workflow_path) {
            Ok(state) => {
                let mut items = vec![
                    workflow_back_item(WorkflowControlsDestination::Root),
                    workflow_edit_file_item(
                        workflow_path.to_path_buf(),
                        WorkflowControlsDestination::File {
                            workflow_path: workflow_path.to_path_buf(),
                        },
                    ),
                ];

                if let Some(err) = &state.registry_error {
                    items.push(SelectionItem {
                        name: "Registry Error".to_string(),
                        description: Some(err.clone()),
                        selected_description: Some(
                            "Fix the YAML in your editor to restore jobs and trigger actions."
                                .to_string(),
                        ),
                        is_disabled: true,
                        ..Default::default()
                    });
                } else {
                    items.push(SelectionItem {
                        name: "Jobs".to_string(),
                        description: Some(count_label(state.summary.job_count, "job")),
                        selected_description: Some(
                            "Open the jobs menu for this workflow. From there you can drill into a job, run it, toggle it, and edit fields."
                                .to_string(),
                        ),
                        actions: vec![Box::new({
                            let workflow_path = workflow_path.to_path_buf();
                            move |tx| {
                                tx.send(AppEvent::OpenWorkflowControlView {
                                    destination: WorkflowControlsDestination::Jobs {
                                        workflow_path: workflow_path.clone(),
                                    },
                                });
                            }
                        })],
                        dismiss_on_select: false,
                        ..Default::default()
                    });
                    items.push(SelectionItem {
                        name: "Triggers".to_string(),
                        description: Some(count_label(
                            state.summary.trigger_count,
                            "trigger",
                        )),
                        selected_description: Some(
                            "Open this workflow's triggers. Trigger runs stay visible in the footer and /ps."
                                .to_string(),
                        ),
                        actions: vec![Box::new({
                            let workflow_path = workflow_path.to_path_buf();
                            move |tx| {
                                tx.send(AppEvent::OpenWorkflowControlView {
                                    destination: WorkflowControlsDestination::ManualTriggers {
                                        workflow_path: workflow_path.clone(),
                                    },
                                });
                            }
                        })],
                        dismiss_on_select: false,
                        is_disabled: state.summary.trigger_count == 0,
                        ..Default::default()
                    });
                }

                SelectionViewParams {
                    view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
                    title: Some("Workflow".to_string()),
                    subtitle: Some(format!(
                        "{} · {}",
                        state.summary.display_name, state.summary.filename
                    )),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    is_searchable: true,
                    search_placeholder: Some("Type to search workflow actions".to_string()),
                    initial_selected_idx,
                    ..Default::default()
                }
            }
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
        match workflow_loaded_file_state(self.config.cwd.as_path(), workflow_path) {
            Ok((summary, registry, workflow)) => {
                let running_labels = self.background_workflow_labels();
                let queued_labels = self.queued_trigger_labels();
                let running_set = running_labels.iter().cloned().collect::<HashSet<_>>();
                let queued_set = queued_labels.iter().cloned().collect::<HashSet<_>>();
                let mut items = vec![
                    workflow_back_item(WorkflowControlsDestination::Root),
                    workflow_edit_file_item(
                        workflow_path.to_path_buf(),
                        WorkflowControlsDestination::Jobs {
                            workflow_path: workflow_path.to_path_buf(),
                        },
                    ),
                ];

                let mut jobs = registry
                    .jobs
                    .values()
                    .filter(|job| job.workflow_path == workflow.source_path)
                    .cloned()
                    .collect::<Vec<_>>();
                jobs.sort_by_key(|job| job.definition_index);

                if jobs.is_empty() {
                    items.push(SelectionItem {
                        name: "No jobs defined".to_string(),
                        description: Some(
                            "Edit workflow.yaml to add jobs to this workflow.".to_string(),
                        ),
                        is_disabled: true,
                        ..Default::default()
                    });
                } else {
                    for job in jobs {
                        let workflow_path = workflow_path.to_path_buf();
                        let job_name = job.name.clone();
                        let status = workflow_target_status(
                            &format!("{} · {}", workflow.name, job.name),
                            &running_set,
                            &queued_set,
                        );
                        items.push(SelectionItem {
                            name: job.name.clone(),
                            description: Some(format!(
                                "{} · {} · {} · {status}",
                                if job.config.enabled { "Enabled" } else { "Disabled" },
                                workflow_context_label(job.config.context),
                                workflow_response_label(job.config.response)
                            )),
                            selected_description: Some(
                                "Open this job. You can run it now, toggle enabled state, and edit its fields."
                                    .to_string(),
                            ),
                            search_value: Some(format!(
                                "{} {} {} {}",
                                summary.display_name,
                                summary.filename,
                                job.name,
                                status
                            )),
                            actions: vec![Box::new(move |tx| {
                                tx.send(AppEvent::OpenWorkflowControlView {
                                    destination: WorkflowControlsDestination::Job {
                                        workflow_path: workflow_path.clone(),
                                        job_name: job_name.clone(),
                                    },
                                });
                            })],
                            dismiss_on_select: false,
                            ..Default::default()
                        });
                    }
                }

                SelectionViewParams {
                    view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
                    title: Some("Workflow Jobs".to_string()),
                    subtitle: Some(format!("{} · {}", workflow.name, summary.filename)),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    is_searchable: true,
                    search_placeholder: Some("Type to search jobs".to_string()),
                    initial_selected_idx,
                    ..Default::default()
                }
            }
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
        match workflow_loaded_file_state(self.config.cwd.as_path(), workflow_path) {
            Ok((summary, _registry, workflow)) => {
                let running_labels = self.background_workflow_labels();
                let queued_labels = self.queued_trigger_labels();
                let running_set = running_labels.iter().cloned().collect::<HashSet<_>>();
                let queued_set = queued_labels.iter().cloned().collect::<HashSet<_>>();
                let mut items = vec![
                    workflow_back_item(WorkflowControlsDestination::Root),
                    workflow_edit_file_item(
                        workflow_path.to_path_buf(),
                        WorkflowControlsDestination::ManualTriggers {
                            workflow_path: workflow_path.to_path_buf(),
                        },
                    ),
                ];

                let triggers = workflow.triggers.iter().collect::<Vec<_>>();
                if triggers.is_empty() {
                    items.push(SelectionItem {
                        name: "No triggers defined".to_string(),
                        description: Some("Edit workflow.yaml to add triggers.".to_string()),
                        is_disabled: true,
                        ..Default::default()
                    });
                } else {
                    for trigger in triggers {
                        let workflow_path = workflow_path.to_path_buf();
                        let trigger_id = trigger.id.clone();
                        let label = format!("{} · {}", workflow.name, trigger.id);
                        let status = workflow_target_status(&label, &running_set, &queued_set);
                        items.push(SelectionItem {
                            name: trigger.id.clone(),
                            description: Some(format!(
                                "{} · {} · {status}",
                                workflow_trigger_kind_display(&trigger.kind),
                                if trigger.enabled { "Enabled" } else { "Disabled" }
                            )),
                            selected_description: Some(
                                "Open this trigger. From there you can run it, toggle it, change its type, and edit its parameters."
                                    .to_string(),
                            ),
                            search_value: Some(format!(
                                "{} {} {} {}",
                                summary.display_name,
                                summary.filename,
                                trigger.id,
                                status
                            )),
                            actions: vec![Box::new(move |tx| {
                                tx.send(AppEvent::OpenWorkflowControlView {
                                    destination: WorkflowControlsDestination::ManualTrigger {
                                        workflow_path: workflow_path.clone(),
                                        trigger_id: trigger_id.clone(),
                                    },
                                });
                            })],
                            dismiss_on_select: false,
                            ..Default::default()
                        });
                    }
                }

                SelectionViewParams {
                    view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
                    title: Some("Workflow Triggers".to_string()),
                    subtitle: Some(format!("{} · {}", workflow.name, summary.filename)),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    is_searchable: true,
                    search_placeholder: Some("Type to search triggers".to_string()),
                    initial_selected_idx,
                    ..Default::default()
                }
            }
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
        match workflow_loaded_trigger_state(self.config.cwd.as_path(), workflow_path, trigger_id) {
            Ok((summary, workflow, trigger)) => {
                let mut items = vec![
                    workflow_back_item(WorkflowControlsDestination::Root),
                    workflow_edit_file_item(
                        workflow_path.to_path_buf(),
                        WorkflowControlsDestination::ManualTrigger {
                            workflow_path: workflow_path.to_path_buf(),
                            trigger_id: trigger.id.clone(),
                        },
                    ),
                ];

                if trigger.enabled {
                    items.push(SelectionItem {
                        name: "Run Now".to_string(),
                        description: Some(
                            "Run this trigger immediately in a background workflow thread."
                                .to_string(),
                        ),
                        selected_description: Some(
                            "Start this trigger now. Running state stays visible in the footer and /ps."
                                .to_string(),
                        ),
                        actions: vec![Box::new({
                            let workflow_name = workflow.name.clone();
                            let trigger_id = trigger.id.clone();
                            move |tx| {
                                tx.send(AppEvent::StartManualWorkflowTrigger {
                                    workflow_name: workflow_name.clone(),
                                    trigger_id: trigger_id.clone(),
                                });
                            }
                        })],
                        dismiss_on_select: false,
                        ..Default::default()
                    });
                } else {
                    items.push(SelectionItem {
                        name: "Run Now".to_string(),
                        description: Some("This trigger is disabled.".to_string()),
                        selected_description: Some(
                            "Enable this trigger first, then run it from here.".to_string(),
                        ),
                        is_disabled: true,
                        ..Default::default()
                    });
                }

                items.push(SelectionItem {
                    name: if trigger.enabled {
                        "Disable Trigger".to_string()
                    } else {
                        "Enable Trigger".to_string()
                    },
                    description: Some(if trigger.enabled {
                        "Prevent this trigger from starting until it is enabled again.".to_string()
                    } else {
                        "Allow this trigger to run again.".to_string()
                    }),
                    selected_description: Some(
                        "Toggle this trigger's enabled state in workflow.yaml.".to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let trigger_id = trigger.id.clone();
                        move |tx| {
                            tx.send(AppEvent::ToggleWorkflowTriggerEnabled {
                                workflow_path: workflow_path.clone(),
                                trigger_id: trigger_id.clone(),
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: format!("Type: {}", workflow_trigger_kind_display(&trigger.kind)),
                    description: Some(
                        "Choose which trigger type this workflow entry should use.".to_string(),
                    ),
                    selected_description: Some(
                        "Open the trigger type picker, then choose the new trigger type."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let trigger_id = trigger.id.clone();
                        move |tx| {
                            tx.send(AppEvent::OpenWorkflowControlView {
                                destination: WorkflowControlsDestination::TriggerType {
                                    workflow_path: workflow_path.clone(),
                                    trigger_id: trigger_id.clone(),
                                },
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: "Edit Trigger ID".to_string(),
                    description: Some("Rename this trigger id.".to_string()),
                    selected_description: Some(
                        "Open the trigger id in your external editor and save the updated value back into workflow.yaml."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let trigger_id = trigger.id.clone();
                        move |tx| {
                            tx.send(AppEvent::EditWorkflowTriggerField {
                                workflow_path: workflow_path.clone(),
                                trigger_id: trigger_id.clone(),
                                field: WorkflowTriggerEditableField::Id,
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: "Edit Target Jobs".to_string(),
                    description: Some(count_label(
                        workflow
                            .triggers
                            .iter()
                            .find(|candidate| candidate.id == trigger.id)
                            .map_or(0, |candidate| candidate.jobs.len()),
                        "job",
                    )),
                    selected_description: Some(
                        "Open this trigger's `jobs` field in your external editor and save the YAML list back into the workflow file."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let trigger_id = trigger.id.clone();
                        move |tx| {
                            tx.send(AppEvent::EditWorkflowTriggerField {
                                workflow_path: workflow_path.clone(),
                                trigger_id: trigger_id.clone(),
                                field: WorkflowTriggerEditableField::Jobs,
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                let parameter_item = workflow_trigger_parameter_item(workflow_path, &trigger);
                items.push(parameter_item);

                SelectionViewParams {
                    view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
                    title: Some("Workflow Trigger".to_string()),
                    subtitle: Some(format!(
                        "{} · {} · {}",
                        workflow.name, trigger.id, summary.filename
                    )),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    is_searchable: true,
                    search_placeholder: Some("Type to search trigger actions".to_string()),
                    initial_selected_idx,
                    ..Default::default()
                }
            }
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
        match workflow_loaded_trigger_state(self.config.cwd.as_path(), workflow_path, trigger_id) {
            Ok((summary, workflow, trigger)) => {
                let mut items = vec![workflow_back_item(
                    WorkflowControlsDestination::ManualTrigger {
                        workflow_path: workflow_path.to_path_buf(),
                        trigger_id: trigger.id.clone(),
                    },
                )];

                for trigger_type in [
                    WorkflowTriggerType::Manual,
                    WorkflowTriggerType::BeforeTurn,
                    WorkflowTriggerType::AfterTurn,
                    WorkflowTriggerType::FileWatch,
                    WorkflowTriggerType::Idle,
                    WorkflowTriggerType::Interval,
                    WorkflowTriggerType::Cron,
                ] {
                    let is_active = workflow_trigger_matches_type(&trigger.kind, trigger_type);
                    items.push(SelectionItem {
                        name: workflow_trigger_type_label(trigger_type).to_string(),
                        description: Some(if is_active {
                            "Current type".to_string()
                        } else {
                            workflow_trigger_type_description(trigger_type).to_string()
                        }),
                        selected_description: Some(
                            "Write the selected trigger type back into workflow.yaml.".to_string(),
                        ),
                        actions: vec![Box::new({
                            let workflow_path = workflow_path.to_path_buf();
                            let trigger_id = trigger.id.clone();
                            move |tx| {
                                tx.send(AppEvent::SetWorkflowTriggerType {
                                    workflow_path: workflow_path.clone(),
                                    trigger_id: trigger_id.clone(),
                                    trigger_type,
                                });
                            }
                        })],
                        dismiss_on_select: false,
                        is_disabled: is_active,
                        ..Default::default()
                    });
                }

                SelectionViewParams {
                    view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
                    title: Some("Trigger Type".to_string()),
                    subtitle: Some(format!(
                        "{} · {} · {}",
                        workflow.name, trigger.id, summary.filename
                    )),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    is_searchable: true,
                    search_placeholder: Some("Type to search trigger types".to_string()),
                    initial_selected_idx,
                    ..Default::default()
                }
            }
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
        match workflow_loaded_job_state(self.config.cwd.as_path(), workflow_path, job_name) {
            Ok((summary, workflow, job)) => {
                let mut items = vec![
                    workflow_back_item(WorkflowControlsDestination::Root),
                    workflow_edit_file_item(
                        workflow_path.to_path_buf(),
                        WorkflowControlsDestination::Job {
                            workflow_path: workflow_path.to_path_buf(),
                            job_name: job_name.to_string(),
                        },
                    ),
                ];

                items.push(SelectionItem {
                    name: "Run Now".to_string(),
                    description: Some(
                        "Run this job immediately in a background workflow thread.".to_string(),
                    ),
                    selected_description: Some(
                        "Start this job now. Job `enabled` only affects trigger-driven runs; manual runs are always allowed."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_name = workflow.name.clone();
                        let job_name = job.name.clone();
                        move |tx| {
                            tx.send(AppEvent::StartManualWorkflowJob {
                                workflow_name: workflow_name.clone(),
                                job_name: job_name.clone(),
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: if job.config.enabled {
                        "Disable Job".to_string()
                    } else {
                        "Enable Job".to_string()
                    },
                    description: Some(if job.config.enabled {
                        "Prevent triggers from including this job until it is enabled again."
                            .to_string()
                    } else {
                        "Allow triggers to include this job again.".to_string()
                    }),
                    selected_description: Some(
                        "Toggle whether trigger-driven workflow runs may include this job."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let job_name = job.name.clone();
                        move |tx| {
                            tx.send(AppEvent::ToggleWorkflowJobEnabled {
                                workflow_path: workflow_path.clone(),
                                job_name: job_name.clone(),
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: format!("Context: {}", workflow_context_label(job.config.context)),
                    description: Some(
                        "Toggle between embed and ephemeral execution context.".to_string(),
                    ),
                    selected_description: Some(
                        "Toggle this job's `context` field in workflow.yaml.".to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let job_name = job.name.clone();
                        move |tx| {
                            tx.send(AppEvent::CycleWorkflowJobContext {
                                workflow_path: workflow_path.clone(),
                                job_name: job_name.clone(),
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: format!("Response: {}", workflow_response_label(job.config.response)),
                    description: Some(
                        "Toggle whether this job replies as assistant output or a user follow-up."
                            .to_string(),
                    ),
                    selected_description: Some(
                        "Toggle this job's `response` field in workflow.yaml.".to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let job_name = job.name.clone();
                        move |tx| {
                            tx.send(AppEvent::CycleWorkflowJobResponse {
                                workflow_path: workflow_path.clone(),
                                job_name: job_name.clone(),
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: "Edit Needs".to_string(),
                    description: Some(count_label(job.config.needs.len(), "dependency")),
                    selected_description: Some(
                        "Open the `needs` field in your external editor and save the YAML list back into the workflow file."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let job_name = job.name.clone();
                        move |tx| {
                            tx.send(AppEvent::EditWorkflowJobField {
                                workflow_path: workflow_path.clone(),
                                job_name: job_name.clone(),
                                field: WorkflowJobEditableField::Needs,
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                items.push(SelectionItem {
                    name: "Edit Steps".to_string(),
                    description: Some(count_label(job.config.steps.len(), "step")),
                    selected_description: Some(
                        "Open this job's `steps` field in your external editor and save the YAML back into the workflow file."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        let workflow_path = workflow_path.to_path_buf();
                        let job_name = job.name.clone();
                        move |tx| {
                            tx.send(AppEvent::EditWorkflowJobField {
                                workflow_path: workflow_path.clone(),
                                job_name: job_name.clone(),
                                field: WorkflowJobEditableField::Steps,
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                });

                SelectionViewParams {
                    view_id: Some(WORKFLOW_CONTROLS_VIEW_ID),
                    title: Some("Workflow Job".to_string()),
                    subtitle: Some(format!(
                        "{} · {} · {}",
                        workflow.name, job.name, summary.filename
                    )),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    is_searchable: true,
                    search_placeholder: Some("Type to search job actions".to_string()),
                    initial_selected_idx,
                    ..Default::default()
                }
            }
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

fn workflow_menu_state(cwd: &Path) -> Result<WorkflowMenuState, String> {
    let workflow_paths = workflow_editor::workflow_file_paths(cwd)?;
    let registry = load_workflow_registry(cwd);
    let registry_error = registry.as_ref().err().map(ToString::to_string);
    let files = workflow_paths
        .into_iter()
        .map(|workflow_path| match registry.as_ref() {
            Ok(registry) => {
                if let Some(workflow) = registry
                    .files
                    .iter()
                    .find(|workflow| workflow.source_path == workflow_path)
                {
                    let mut jobs = registry
                        .jobs
                        .values()
                        .filter(|job| job.workflow_path == workflow.source_path)
                        .cloned()
                        .collect::<Vec<_>>();
                    jobs.sort_by_key(|job| job.definition_index);
                    let triggers = workflow
                        .triggers
                        .iter()
                        .map(|trigger| WorkflowTriggerSummary {
                            id: trigger.id.clone(),
                            enabled: trigger.enabled,
                            kind: trigger.kind.clone(),
                        })
                        .collect::<Vec<_>>();
                    WorkflowFileSummary {
                        workflow_path: workflow_path.clone(),
                        workflow_name: Some(workflow.name.clone()),
                        display_name: workflow.name.clone(),
                        filename: filename_label(&workflow_path),
                        job_count: jobs.len(),
                        jobs: jobs.into_iter().map(|job| job.name).collect(),
                        trigger_count: triggers.len(),
                        triggers,
                    }
                } else {
                    fallback_workflow_summary(workflow_path)
                }
            }
            Err(_) => fallback_workflow_summary(workflow_path),
        })
        .collect::<Vec<_>>();

    Ok(WorkflowMenuState {
        files,
        registry_error,
    })
}

fn workflow_file_view_state(
    cwd: &Path,
    workflow_path: &Path,
) -> Result<WorkflowFileViewState, String> {
    let state = workflow_menu_state(cwd)?;
    let summary = state
        .files
        .iter()
        .find(|file| file.workflow_path == workflow_path)
        .cloned()
        .ok_or_else(|| format!("workflow file `{}` does not exist", workflow_path.display()))?;
    Ok(WorkflowFileViewState {
        summary,
        registry_error: state.registry_error,
    })
}

fn workflow_loaded_file_state(
    cwd: &Path,
    workflow_path: &Path,
) -> Result<
    (
        WorkflowFileSummary,
        LoadedWorkflowRegistry,
        LoadedWorkflowFile,
    ),
    String,
> {
    let summary = workflow_menu_state(cwd)?
        .files
        .into_iter()
        .find(|file| file.workflow_path == workflow_path)
        .ok_or_else(|| format!("workflow file `{}` does not exist", workflow_path.display()))?;
    let registry = load_workflow_registry(cwd).map_err(|err| err.to_string())?;
    let workflow = registry
        .files
        .iter()
        .find(|workflow| workflow.source_path == workflow_path)
        .cloned()
        .ok_or_else(|| format!("workflow `{}` is not available", workflow_path.display()))?;
    Ok((summary, registry, workflow))
}

fn workflow_loaded_job_state(
    cwd: &Path,
    workflow_path: &Path,
    job_name: &str,
) -> Result<(WorkflowFileSummary, LoadedWorkflowFile, LoadedWorkflowJob), String> {
    let (summary, registry, workflow) = workflow_loaded_file_state(cwd, workflow_path)?;
    let job = registry
        .jobs
        .get(job_name)
        .filter(|job| job.workflow_path == workflow.source_path)
        .cloned()
        .ok_or_else(|| format!("workflow job `{job_name}` does not exist"))?;
    Ok((summary, workflow, job))
}

fn workflow_loaded_trigger_state(
    cwd: &Path,
    workflow_path: &Path,
    trigger_id: &str,
) -> Result<
    (
        WorkflowFileSummary,
        LoadedWorkflowFile,
        WorkflowTriggerSummary,
    ),
    String,
> {
    let (summary, _registry, workflow) = workflow_loaded_file_state(cwd, workflow_path)?;
    let trigger = workflow
        .triggers
        .iter()
        .find(|trigger| trigger.id == trigger_id)
        .map(|trigger| WorkflowTriggerSummary {
            id: trigger.id.clone(),
            enabled: trigger.enabled,
            kind: trigger.kind.clone(),
        })
        .ok_or_else(|| format!("workflow trigger `{trigger_id}` does not exist"))?;
    Ok((summary, workflow, trigger))
}

fn workflow_trigger_parameter_item(
    workflow_path: &Path,
    trigger: &WorkflowTriggerSummary,
) -> SelectionItem {
    let Some((label, description)) = workflow_trigger_parameter_metadata(&trigger.kind) else {
        return SelectionItem {
            name: "No Trigger Parameter".to_string(),
            description: Some(
                "This trigger type does not require an extra schedule parameter.".to_string(),
            ),
            selected_description: Some(
                "Change the trigger type if you need a schedule parameter such as `after`, `every`, or `cron`."
                    .to_string(),
            ),
            is_disabled: true,
            ..Default::default()
        };
    };

    SelectionItem {
        name: format!("Edit {label}"),
        description: Some(description),
        selected_description: Some(
            "Open this trigger parameter in your external editor and save it back into workflow.yaml."
                .to_string(),
        ),
        actions: vec![Box::new({
            let workflow_path = workflow_path.to_path_buf();
            let trigger_id = trigger.id.clone();
            move |tx| {
                tx.send(AppEvent::EditWorkflowTriggerField {
                    workflow_path: workflow_path.clone(),
                    trigger_id: trigger_id.clone(),
                    field: WorkflowTriggerEditableField::Parameter,
                });
            }
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn fallback_workflow_summary(workflow_path: PathBuf) -> WorkflowFileSummary {
    WorkflowFileSummary {
        display_name: workflow_path
            .file_stem()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| filename_label(&workflow_path)),
        filename: filename_label(&workflow_path),
        workflow_path,
        workflow_name: None,
        jobs: Vec::new(),
        triggers: Vec::new(),
        trigger_count: 0,
        job_count: 0,
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

fn workflow_edit_file_item(
    workflow_path: PathBuf,
    reopen: WorkflowControlsDestination,
) -> SelectionItem {
    SelectionItem {
        name: "Edit workflow.yaml".to_string(),
        description: Some(format!(
            "Open {} in your editor.",
            filename_label(&workflow_path)
        )),
        selected_description: Some(
            "Open the real workflow YAML file in your external editor.".to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::EditWorkflowFile {
                workflow_path: workflow_path.clone(),
                reopen: reopen.clone(),
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

fn workflow_status_summary(running_labels: &[String], queued_labels: &[String]) -> String {
    format!(
        "Running: {} · Queued: {}",
        running_labels.len(),
        queued_labels.len()
    )
}

fn workflow_target_status(
    label: &str,
    running_set: &HashSet<String>,
    queued_set: &HashSet<String>,
) -> &'static str {
    if running_set.contains(label) {
        "Running"
    } else if queued_set.contains(label) {
        "Queued"
    } else {
        "Ready"
    }
}

fn workflow_context_label(context: WorkflowContextMode) -> &'static str {
    match context {
        WorkflowContextMode::Embed => "Embed",
        WorkflowContextMode::Ephemeral => "Ephemeral",
    }
}

fn workflow_trigger_type_label(trigger_type: WorkflowTriggerType) -> &'static str {
    match trigger_type {
        WorkflowTriggerType::Manual => "Manual",
        WorkflowTriggerType::BeforeTurn => "Before Turn",
        WorkflowTriggerType::AfterTurn => "After Turn",
        WorkflowTriggerType::FileWatch => "File Watch",
        WorkflowTriggerType::Idle => "Idle",
        WorkflowTriggerType::Interval => "Interval",
        WorkflowTriggerType::Cron => "Cron",
    }
}

fn workflow_trigger_kind_display(kind: &WorkflowTriggerKind) -> String {
    match kind {
        WorkflowTriggerKind::Manual => "Manual".to_string(),
        WorkflowTriggerKind::BeforeTurn => "Before Turn".to_string(),
        WorkflowTriggerKind::AfterTurn => "After Turn".to_string(),
        WorkflowTriggerKind::FileWatch => "File Watch".to_string(),
        WorkflowTriggerKind::Idle { after } => format!("Idle ({after})"),
        WorkflowTriggerKind::Interval { every } => format!("Interval ({every})"),
        WorkflowTriggerKind::Cron { cron } => format!("Cron ({cron})"),
    }
}

fn workflow_trigger_type_description(trigger_type: WorkflowTriggerType) -> &'static str {
    match trigger_type {
        WorkflowTriggerType::Manual => "Run only when triggered from the workflow menu.",
        WorkflowTriggerType::BeforeTurn => "Run automatically before the next user turn.",
        WorkflowTriggerType::AfterTurn => "Run automatically after the current turn finishes.",
        WorkflowTriggerType::FileWatch => {
            "Run automatically when workspace files change. Overlapping runs are skipped."
        }
        WorkflowTriggerType::Idle => "Run after the workspace has been idle for a duration.",
        WorkflowTriggerType::Interval => "Run on a fixed repeating interval.",
        WorkflowTriggerType::Cron => "Run on a cron schedule.",
    }
}

fn workflow_trigger_matches_type(
    kind: &WorkflowTriggerKind,
    trigger_type: WorkflowTriggerType,
) -> bool {
    matches!(
        (kind, trigger_type),
        (&WorkflowTriggerKind::Manual, WorkflowTriggerType::Manual)
            | (
                &WorkflowTriggerKind::BeforeTurn,
                WorkflowTriggerType::BeforeTurn
            )
            | (
                &WorkflowTriggerKind::AfterTurn,
                WorkflowTriggerType::AfterTurn
            )
            | (
                &WorkflowTriggerKind::FileWatch,
                WorkflowTriggerType::FileWatch
            )
            | (&WorkflowTriggerKind::Idle { .. }, WorkflowTriggerType::Idle)
            | (
                &WorkflowTriggerKind::Interval { .. },
                WorkflowTriggerType::Interval
            )
            | (&WorkflowTriggerKind::Cron { .. }, WorkflowTriggerType::Cron)
    )
}

fn workflow_trigger_parameter_metadata(
    kind: &WorkflowTriggerKind,
) -> Option<(&'static str, String)> {
    match kind {
        WorkflowTriggerKind::Idle { after } => {
            Some(("Idle Delay", format!("Current `after`: `{after}`.")))
        }
        WorkflowTriggerKind::Interval { every } => {
            Some(("Interval", format!("Current `every`: `{every}`.")))
        }
        WorkflowTriggerKind::Cron { cron } => {
            Some(("Cron Schedule", format!("Current `cron`: `{cron}`.")))
        }
        WorkflowTriggerKind::Manual
        | WorkflowTriggerKind::BeforeTurn
        | WorkflowTriggerKind::AfterTurn
        | WorkflowTriggerKind::FileWatch => None,
    }
}

fn workflow_response_label(response: WorkflowResponseMode) -> &'static str {
    match response {
        WorkflowResponseMode::Assistant => "Assistant",
        WorkflowResponseMode::User => "User",
    }
}

fn workflow_job_field_label(field: WorkflowJobEditableField) -> &'static str {
    match field {
        WorkflowJobEditableField::Needs => "needs",
        WorkflowJobEditableField::Steps => "steps",
    }
}

fn workflow_trigger_field_label(field: WorkflowTriggerEditableField) -> &'static str {
    match field {
        WorkflowTriggerEditableField::Id => "id",
        WorkflowTriggerEditableField::Jobs => "jobs",
        WorkflowTriggerEditableField::Parameter => "parameter",
    }
}

fn filename_label(path: &Path) -> String {
    path.file_name()
        .map(|filename| filename.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn count_label(count: usize, noun: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{suffix}")
}

struct WorkflowFileViewState {
    summary: WorkflowFileSummary,
    registry_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatwidget::tests::render_bottom_popup;
    use crate::test_support::PathBufExt;
    use pretty_assertions::assert_eq;
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

    #[test]
    fn workflow_menu_state_lists_files_even_when_registry_is_invalid() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let workflows_dir = workspace.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let path = workflows_dir.join("broken.yaml");
        std::fs::write(&path, "name: [").unwrap();

        let state = workflow_menu_state(workspace.as_path()).unwrap();
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].filename, "broken.yaml");
        assert!(state.registry_error.is_some());
    }
}
