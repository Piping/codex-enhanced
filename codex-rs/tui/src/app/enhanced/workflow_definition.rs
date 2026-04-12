use std::path::Path;

pub(crate) use codex_workflow::controls::WorkflowControlsAction;
pub(crate) use codex_workflow::controls::WorkflowControlsItem;
pub(crate) use codex_workflow::controls::WorkflowControlsMenu;
pub(crate) use codex_workflow::controls::workflow_context_label;
pub(crate) use codex_workflow::controls::workflow_file_controls_menu;
pub(crate) use codex_workflow::controls::workflow_job_controls_menu;
pub(crate) use codex_workflow::controls::workflow_job_field_label;
pub(crate) use codex_workflow::controls::workflow_jobs_controls_menu;
pub(crate) use codex_workflow::controls::workflow_manual_trigger_controls_menu;
pub(crate) use codex_workflow::controls::workflow_manual_triggers_controls_menu;
pub(crate) use codex_workflow::controls::workflow_response_label;
pub(crate) use codex_workflow::controls::workflow_root_controls_menu;
pub(crate) use codex_workflow::controls::workflow_trigger_field_label;
pub(crate) use codex_workflow::controls::workflow_trigger_type_controls_menu;
pub(crate) use codex_workflow::controls::workflow_trigger_type_label;

pub(crate) use codex_workflow::definition::LoadedWorkflowRegistry;
pub(crate) use codex_workflow::definition::WorkflowTriggerKindDiscriminant;
pub(crate) use codex_workflow::definition::load_workflow_registry;
pub(crate) use codex_workflow::editor::WorkflowJobEditableField;
pub(crate) use codex_workflow::editor::WorkflowTriggerEditableField;
pub(crate) use codex_workflow::editor::WorkflowTriggerType;
pub(crate) use codex_workflow::editor::create_default_workflow_template;
pub(crate) use codex_workflow::editor::cycle_job_context;
pub(crate) use codex_workflow::editor::cycle_job_response;
pub(crate) use codex_workflow::editor::job_field_seed;
pub(crate) use codex_workflow::editor::set_trigger_type;
pub(crate) use codex_workflow::editor::toggle_job_enabled;
pub(crate) use codex_workflow::editor::toggle_trigger_enabled;
pub(crate) use codex_workflow::editor::trigger_field_seed;
pub(crate) use codex_workflow::editor::write_job_field;
pub(crate) use codex_workflow::editor::write_trigger_field;

pub(crate) fn load_workflow_registry_for_ui(cwd: &Path) -> Result<LoadedWorkflowRegistry, String> {
    load_workflow_registry(cwd).map_err(|err| err.to_string())
}
