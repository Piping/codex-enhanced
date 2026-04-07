use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde_yaml::Mapping;
use serde_yaml::Value as YamlValue;

use super::workflow_definition::WorkflowContextMode;
use super::workflow_definition::WorkflowResponseMode;
use super::workflow_definition::WorkflowStep;
use crate::app_event::WorkflowJobEditableField;

pub(crate) const DEFAULT_WORKFLOW_TEMPLATE_FILENAME: &str = "workflow.yaml";

const WORKFLOW_DIR_NAME: &str = "workflows";
const DEFAULT_WORKFLOW_TEMPLATE: &str = r#"name: sample-workflow

triggers:
  - type: manual
    id: run_now
    jobs: [main]

jobs:
  main:
    context: ephemeral
    response: assistant
    steps:
      - prompt: |
          Describe the work this workflow should do.
"#;

pub(crate) fn workflow_file_paths(cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let workflow_dir = workflow_dir(cwd);
    if !workflow_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = fs::read_dir(&workflow_dir)
        .map_err(|err| {
            format!(
                "failed to read workflow directory `{}`: {err}",
                workflow_dir.display()
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

pub(crate) fn create_default_workflow_template(cwd: &Path) -> Result<PathBuf, String> {
    let workflow_dir = workflow_dir(cwd);
    fs::create_dir_all(&workflow_dir).map_err(|err| {
        format!(
            "failed to create workflow directory `{}`: {err}",
            workflow_dir.display()
        )
    })?;

    let mut candidate = workflow_dir.join(DEFAULT_WORKFLOW_TEMPLATE_FILENAME);
    if candidate.exists() {
        let mut suffix = 2_u32;
        loop {
            let next = workflow_dir.join(format!("workflow-{suffix}.yaml"));
            if !next.exists() {
                candidate = next;
                break;
            }
            suffix = suffix.saturating_add(1);
        }
    }

    fs::write(&candidate, DEFAULT_WORKFLOW_TEMPLATE).map_err(|err| {
        format!(
            "failed to write workflow template `{}`: {err}",
            candidate.display()
        )
    })?;
    Ok(candidate)
}

pub(crate) fn toggle_job_enabled(workflow_path: &Path, job_name: &str) -> Result<bool, String> {
    mutate_job(workflow_path, job_name, |job| {
        let enabled = job
            .get(string_key("enabled"))
            .and_then(|value| serde_yaml::from_value::<bool>(value.clone()).ok())
            .unwrap_or(true);
        let next_enabled = !enabled;
        job.insert(string_key("enabled"), YamlValue::Bool(next_enabled));
        Ok(next_enabled)
    })
}

pub(crate) fn cycle_job_context(
    workflow_path: &Path,
    job_name: &str,
) -> Result<WorkflowContextMode, String> {
    mutate_job(workflow_path, job_name, |job| {
        let current = job
            .get(string_key("context"))
            .and_then(|value| serde_yaml::from_value::<WorkflowContextMode>(value.clone()).ok())
            .unwrap_or_default();
        let next = match current {
            WorkflowContextMode::Embed => WorkflowContextMode::Ephemeral,
            WorkflowContextMode::Ephemeral => WorkflowContextMode::Embed,
        };
        job.insert(
            string_key("context"),
            serde_yaml::to_value(next).map_err(|err| err.to_string())?,
        );
        Ok(next)
    })
}

pub(crate) fn cycle_job_response(
    workflow_path: &Path,
    job_name: &str,
) -> Result<WorkflowResponseMode, String> {
    mutate_job(workflow_path, job_name, |job| {
        let current = job
            .get(string_key("response"))
            .and_then(|value| serde_yaml::from_value::<WorkflowResponseMode>(value.clone()).ok())
            .unwrap_or_default();
        let next = match current {
            WorkflowResponseMode::Assistant => WorkflowResponseMode::User,
            WorkflowResponseMode::User => WorkflowResponseMode::Assistant,
        };
        job.insert(
            string_key("response"),
            serde_yaml::to_value(next).map_err(|err| err.to_string())?,
        );
        Ok(next)
    })
}

pub(crate) fn job_field_seed(
    workflow_path: &Path,
    job_name: &str,
    field: WorkflowJobEditableField,
) -> Result<String, String> {
    let document = load_yaml_document(workflow_path)?;
    let job = workflow_job_mapping(&document, job_name)?;
    match field {
        WorkflowJobEditableField::Needs => {
            let needs = job
                .get(string_key("needs"))
                .map(|value| serde_yaml::from_value::<Vec<String>>(value.clone()))
                .transpose()
                .map_err(|err| err.to_string())?
                .unwrap_or_default();
            serialize_yaml_fragment(&needs)
        }
        WorkflowJobEditableField::Steps => {
            let steps = job
                .get(string_key("steps"))
                .map(|value| serde_yaml::from_value::<Vec<WorkflowStep>>(value.clone()))
                .transpose()
                .map_err(|err| err.to_string())?
                .unwrap_or_default();
            serialize_yaml_fragment(&steps)
        }
    }
}

pub(crate) fn write_job_field(
    workflow_path: &Path,
    job_name: &str,
    field: WorkflowJobEditableField,
    text: &str,
) -> Result<(), String> {
    match field {
        WorkflowJobEditableField::Needs => {
            let needs = match text.trim() {
                "" => Vec::new(),
                _ => serde_yaml::from_str::<Vec<String>>(text).map_err(|err| err.to_string())?,
            };
            mutate_job(workflow_path, job_name, |job| {
                job.insert(
                    string_key("needs"),
                    serde_yaml::to_value(needs).map_err(|err| err.to_string())?,
                );
                Ok(())
            })
        }
        WorkflowJobEditableField::Steps => {
            let steps = match text.trim() {
                "" => Vec::new(),
                _ => serde_yaml::from_str::<Vec<WorkflowStep>>(text)
                    .map_err(|err| err.to_string())?,
            };
            mutate_job(workflow_path, job_name, |job| {
                job.insert(
                    string_key("steps"),
                    serde_yaml::to_value(steps).map_err(|err| err.to_string())?,
                );
                Ok(())
            })
        }
    }
}

fn workflow_dir(cwd: &Path) -> PathBuf {
    cwd.join(".codex").join(WORKFLOW_DIR_NAME)
}

fn load_yaml_document(workflow_path: &Path) -> Result<YamlValue, String> {
    let contents = fs::read_to_string(workflow_path).map_err(|err| {
        format!(
            "failed to read workflow file `{}`: {err}",
            workflow_path.display()
        )
    })?;
    serde_yaml::from_str(&contents).map_err(|err| {
        format!(
            "failed to parse workflow file `{}`: {err}",
            workflow_path.display()
        )
    })
}

fn save_yaml_document(workflow_path: &Path, document: &YamlValue) -> Result<(), String> {
    let contents = serde_yaml::to_string(document).map_err(|err| {
        format!(
            "failed to serialize workflow file `{}`: {err}",
            workflow_path.display()
        )
    })?;
    fs::write(workflow_path, contents).map_err(|err| {
        format!(
            "failed to write workflow file `{}`: {err}",
            workflow_path.display()
        )
    })
}

fn workflow_job_mapping<'a>(
    document: &'a YamlValue,
    job_name: &str,
) -> Result<&'a Mapping, String> {
    let document = document
        .as_mapping()
        .ok_or_else(|| "workflow file root must be a YAML mapping".to_string())?;
    let jobs = document
        .get(string_key("jobs"))
        .and_then(YamlValue::as_mapping)
        .ok_or_else(|| "workflow file does not define a `jobs` mapping".to_string())?;
    jobs.get(string_key(job_name))
        .and_then(YamlValue::as_mapping)
        .ok_or_else(|| format!("workflow job `{job_name}` does not exist"))
}

fn workflow_job_mapping_mut<'a>(
    document: &'a mut YamlValue,
    job_name: &str,
) -> Result<&'a mut Mapping, String> {
    let document = document
        .as_mapping_mut()
        .ok_or_else(|| "workflow file root must be a YAML mapping".to_string())?;
    let jobs = document
        .get_mut(string_key("jobs"))
        .and_then(YamlValue::as_mapping_mut)
        .ok_or_else(|| "workflow file does not define a `jobs` mapping".to_string())?;
    jobs.get_mut(string_key(job_name))
        .and_then(YamlValue::as_mapping_mut)
        .ok_or_else(|| format!("workflow job `{job_name}` does not exist"))
}

