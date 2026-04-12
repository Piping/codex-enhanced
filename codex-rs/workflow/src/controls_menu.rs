use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use crate::controls_labels::workflow_context_label;
use crate::controls_labels::workflow_response_label;
use crate::controls_labels::workflow_trigger_kind_display;
use crate::controls_state::WorkflowLoadedFileState;
use crate::controls_state::workflow_file_view_state;
use crate::controls_state::workflow_loaded_file_state;
use crate::controls_state::workflow_menu_state;
use crate::editor::WorkflowJobEditableField;
use crate::editor::WorkflowTriggerEditableField;
use crate::editor::WorkflowTriggerType;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowControlsDestination {
    Root,
    File {
        workflow_path: PathBuf,
    },
    Jobs {
        workflow_path: PathBuf,
    },
    Job {
        workflow_path: PathBuf,
        job_name: String,
    },
    ManualTriggers {
        workflow_path: PathBuf,
    },
    ManualTrigger {
        workflow_path: PathBuf,
        trigger_id: String,
    },
    TriggerType {
        workflow_path: PathBuf,
        trigger_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowControlsAction {
    ShowBackgroundTasks,
    CreateDefaultWorkflowTemplate,
    EditWorkflowFile {
        workflow_path: PathBuf,
        reopen: WorkflowControlsDestination,
    },
    OpenDestination(WorkflowControlsDestination),
    StartManualWorkflowTrigger {
        workflow_name: String,
        trigger_id: String,
    },
    StartManualWorkflowJob {
        workflow_name: String,
        job_name: String,
    },
    ToggleWorkflowTriggerEnabled {
        workflow_path: PathBuf,
        trigger_id: String,
    },
    ToggleWorkflowJobEnabled {
        workflow_path: PathBuf,
        job_name: String,
    },
    SetWorkflowTriggerType {
        workflow_path: PathBuf,
        trigger_id: String,
        trigger_type: WorkflowTriggerType,
    },
    CycleWorkflowJobContext {
        workflow_path: PathBuf,
        job_name: String,
    },
    CycleWorkflowJobResponse {
        workflow_path: PathBuf,
        job_name: String,
    },
    EditWorkflowJobField {
        workflow_path: PathBuf,
        job_name: String,
        field: WorkflowJobEditableField,
    },
    EditWorkflowTriggerField {
        workflow_path: PathBuf,
        trigger_id: String,
        field: WorkflowTriggerEditableField,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowControlsItem {
    pub name: String,
    pub description: Option<String>,
    pub selected_description: Option<String>,
    pub search_value: Option<String>,
    pub is_disabled: bool,
    pub action: Option<WorkflowControlsAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowControlsMenu {
    pub title: String,
    pub subtitle: String,
    pub search_placeholder: String,
    pub items: Vec<WorkflowControlsItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkflowRunStatus {
    Ready,
    Running,
    Queued,
}

impl WorkflowRunStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Running => "Running",
            Self::Queued => "Queued",
        }
    }
}

pub fn workflow_root_controls_menu(
    cwd: &Path,
    running_labels: &[String],
    queued_labels: &[String],
) -> WorkflowControlsMenu {
    let mut items = vec![WorkflowControlsItem {
        name: "Background Tasks".to_string(),
        description: Some(workflow_status_summary(running_labels, queued_labels)),
        selected_description: Some(
            "Insert a background task snapshot into the transcript. /ps shows the same live workflow state."
                .to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::ShowBackgroundTasks),
    }];

    match workflow_menu_state(cwd) {
        Ok(state) => {
            if let Some(err) = state.registry_error {
                items.push(disabled_menu_item(
                    "Workflow Registry Error",
                    err,
                    Some(
                        "Structured workflow actions are unavailable until the YAML parses again, but you can still open files below."
                            .to_string(),
                    ),
                ));
            }

            if state.files.is_empty() {
                items.push(WorkflowControlsItem {
                    name: "Create workflow.yaml".to_string(),
                    description: Some(
                        "Create a starter template under .codex/workflows and open it in your editor."
                            .to_string(),
                    ),
                    selected_description: Some(
                        "Create a default workflow template, then open it in your configured editor."
                            .to_string(),
                    ),
                    search_value: None,
                    is_disabled: false,
                    action: Some(WorkflowControlsAction::CreateDefaultWorkflowTemplate),
                });
            } else {
                for file in state.files {
                    let workflow_prefix = file.display_name.clone();
                    let workflow_filename = file.filename.clone();

                    items.push(WorkflowControlsItem {
                        name: format!("{workflow_prefix} - edit yaml"),
                        description: Some(format!("Open {workflow_filename} in your editor.")),
                        selected_description: Some(match file.workflow_name {
                            Some(_) => "Open the real workflow YAML file in your external editor."
                                .to_string(),
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
                        is_disabled: false,
                        action: Some(WorkflowControlsAction::EditWorkflowFile {
                            workflow_path: file.workflow_path.clone(),
                            reopen: WorkflowControlsDestination::Root,
                        }),
                    });

                    if file.workflow_name.is_some() {
                        for job_name in &file.jobs {
                            items.push(WorkflowControlsItem {
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
                                is_disabled: false,
                                action: Some(WorkflowControlsAction::OpenDestination(
                                    WorkflowControlsDestination::Job {
                                        workflow_path: file.workflow_path.clone(),
                                        job_name: job_name.clone(),
                                    },
                                )),
                            });
                        }

                        for trigger in &file.triggers {
                            let trigger_kind = workflow_trigger_kind_display(&trigger.kind);
                            items.push(WorkflowControlsItem {
                                name: format!("{workflow_prefix} - trigger - {}", trigger.id),
                                description: Some(format!(
                                    "{trigger_kind} · {}",
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
                                    trigger_kind.to_ascii_lowercase()
                                )),
                                is_disabled: false,
                                action: Some(WorkflowControlsAction::OpenDestination(
                                    WorkflowControlsDestination::ManualTrigger {
                                        workflow_path: file.workflow_path.clone(),
                                        trigger_id: trigger.id.clone(),
                                    },
                                )),
                            });
                        }
                    }
                }
            }
        }
        Err(err) => items.push(disabled_menu_item(
            "Workflow Registry Error",
            err,
            Some(
                "Fix the workflow files under .codex/workflows, then reopen /workflow.".to_string(),
            ),
        )),
    }

    WorkflowControlsMenu {
        title: "Workflow".to_string(),
        subtitle: "Manage workflow files, jobs, and triggers directly.".to_string(),
        search_placeholder: "Type to search workflows".to_string(),
        items,
    }
}

pub fn workflow_file_controls_menu(
    cwd: &Path,
    workflow_path: &Path,
) -> Result<WorkflowControlsMenu, String> {
    let state = workflow_file_view_state(cwd, workflow_path)?;
    let mut items = vec![
        workflow_back_item(WorkflowControlsDestination::Root),
        workflow_edit_file_item(
            workflow_path.to_path_buf(),
            WorkflowControlsDestination::File {
                workflow_path: workflow_path.to_path_buf(),
            },
        ),
    ];

    if let Some(err) = state.registry_error {
        items.push(disabled_menu_item(
            "Registry Error",
            err,
            Some("Fix the YAML in your editor to restore jobs and trigger actions.".to_string()),
        ));
    } else {
        items.push(WorkflowControlsItem {
            name: "Jobs".to_string(),
            description: Some(count_label(state.summary.job_count, "job")),
            selected_description: Some(
                "Open the jobs menu for this workflow. From there you can drill into a job, run it, toggle it, and edit fields."
                    .to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::OpenDestination(
                WorkflowControlsDestination::Jobs {
                    workflow_path: workflow_path.to_path_buf(),
                },
            )),
        });
        items.push(WorkflowControlsItem {
            name: "Triggers".to_string(),
            description: Some(count_label(state.summary.trigger_count, "trigger")),
            selected_description: Some(
                "Open this workflow's triggers. Trigger runs stay visible in the footer and /ps."
                    .to_string(),
            ),
            search_value: None,
            is_disabled: state.summary.trigger_count == 0,
            action: (state.summary.trigger_count > 0).then_some(
                WorkflowControlsAction::OpenDestination(
                    WorkflowControlsDestination::ManualTriggers {
                        workflow_path: workflow_path.to_path_buf(),
                    },
                ),
            ),
        });
    }

    Ok(WorkflowControlsMenu {
        title: "Workflow".to_string(),
        subtitle: format!(
            "{} · {}",
            state.summary.display_name, state.summary.filename
        ),
        search_placeholder: "Type to search workflow actions".to_string(),
        items,
    })
}

pub fn workflow_jobs_controls_menu(
    cwd: &Path,
    workflow_path: &Path,
    running_labels: &[String],
    queued_labels: &[String],
) -> Result<WorkflowControlsMenu, String> {
    let WorkflowLoadedFileState {
        summary,
        registry,
        workflow,
    } = workflow_loaded_file_state(cwd, workflow_path)?;
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
        items.push(disabled_menu_item(
            "No jobs defined",
            "Edit workflow.yaml to add jobs to this workflow.".to_string(),
            None,
        ));
    } else {
        for job in jobs {
            let status = workflow_target_status(
                &format!("{} · {}", workflow.name, job.name),
                &running_set,
                &queued_set,
            );
            items.push(WorkflowControlsItem {
                name: job.name.clone(),
                description: Some(format!(
                    "{} · {} · {} · {}",
                    if job.config.enabled {
                        "Enabled"
                    } else {
                        "Disabled"
                    },
                    workflow_context_label(job.config.context),
                    workflow_response_label(job.config.response),
                    status.label()
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
                    status.label()
                )),
                is_disabled: false,
                action: Some(WorkflowControlsAction::OpenDestination(
                    WorkflowControlsDestination::Job {
                        workflow_path: workflow_path.to_path_buf(),
                        job_name: job.name.clone(),
                    },
                )),
            });
        }
    }

    Ok(WorkflowControlsMenu {
        title: "Workflow Jobs".to_string(),
        subtitle: format!("{} · {}", workflow.name, summary.filename),
        search_placeholder: "Type to search jobs".to_string(),
        items,
    })
}

pub fn workflow_manual_triggers_controls_menu(
    cwd: &Path,
    workflow_path: &Path,
    running_labels: &[String],
    queued_labels: &[String],
) -> Result<WorkflowControlsMenu, String> {
    let WorkflowLoadedFileState {
        summary, workflow, ..
    } = workflow_loaded_file_state(cwd, workflow_path)?;
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

    if workflow.triggers.is_empty() {
        items.push(disabled_menu_item(
            "No triggers defined",
            "Edit workflow.yaml to add triggers.".to_string(),
            None,
        ));
    } else {
        for trigger in &workflow.triggers {
            let status = workflow_target_status(
                &format!("{} · {}", workflow.name, trigger.id),
                &running_set,
                &queued_set,
            );
            items.push(WorkflowControlsItem {
                name: trigger.id.clone(),
                description: Some(format!(
                    "{} · {} · {}",
                    workflow_trigger_kind_display(&trigger.kind),
                    if trigger.enabled { "Enabled" } else { "Disabled" },
                    status.label()
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
                    status.label()
                )),
                is_disabled: false,
                action: Some(WorkflowControlsAction::OpenDestination(
                    WorkflowControlsDestination::ManualTrigger {
                        workflow_path: workflow_path.to_path_buf(),
                        trigger_id: trigger.id.clone(),
                    },
                )),
            });
        }
    }

    Ok(WorkflowControlsMenu {
        title: "Workflow Triggers".to_string(),
        subtitle: format!("{} · {}", workflow.name, summary.filename),
        search_placeholder: "Type to search triggers".to_string(),
        items,
    })
}

pub(crate) fn workflow_back_item(destination: WorkflowControlsDestination) -> WorkflowControlsItem {
    WorkflowControlsItem {
        name: "Back".to_string(),
        description: Some("Return to the previous workflow menu.".to_string()),
        selected_description: Some("Return to the previous workflow menu.".to_string()),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::OpenDestination(destination)),
    }
}

pub(crate) fn workflow_edit_file_item(
    workflow_path: PathBuf,
    reopen: WorkflowControlsDestination,
) -> WorkflowControlsItem {
    WorkflowControlsItem {
        name: "Edit workflow.yaml".to_string(),
        description: Some(format!(
            "Open {} in your editor.",
            filename_label(&workflow_path)
        )),
        selected_description: Some(
            "Open the real workflow YAML file in your external editor.".to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::EditWorkflowFile {
            workflow_path,
            reopen,
        }),
    }
}

pub(crate) fn disabled_menu_item(
    name: impl Into<String>,
    description: String,
    selected_description: Option<String>,
) -> WorkflowControlsItem {
    WorkflowControlsItem {
        name: name.into(),
        description: Some(description),
        selected_description,
        search_value: None,
        is_disabled: true,
        action: None,
    }
}

pub(crate) fn count_label(count: usize, noun: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{suffix}")
}

pub(crate) fn workflow_target_status(
    label: &str,
    running_set: &HashSet<String>,
    queued_set: &HashSet<String>,
) -> WorkflowRunStatus {
    if running_set.contains(label) {
        WorkflowRunStatus::Running
    } else if queued_set.contains(label) {
        WorkflowRunStatus::Queued
    } else {
        WorkflowRunStatus::Ready
    }
}

fn workflow_status_summary(running_labels: &[String], queued_labels: &[String]) -> String {
    format!(
        "Running: {} · Queued: {}",
        running_labels.len(),
        queued_labels.len()
    )
}

fn filename_label(path: &Path) -> String {
    path.file_name()
        .map(|filename| filename.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}
