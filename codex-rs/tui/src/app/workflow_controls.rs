use std::collections::HashSet;
use std::sync::Arc;

use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell::HistoryCell;

use super::App;
use super::workflow_definition::LoadedWorkflowRegistry;
use super::workflow_definition::WorkflowTriggerKind;
use super::workflow_definition::load_workflow_registry;
use crate::app_server_session::AppServerSession;

const WORKFLOW_CONTROLS_VIEW_ID: &str = "workflow-controls";

impl App {
    pub(crate) fn open_workflow_controls_popup(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(WORKFLOW_CONTROLS_VIEW_ID);
        let params = self.workflow_controls_popup_params(initial_selected_idx);
        if !self
            .chat_widget
            .replace_selection_view_if_active(WORKFLOW_CONTROLS_VIEW_ID, params)
        {
            self.chat_widget
                .show_selection_view(self.workflow_controls_popup_params(initial_selected_idx));
        }
    }

    pub(crate) fn refresh_workflow_controls_if_active(&mut self) {
        let Some(initial_selected_idx) = self
            .chat_widget
            .selected_index_for_active_view(WORKFLOW_CONTROLS_VIEW_ID)
        else {
            return;
        };
        let _ = self.chat_widget.replace_selection_view_if_active(
            WORKFLOW_CONTROLS_VIEW_ID,
            self.workflow_controls_popup_params(Some(initial_selected_idx)),
        );
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

    fn workflow_controls_popup_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let running_labels = self.background_workflow_labels();
        let queued_labels = self.queued_trigger_labels();
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

        match load_workflow_registry(self.config.cwd.as_path()) {
            Ok(registry) => {
                let mut registry_items =
                    workflow_registry_items(&registry, &running_labels, &queued_labels);
                items.append(&mut registry_items);
            }
            Err(error) => {
                items.push(SelectionItem {
                    name: "Workflow Registry Error".to_string(),
                    description: Some(error.to_string()),
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
            subtitle: Some(
                "Trigger workspace workflows and inspect current scheduler/runtime state."
                    .to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: true,
            search_placeholder: Some("Type to search workflows".to_string()),
            initial_selected_idx,
            ..Default::default()
        }
    }
}

fn workflow_registry_items(
    registry: &LoadedWorkflowRegistry,
    running_labels: &[String],
    queued_labels: &[String],
) -> Vec<SelectionItem> {
    let running_set = running_labels.iter().cloned().collect::<HashSet<_>>();
    let queued_set = queued_labels.iter().cloned().collect::<HashSet<_>>();
    let mut items = Vec::new();

    let trigger_items = registry
        .files
        .iter()
        .flat_map(|workflow| {
            workflow.triggers.iter().filter_map(|trigger| {
                if !matches!(trigger.kind, WorkflowTriggerKind::Manual) {
                    return None;
                }
                let label = format!("{} · {}", workflow.name, trigger.id);
                let status = workflow_target_status(&label, &running_set, &queued_set);
                Some(SelectionItem {
                    name: label.clone(),
                    description: Some(format!("Manual trigger · {status}")),
                    selected_description: Some(
                        "Run this workflow trigger now. Running and queued state will stay visible in the footer and /ps."
                            .to_string(),
                    ),
                    search_value: Some(format!("{} {}", workflow.name, trigger.id)),
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
                })
            })
        })
        .collect::<Vec<_>>();

    if !trigger_items.is_empty() {
        items.push(disabled_section_item(
            "Manual Triggers",
            trigger_items.len(),
        ));
        items.extend(trigger_items);
    }

    let job_items = registry
        .jobs
        .values()
        .map(|job| {
            let label = format!("{} · {}", job.workflow_name, job.name);
            let status = workflow_target_status(&label, &running_set, &queued_set);
            SelectionItem {
                name: label.clone(),
                description: Some(format!("Job run · {status}")),
                selected_description: Some(
                    "Run this workflow job directly. Running state will stay visible in the footer and /ps."
                        .to_string(),
                ),
                search_value: Some(format!("{} {}", job.workflow_name, job.name)),
                actions: vec![Box::new({
                    let workflow_name = job.workflow_name.clone();
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
            }
        })
        .collect::<Vec<_>>();

    if !job_items.is_empty() {
        items.push(disabled_section_item("Jobs", job_items.len()));
        items.extend(job_items);
    }

    if items.is_empty() {
        items.push(SelectionItem {
            name: "No runnable workflow targets".to_string(),
            description: Some(
                "Add manual triggers or jobs under .codex/workflows to make them runnable from /workflow."
                    .to_string(),
            ),
            selected_description: Some(
                "Automatic before_turn / after_turn workflows still run through the existing scheduler/runtime path."
                    .to_string(),
            ),
            is_disabled: true,
            ..Default::default()
        });
    }

    items
}

fn disabled_section_item(title: &str, count: usize) -> SelectionItem {
    SelectionItem {
        name: title.to_string(),
        description: Some(format!("{count} available")),
        is_disabled: true,
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