fn mutate_job<T>(
    workflow_path: &Path,
    job_name: &str,
    mutator: impl FnOnce(&mut Mapping) -> Result<T, String>,
) -> Result<T, String> {
    let mut document = load_yaml_document(workflow_path)?;
    let result = mutator(workflow_job_mapping_mut(&mut document, job_name)?)?;
    save_yaml_document(workflow_path, &document)?;
    Ok(result)
}

fn serialize_yaml_fragment(value: &impl serde::Serialize) -> Result<String, String> {
    let text = serde_yaml::to_string(value).map_err(|err| err.to_string())?;
    Ok(text.trim_start_matches("---\n").trim_end().to_string())
}

fn string_key(value: &str) -> YamlValue {
    YamlValue::String(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn write_workflow(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).unwrap();
        fs::write(
            path,
            r#"name: director

triggers:
  - type: manual
    id: review
    jobs: [notify]

jobs:
  notify:
    response: assistant
    steps:
      - prompt: |
          send an update
"#,
        )
        .unwrap();
    }

    #[test]
    fn create_default_workflow_template_writes_new_file() {
        let dir = tempdir().unwrap();
        let created = create_default_workflow_template(dir.path()).unwrap();

        assert_eq!(
            created.file_name().unwrap().to_string_lossy(),
            DEFAULT_WORKFLOW_TEMPLATE_FILENAME
        );
        let text = fs::read_to_string(created).unwrap();
        assert!(text.contains("sample-workflow"));
        assert!(text.contains("run_now"));
    }

    #[test]
    fn toggle_job_enabled_writes_explicit_bool() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex/workflows/workflow.yaml");
        write_workflow(&path);

        let enabled = toggle_job_enabled(&path, "notify").unwrap();
        assert!(!enabled);
        let disabled = fs::read_to_string(&path).unwrap();
        assert!(disabled.contains("enabled: false"));

        let enabled_again = toggle_job_enabled(&path, "notify").unwrap();
        assert!(enabled_again);
        let enabled_text = fs::read_to_string(&path).unwrap();
        assert!(enabled_text.contains("enabled: true"));
    }

    #[test]
    fn cycle_job_context_and_response_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex/workflows/workflow.yaml");
        write_workflow(&path);

        assert_eq!(
            cycle_job_context(&path, "notify").unwrap(),
            WorkflowContextMode::Embed
        );
        assert_eq!(
            cycle_job_response(&path, "notify").unwrap(),
            WorkflowResponseMode::User
        );
    }

    #[test]
    fn edit_needs_and_steps_fields_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex/workflows/workflow.yaml");
        write_workflow(&path);

        write_job_field(
            &path,
            "notify",
            WorkflowJobEditableField::Needs,
            "- prepare\n- publish\n",
        )
        .unwrap();
        write_job_field(
            &path,
            "notify",
            WorkflowJobEditableField::Steps,
            r#"- prompt: |
    summarize the changes
- run: cargo test -p codex-tui
"#,
        )
        .unwrap();

        assert_eq!(
            job_field_seed(&path, "notify", WorkflowJobEditableField::Needs).unwrap(),
            "- prepare\n- publish"
        );
        let steps = job_field_seed(&path, "notify", WorkflowJobEditableField::Steps).unwrap();
        assert!(steps.contains("summarize the changes"));
        assert!(steps.contains("cargo test -p codex-tui"));
    }
}
