use std::path::Path;

use crate::controls_labels::workflow_context_label;
use crate::controls_labels::workflow_response_label;
use crate::controls_labels::workflow_trigger_kind_display;
use crate::controls_labels::workflow_trigger_matches_type;
use crate::controls_labels::workflow_trigger_parameter_metadata;
use crate::controls_labels::workflow_trigger_type_description;
use crate::controls_labels::workflow_trigger_type_label;
use crate::controls_labels::workflow_trigger_types;
use crate::controls_menu::WorkflowControlsAction;
use crate::controls_menu::WorkflowControlsDestination;
use crate::controls_menu::WorkflowControlsItem;
use crate::controls_menu::WorkflowControlsMenu;
use crate::controls_menu::count_label;
use crate::controls_menu::disabled_menu_item;
use crate::controls_menu::workflow_back_item;
use crate::controls_menu::workflow_edit_file_item;
use crate::controls_state::WorkflowLoadedJobState;
use crate::controls_state::WorkflowLoadedTriggerState;
use crate::controls_state::WorkflowTriggerSummary;
use crate::controls_state::workflow_loaded_job_state;
use crate::controls_state::workflow_loaded_trigger_state;
use crate::editor::WorkflowJobEditableField;
use crate::editor::WorkflowTriggerEditableField;

pub fn workflow_job_controls_menu(
    cwd: &Path,
    workflow_path: &Path,
    job_name: &str,
) -> Result<WorkflowControlsMenu, String> {
    let WorkflowLoadedJobState {
        summary,
        workflow,
        job,
    } = workflow_loaded_job_state(cwd, workflow_path, job_name)?;
    let items = vec![
        workflow_back_item(WorkflowControlsDestination::Root),
        workflow_edit_file_item(
            workflow_path.to_path_buf(),
            WorkflowControlsDestination::Job {
                workflow_path: workflow_path.to_path_buf(),
                job_name: job_name.to_string(),
            },
        ),
        WorkflowControlsItem {
            name: "Run Now".to_string(),
            description: Some(
                "Run this job immediately in a background workflow thread.".to_string(),
            ),
            selected_description: Some(
                "Start this job now. Job `enabled` only affects trigger-driven runs; manual runs are always allowed."
                    .to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::StartManualWorkflowJob {
                workflow_name: workflow.name.clone(),
                job_name: job.name.clone(),
            }),
        },
        WorkflowControlsItem {
            name: if job.config.enabled {
                "Disable Job".to_string()
            } else {
                "Enable Job".to_string()
            },
            description: Some(if job.config.enabled {
                "Prevent triggers from including this job until it is enabled again.".to_string()
            } else {
                "Allow triggers to include this job again.".to_string()
            }),
            selected_description: Some(
                "Toggle whether trigger-driven workflow runs may include this job.".to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::ToggleWorkflowJobEnabled {
                workflow_path: workflow_path.to_path_buf(),
                job_name: job.name.clone(),
            }),
        },
        WorkflowControlsItem {
            name: format!("Context: {}", workflow_context_label(job.config.context)),
            description: Some("Toggle between embed and ephemeral execution context.".to_string()),
            selected_description: Some(
                "Toggle this job's `context` field in workflow.yaml.".to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::CycleWorkflowJobContext {
                workflow_path: workflow_path.to_path_buf(),
                job_name: job.name.clone(),
            }),
        },
        WorkflowControlsItem {
            name: format!("Response: {}", workflow_response_label(job.config.response)),
            description: Some(
                "Toggle whether this job replies as assistant output or a user follow-up."
                    .to_string(),
            ),
            selected_description: Some(
                "Toggle this job's `response` field in workflow.yaml.".to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::CycleWorkflowJobResponse {
                workflow_path: workflow_path.to_path_buf(),
                job_name: job.name.clone(),
            }),
        },
        WorkflowControlsItem {
            name: "Edit Needs".to_string(),
            description: Some(count_label(job.config.needs.len(), "dependency")),
            selected_description: Some(
                "Open the `needs` field in your external editor and save the YAML list back into the workflow file."
                    .to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::EditWorkflowJobField {
                workflow_path: workflow_path.to_path_buf(),
                job_name: job.name.clone(),
                field: WorkflowJobEditableField::Needs,
            }),
        },
        WorkflowControlsItem {
            name: "Edit Steps".to_string(),
            description: Some(count_label(job.config.steps.len(), "step")),
            selected_description: Some(
                "Open this job's `steps` field in your external editor and save the YAML back into the workflow file."
                    .to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::EditWorkflowJobField {
                workflow_path: workflow_path.to_path_buf(),
                job_name: job.name.clone(),
                field: WorkflowJobEditableField::Steps,
            }),
        },
    ];

    Ok(WorkflowControlsMenu {
        title: "Workflow Job".to_string(),
        subtitle: format!("{} · {} · {}", workflow.name, job.name, summary.filename),
        search_placeholder: "Type to search job actions".to_string(),
        items,
    })
}

