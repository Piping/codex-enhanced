use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde_yaml::Mapping;
use serde_yaml::Value as YamlValue;

use crate::definition::WorkflowContextMode;
use crate::definition::WorkflowResponseMode;
use crate::definition::WorkflowStep;
use crate::definition::workflow_dir;
use crate::definition::workflow_file_paths as load_workflow_file_paths;
use crate::yaml::serialize_yaml_value;

pub const DEFAULT_WORKFLOW_TEMPLATE_FILENAME: &str = "workflow.yaml";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowJobEditableField {
    Needs,
    Steps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerEditableField {
    Id,
    Jobs,
    Parameter,
    BindThread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerType {
    Manual,
    BeforeTurn,
    AfterTurn,
    FileWatch,
    Idle,
    Interval,
    Cron,
}

pub fn workflow_file_paths(cwd: &Path) -> Result<Vec<PathBuf>, String> {
    load_workflow_file_paths(cwd).map_err(|err| err.to_string())
}

pub fn create_default_workflow_template(cwd: &Path) -> Result<PathBuf, String> {
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

pub fn toggle_job_enabled(workflow_path: &Path, job_name: &str) -> Result<bool, String> {
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

pub fn toggle_trigger_enabled(workflow_path: &Path, trigger_id: &str) -> Result<bool, String> {
    mutate_trigger(workflow_path, trigger_id, |trigger| {
        let enabled = trigger
            .get(string_key("enabled"))
            .and_then(|value| serde_yaml::from_value::<bool>(value.clone()).ok())
            .unwrap_or(true);
        let next_enabled = !enabled;
        trigger.insert(string_key("enabled"), YamlValue::Bool(next_enabled));
        Ok(next_enabled)
    })
}

pub fn set_trigger_type(
    workflow_path: &Path,
    trigger_id: &str,
    trigger_type: WorkflowTriggerType,
) -> Result<String, String> {
    mutate_trigger(workflow_path, trigger_id, |trigger| {
        let current_parameter = trigger_parameter_seed_from_mapping(trigger);
        clear_trigger_type_fields(trigger);
        trigger.insert(
            string_key("type"),
            YamlValue::String(trigger_type_key(trigger_type).to_string()),
        );
        if let Some((parameter_key, default_value)) = trigger_type_parameter_defaults(trigger_type)
        {
            trigger.insert(
                string_key(parameter_key),
                YamlValue::String(current_parameter.unwrap_or_else(|| default_value.to_string())),
            );
        }
        Ok(trigger_id.to_string())
    })
}

pub fn cycle_job_context(
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

pub fn cycle_job_response(
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

pub fn job_field_seed(
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

pub fn write_job_field(
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

pub fn trigger_field_seed(
    workflow_path: &Path,
    trigger_id: &str,
    field: WorkflowTriggerEditableField,
) -> Result<String, String> {
    let document = load_yaml_document(workflow_path)?;
    let trigger = workflow_trigger_mapping(&document, trigger_id)?;
    match field {
        WorkflowTriggerEditableField::Id => trigger
            .get(string_key("id"))
            .and_then(YamlValue::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| format!("workflow trigger `{trigger_id}` does not define an `id`")),
        WorkflowTriggerEditableField::Jobs => {
            let jobs = trigger
                .get(string_key("jobs"))
                .map(|value| serde_yaml::from_value::<Vec<String>>(value.clone()))
                .transpose()
                .map_err(|err| err.to_string())?
                .unwrap_or_default();
            serialize_yaml_fragment(&jobs)
        }
        WorkflowTriggerEditableField::Parameter => trigger_parameter_seed_from_mapping(trigger)
            .ok_or_else(|| format!("workflow trigger `{trigger_id}` has no editable parameter")),
        WorkflowTriggerEditableField::BindThread => Ok(trigger
            .get(string_key("bind_thread"))
            .and_then(YamlValue::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()),
    }
}

pub fn write_trigger_field(
    workflow_path: &Path,
    trigger_id: &str,
    field: WorkflowTriggerEditableField,
    text: &str,
) -> Result<String, String> {
    match field {
        WorkflowTriggerEditableField::Id => {
            let next_trigger_id = text.trim();
            if next_trigger_id.is_empty() {
                return Err("workflow trigger id cannot be empty".to_string());
            }
            mutate_trigger(workflow_path, trigger_id, |trigger| {
                trigger.insert(
                    string_key("id"),
                    YamlValue::String(next_trigger_id.to_string()),
                );
                Ok(next_trigger_id.to_string())
            })
        }
        WorkflowTriggerEditableField::Jobs => {
            let jobs = match text.trim() {
                "" => Vec::new(),
                _ => serde_yaml::from_str::<Vec<String>>(text).map_err(|err| err.to_string())?,
            };
            mutate_trigger(workflow_path, trigger_id, |trigger| {
                trigger.insert(
                    string_key("jobs"),
                    serde_yaml::to_value(jobs).map_err(|err| err.to_string())?,
                );
                Ok(trigger_id.to_string())
            })
        }
        WorkflowTriggerEditableField::Parameter => {
            let next_value = text.trim();
            if next_value.is_empty() {
                return Err("workflow trigger parameter cannot be empty".to_string());
            }
            mutate_trigger(workflow_path, trigger_id, |trigger| {
                let Some(parameter_key) = trigger_parameter_key_from_mapping(trigger) else {
                    return Err(format!(
                        "workflow trigger `{trigger_id}` has no editable parameter"
                    ));
                };
                trigger.insert(
                    string_key(parameter_key),
                    YamlValue::String(next_value.to_string()),
                );
                Ok(trigger_id.to_string())
            })
        }
        WorkflowTriggerEditableField::BindThread => {
            mutate_trigger(workflow_path, trigger_id, |trigger| {
                let next_value = text.trim();
                if next_value.is_empty() {
                    trigger.remove(string_key("bind_thread"));
                } else {
                    trigger.insert(
                        string_key("bind_thread"),
                        YamlValue::String(next_value.to_string()),
                    );
                }
                Ok(trigger_id.to_string())
            })
        }
    }
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
    let contents = serialize_yaml_value(document).map_err(|err| {
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

fn workflow_trigger_mapping<'a>(
    document: &'a YamlValue,
    trigger_id: &str,
) -> Result<&'a Mapping, String> {
    let document = document
        .as_mapping()
        .ok_or_else(|| "workflow file root must be a YAML mapping".to_string())?;
    let triggers = document
        .get(string_key("triggers"))
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| "workflow file does not define a `triggers` sequence".to_string())?;

    for (index, trigger) in triggers.iter().enumerate() {
        let Some(trigger_mapping) = trigger.as_mapping() else {
            continue;
        };
        let candidate_id = trigger_mapping
            .get(string_key("id"))
            .and_then(YamlValue::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("trigger-{}", index + 1));
        if candidate_id == trigger_id {
            return Ok(trigger_mapping);
        }
    }

    Err(format!("workflow trigger `{trigger_id}` does not exist"))
}

fn workflow_trigger_mapping_mut<'a>(
    document: &'a mut YamlValue,
    trigger_id: &str,
) -> Result<&'a mut Mapping, String> {
    let document = document
        .as_mapping_mut()
        .ok_or_else(|| "workflow file root must be a YAML mapping".to_string())?;
    let triggers = document
        .get_mut(string_key("triggers"))
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| "workflow file does not define a `triggers` sequence".to_string())?;

    for (index, trigger) in triggers.iter_mut().enumerate() {
        let Some(trigger_mapping) = trigger.as_mapping_mut() else {
            continue;
        };
        let candidate_id = trigger_mapping
            .get(string_key("id"))
            .and_then(YamlValue::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("trigger-{}", index + 1));
        if candidate_id == trigger_id {
            return Ok(trigger_mapping);
        }
    }

    Err(format!("workflow trigger `{trigger_id}` does not exist"))
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

fn mutate_trigger<T>(
    workflow_path: &Path,
    trigger_id: &str,
    mutator: impl FnOnce(&mut Mapping) -> Result<T, String>,
) -> Result<T, String> {
    let mut document = load_yaml_document(workflow_path)?;
    let result = mutator(workflow_trigger_mapping_mut(&mut document, trigger_id)?)?;
    save_yaml_document(workflow_path, &document)?;
    Ok(result)
}

fn serialize_yaml_fragment(value: &impl serde::Serialize) -> Result<String, String> {
    let value = serde_yaml::to_value(value).map_err(|err| err.to_string())?;
    let text = serialize_yaml_value(&value)?;
    Ok(text.trim_end().to_string())
}

fn clear_trigger_type_fields(trigger: &mut Mapping) {
    for key in ["type", "after", "every", "cron"] {
        trigger.remove(string_key(key));
    }
}

fn trigger_type_key(trigger_type: WorkflowTriggerType) -> &'static str {
    match trigger_type {
        WorkflowTriggerType::Manual => "manual",
        WorkflowTriggerType::BeforeTurn => "before_turn",
        WorkflowTriggerType::AfterTurn => "after_turn",
        WorkflowTriggerType::FileWatch => "file_watch",
        WorkflowTriggerType::Idle => "idle",
        WorkflowTriggerType::Interval => "interval",
        WorkflowTriggerType::Cron => "cron",
    }
}

fn trigger_type_parameter_defaults(
    trigger_type: WorkflowTriggerType,
) -> Option<(&'static str, &'static str)> {
    match trigger_type {
        WorkflowTriggerType::Idle => Some(("after", "5m")),
        WorkflowTriggerType::Interval => Some(("every", "5m")),
        WorkflowTriggerType::Cron => Some(("cron", "0 * * * *")),
        WorkflowTriggerType::Manual
        | WorkflowTriggerType::BeforeTurn
        | WorkflowTriggerType::AfterTurn
        | WorkflowTriggerType::FileWatch => None,
    }
}

fn trigger_parameter_seed_from_mapping(trigger: &Mapping) -> Option<String> {
    let parameter_key = trigger_parameter_key_from_mapping(trigger)?;
    trigger
        .get(string_key(parameter_key))
        .and_then(YamlValue::as_str)
        .map(ToString::to_string)
}

fn trigger_parameter_key_from_mapping(trigger: &Mapping) -> Option<&'static str> {
    match trigger.get(string_key("type")).and_then(YamlValue::as_str) {
        Some("idle") => Some("after"),
        Some("interval") => Some("every"),
        Some("cron") => Some("cron"),
        Some("manual" | "before_turn" | "after_turn" | "file_watch") | None => None,
        Some(_) => None,
    }
}

fn string_key(value: &str) -> YamlValue {
    YamlValue::String(value.to_string())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn write_workflow(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).unwrap();
        fs::write(
            path,
            r#"name: director

triggers:
  - type: manual
    id: review
    jobs: [notify]
  - type: interval
    id: pulse
    every: 30m
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
        assert!(enabled_text.contains("prompt: |"));
        assert!(enabled_text.contains("send an update"));
    }

    #[test]
    fn toggle_trigger_enabled_writes_explicit_bool() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex/workflows/workflow.yaml");
        write_workflow(&path);

        let enabled = toggle_trigger_enabled(&path, "review").unwrap();
        assert!(!enabled);
        let disabled = fs::read_to_string(&path).unwrap();
        assert!(disabled.contains("enabled: false"));

        let enabled_again = toggle_trigger_enabled(&path, "review").unwrap();
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
  timeout: 5m
- run: cargo test -p codex-tui
  timeout: 2m
"#,
        )
        .unwrap();

        assert_eq!(
            job_field_seed(&path, "notify", WorkflowJobEditableField::Needs).unwrap(),
            "- prepare\n- publish"
        );
        let steps = job_field_seed(&path, "notify", WorkflowJobEditableField::Steps).unwrap();
        assert!(steps.contains("prompt: |"));
        assert!(steps.contains("summarize the changes"));
        assert!(steps.contains("timeout: 5m"));
        assert!(steps.contains("cargo test -p codex-tui"));
        assert!(steps.contains("timeout: 2m"));
    }

    #[test]
    fn edit_trigger_fields_and_type_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex/workflows/workflow.yaml");
        write_workflow(&path);

        assert_eq!(
            trigger_field_seed(&path, "review", WorkflowTriggerEditableField::Id).unwrap(),
            "review"
        );
        assert_eq!(
            trigger_field_seed(&path, "pulse", WorkflowTriggerEditableField::Parameter).unwrap(),
            "30m"
        );

        let next_trigger_id = write_trigger_field(
            &path,
            "review",
            WorkflowTriggerEditableField::Id,
            "review_now",
        )
        .unwrap();
        assert_eq!(next_trigger_id, "review_now");
        write_trigger_field(
            &path,
            "review_now",
            WorkflowTriggerEditableField::Jobs,
            "- notify\n- review_now\n",
        )
        .unwrap();
        set_trigger_type(&path, "review_now", WorkflowTriggerType::Cron).unwrap();
        write_trigger_field(
            &path,
            "review_now",
            WorkflowTriggerEditableField::Parameter,
            "*/15 * * * *",
        )
        .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("id: review_now"));
        assert!(text.contains("type: cron"));
        assert!(text.contains("cron: '*/15 * * * *'") || text.contains("cron: \"*/15 * * * *\""));
        assert!(text.contains("- review_now"));

        write_trigger_field(
            &path,
            "review_now",
            WorkflowTriggerEditableField::BindThread,
            "thread-1",
        )
        .unwrap();
        assert_eq!(
            trigger_field_seed(
                &path,
                "review_now",
                WorkflowTriggerEditableField::BindThread
            )
            .unwrap(),
            "thread-1"
        );

        write_trigger_field(
            &path,
            "review_now",
            WorkflowTriggerEditableField::BindThread,
            "",
        )
        .unwrap();
        assert_eq!(
            trigger_field_seed(
                &path,
                "review_now",
                WorkflowTriggerEditableField::BindThread
            )
            .unwrap(),
            ""
        );

        set_trigger_type(&path, "review_now", WorkflowTriggerType::FileWatch).unwrap();
        let file_watch_text = fs::read_to_string(&path).unwrap();
        assert!(file_watch_text.contains("type: file_watch"));
        assert!(!file_watch_text.contains("cron:"));
        assert!(!file_watch_text.contains("bind_thread:"));
    }
}
