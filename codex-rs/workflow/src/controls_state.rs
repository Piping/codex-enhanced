use std::path::Path;
use std::path::PathBuf;

use crate::definition::LoadedWorkflowFile;
use crate::definition::LoadedWorkflowJob;
use crate::definition::LoadedWorkflowRegistry;
use crate::definition::WorkflowTriggerKind;
use crate::definition::load_workflow_registry;
use crate::definition::workflow_file_paths;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowMenuState {
    pub files: Vec<WorkflowFileSummary>,
    pub registry_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowFileSummary {
    pub workflow_path: PathBuf,
    pub workflow_name: Option<String>,
    pub display_name: String,
    pub filename: String,
    pub jobs: Vec<String>,
    pub triggers: Vec<WorkflowTriggerSummary>,
    pub trigger_count: usize,
    pub job_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowTriggerSummary {
    pub id: String,
    pub enabled: bool,
    pub kind: WorkflowTriggerKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRegistryInspection {
    pub file_paths: Vec<PathBuf>,
    pub registry: Option<LoadedWorkflowRegistry>,
    pub registry_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowFileViewState {
    pub summary: WorkflowFileSummary,
    pub registry_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowLoadedFileState {
    pub summary: WorkflowFileSummary,
    pub registry: LoadedWorkflowRegistry,
    pub workflow: LoadedWorkflowFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowLoadedJobState {
    pub summary: WorkflowFileSummary,
    pub workflow: LoadedWorkflowFile,
    pub job: LoadedWorkflowJob,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowLoadedTriggerState {
    pub summary: WorkflowFileSummary,
    pub workflow: LoadedWorkflowFile,
    pub trigger: WorkflowTriggerSummary,
}

pub fn inspect_workflow_registry(cwd: &Path) -> Result<WorkflowRegistryInspection, String> {
    let file_paths = workflow_file_paths(cwd).map_err(|err| err.to_string())?;
    match load_workflow_registry(cwd) {
        Ok(registry) => Ok(WorkflowRegistryInspection {
            file_paths,
            registry: Some(registry),
            registry_error: None,
        }),
        Err(err) => Ok(WorkflowRegistryInspection {
            file_paths,
            registry: None,
            registry_error: Some(err.to_string()),
        }),
    }
}

pub fn workflow_menu_state(cwd: &Path) -> Result<WorkflowMenuState, String> {
    let inspection = inspect_workflow_registry(cwd)?;
    let files = inspection
        .file_paths
        .into_iter()
        .map(|workflow_path| match inspection.registry.as_ref() {
            Some(registry) => {
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
            None => fallback_workflow_summary(workflow_path),
        })
        .collect::<Vec<_>>();

    Ok(WorkflowMenuState {
        files,
        registry_error: inspection.registry_error,
    })
}

pub fn workflow_file_view_state(
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

pub fn workflow_loaded_file_state(
    cwd: &Path,
    workflow_path: &Path,
) -> Result<WorkflowLoadedFileState, String> {
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
    Ok(WorkflowLoadedFileState {
        summary,
        registry,
        workflow,
    })
}

pub fn workflow_loaded_job_state(
    cwd: &Path,
    workflow_path: &Path,
    job_name: &str,
) -> Result<WorkflowLoadedJobState, String> {
    let state = workflow_loaded_file_state(cwd, workflow_path)?;
    let job = state
        .registry
        .jobs
        .get(job_name)
        .filter(|job| job.workflow_path == state.workflow.source_path)
        .cloned()
        .ok_or_else(|| format!("workflow job `{job_name}` does not exist"))?;
    Ok(WorkflowLoadedJobState {
        summary: state.summary,
        workflow: state.workflow,
        job,
    })
}

pub fn workflow_loaded_trigger_state(
    cwd: &Path,
    workflow_path: &Path,
    trigger_id: &str,
) -> Result<WorkflowLoadedTriggerState, String> {
    let state = workflow_loaded_file_state(cwd, workflow_path)?;
    let trigger = state
        .workflow
        .triggers
        .iter()
        .find(|trigger| trigger.id == trigger_id)
        .map(|trigger| WorkflowTriggerSummary {
            id: trigger.id.clone(),
            enabled: trigger.enabled,
            kind: trigger.kind.clone(),
        })
        .ok_or_else(|| format!("workflow trigger `{trigger_id}` does not exist"))?;
    Ok(WorkflowLoadedTriggerState {
        summary: state.summary,
        workflow: state.workflow,
        trigger,
    })
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

fn filename_label(path: &Path) -> String {
    path.file_name()
        .map(|filename| filename.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use crate::controls_menu::WorkflowControlsAction;
    use crate::controls_menu::workflow_root_controls_menu;

    use super::workflow_menu_state;

    #[test]
    fn workflow_menu_state_lists_files_even_when_registry_is_invalid() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let workflows_dir = workspace.join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        let path = workflows_dir.join("broken.yaml");
        fs::write(&path, "name: [").unwrap();

        let state = workflow_menu_state(workspace.as_path()).unwrap();
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].filename, "broken.yaml");
        assert!(state.registry_error.is_some());
    }

    #[test]
    fn workflow_root_controls_menu_keeps_background_tasks_available_on_registry_error() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let workflows_dir = workspace.join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        fs::write(workflows_dir.join("broken.yaml"), "name: [").unwrap();

        let menu = workflow_root_controls_menu(workspace.as_path(), &[], &[]);
        assert_eq!(
            menu.items[0].action,
            Some(WorkflowControlsAction::ShowBackgroundTasks)
        );
        assert!(
            menu.items
                .iter()
                .any(|item| item.name == "Workflow Registry Error")
        );
    }
}