pub fn workflow_manual_trigger_controls_menu(
    cwd: &Path,
    workflow_path: &Path,
    trigger_id: &str,
) -> Result<WorkflowControlsMenu, String> {
    let WorkflowLoadedTriggerState {
        summary,
        workflow,
        trigger,
    } = workflow_loaded_trigger_state(cwd, workflow_path, trigger_id)?;
    let target_job_count = workflow
        .triggers
        .iter()
        .find(|candidate| candidate.id == trigger.id)
        .map_or(0, |candidate| candidate.jobs.len());
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

    items.push(if trigger.enabled {
        WorkflowControlsItem {
            name: "Run Now".to_string(),
            description: Some(
                "Run this trigger immediately in a background workflow thread.".to_string(),
            ),
            selected_description: Some(
                "Start this trigger now. Running state stays visible in the footer and /ps."
                    .to_string(),
            ),
            search_value: None,
            is_disabled: false,
            action: Some(WorkflowControlsAction::StartManualWorkflowTrigger {
                workflow_name: workflow.name.clone(),
                trigger_id: trigger.id.clone(),
            }),
        }
    } else {
        disabled_menu_item(
            "Run Now",
            "This trigger is disabled.".to_string(),
            Some("Enable this trigger first, then run it from here.".to_string()),
        )
    });

    items.push(WorkflowControlsItem {
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
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::ToggleWorkflowTriggerEnabled {
            workflow_path: workflow_path.to_path_buf(),
            trigger_id: trigger.id.clone(),
        }),
    });

    items.push(WorkflowControlsItem {
        name: format!("Type: {}", workflow_trigger_kind_display(&trigger.kind)),
        description: Some("Choose which trigger type this workflow entry should use.".to_string()),
        selected_description: Some(
            "Open the trigger type picker, then choose the new trigger type.".to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::OpenDestination(
            WorkflowControlsDestination::TriggerType {
                workflow_path: workflow_path.to_path_buf(),
                trigger_id: trigger.id.clone(),
            },
        )),
    });

    items.push(WorkflowControlsItem {
        name: "Edit Trigger ID".to_string(),
        description: Some("Rename this trigger id.".to_string()),
        selected_description: Some(
            "Open the trigger id in your external editor and save the updated value back into workflow.yaml."
                .to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::EditWorkflowTriggerField {
            workflow_path: workflow_path.to_path_buf(),
            trigger_id: trigger.id.clone(),
            field: WorkflowTriggerEditableField::Id,
        }),
    });

    items.push(WorkflowControlsItem {
        name: "Edit Target Jobs".to_string(),
        description: Some(count_label(target_job_count, "job")),
        selected_description: Some(
            "Open this trigger's `jobs` field in your external editor and save the YAML list back into the workflow file."
                .to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::EditWorkflowTriggerField {
            workflow_path: workflow_path.to_path_buf(),
            trigger_id: trigger.id.clone(),
            field: WorkflowTriggerEditableField::Jobs,
        }),
    });

    items.push(WorkflowControlsItem {
        name: "Edit Bound Thread".to_string(),
        description: Some(match trigger.bind_thread.as_deref() {
            Some(bind_thread) => format!("Current `bind_thread`: `{bind_thread}`."),
            None => "Current `bind_thread`: any thread.".to_string(),
        }),
        selected_description: Some(
            "Open this trigger's `bind_thread` value in your external editor. Leave it empty to allow any thread event to trigger it."
                .to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::EditWorkflowTriggerField {
            workflow_path: workflow_path.to_path_buf(),
            trigger_id: trigger.id.clone(),
            field: WorkflowTriggerEditableField::BindThread,
        }),
    });

    items.push(workflow_trigger_parameter_item(workflow_path, &trigger));

    Ok(WorkflowControlsMenu {
        title: "Workflow Trigger".to_string(),
        subtitle: format!("{} · {} · {}", workflow.name, trigger.id, summary.filename),
        search_placeholder: "Type to search trigger actions".to_string(),
        items,
    })
}

pub fn workflow_trigger_type_controls_menu(
    cwd: &Path,
    workflow_path: &Path,
    trigger_id: &str,
) -> Result<WorkflowControlsMenu, String> {
    let WorkflowLoadedTriggerState {
        summary,
        workflow,
        trigger,
    } = workflow_loaded_trigger_state(cwd, workflow_path, trigger_id)?;
    let mut items = vec![workflow_back_item(
        WorkflowControlsDestination::ManualTrigger {
            workflow_path: workflow_path.to_path_buf(),
            trigger_id: trigger.id.clone(),
        },
    )];

    for trigger_type in workflow_trigger_types() {
        let is_active = workflow_trigger_matches_type(&trigger.kind, trigger_type);
        items.push(WorkflowControlsItem {
            name: workflow_trigger_type_label(trigger_type).to_string(),
            description: Some(if is_active {
                "Current type".to_string()
            } else {
                workflow_trigger_type_description(trigger_type).to_string()
            }),
            selected_description: Some(
                "Write the selected trigger type back into workflow.yaml.".to_string(),
            ),
            search_value: None,
            is_disabled: is_active,
            action: (!is_active).then_some(WorkflowControlsAction::SetWorkflowTriggerType {
                workflow_path: workflow_path.to_path_buf(),
                trigger_id: trigger.id.clone(),
                trigger_type,
            }),
        });
    }

    Ok(WorkflowControlsMenu {
        title: "Trigger Type".to_string(),
        subtitle: format!("{} · {} · {}", workflow.name, trigger.id, summary.filename),
        search_placeholder: "Type to search trigger types".to_string(),
        items,
    })
}

fn workflow_trigger_parameter_item(
    workflow_path: &Path,
    trigger: &WorkflowTriggerSummary,
) -> WorkflowControlsItem {
    let Some((label, description)) = workflow_trigger_parameter_metadata(&trigger.kind) else {
        return disabled_menu_item(
            "No Trigger Parameter",
            "This trigger type does not require an extra schedule parameter.".to_string(),
            Some(
                "Change the trigger type if you need a schedule parameter such as `after`, `every`, or `cron`."
                    .to_string(),
            ),
        );
    };

    WorkflowControlsItem {
        name: format!("Edit {label}"),
        description: Some(description),
        selected_description: Some(
            "Open this trigger parameter in your external editor and save it back into workflow.yaml."
                .to_string(),
        ),
        search_value: None,
        is_disabled: false,
        action: Some(WorkflowControlsAction::EditWorkflowTriggerField {
            workflow_path: workflow_path.to_path_buf(),
            trigger_id: trigger.id.clone(),
            field: WorkflowTriggerEditableField::Parameter,
        }),
    }
}
