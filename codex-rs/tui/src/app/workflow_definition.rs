use codex_app_server_protocol::TurnStatus;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const WORKFLOW_DIR_NAME: &str = "workflows";

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkflowDefinitionError {
    Io(String),
    Invalid(String),
}

impl std::fmt::Display for WorkflowDefinitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) | Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for WorkflowDefinitionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowContextStrategy {
    Embed,
    EmbedCompact,
    ThreadAuto,
    ThreadNew,
    ThreadFork,
    ThreadForkCompact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowExecutionStrategy {
    InheritSession,
    OverrideYolo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowResponseMode {
    #[default]
    Assistant,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowAfterTurnCondition {
    TurnFinished,
    #[default]
    TurnSucceeded,
}

impl WorkflowAfterTurnCondition {
    pub(crate) fn matches_turn_status(self, status: &TurnStatus) -> bool {
        match (self, status) {
            (Self::TurnFinished, TurnStatus::Completed | TurnStatus::Failed) => true,
            (Self::TurnSucceeded, TurnStatus::Completed) => true,
            (Self::TurnFinished | Self::TurnSucceeded, _) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum WorkflowStep {
    Run {
        run: String,
        retry: Option<u32>,
        timeout: Option<String>,
    },
    Prompt {
        prompt: String,
        retry: Option<u32>,
        timeout: Option<String>,
    },
}

impl WorkflowStep {
    pub(crate) fn retry_attempts(&self) -> u32 {
        match self {
            Self::Run { retry, .. } | Self::Prompt { retry, .. } => retry.unwrap_or(1),
        }
    }

    pub(crate) fn timeout(&self, default_timeout: Duration) -> Result<Duration, String> {
        let timeout = match self {
            Self::Run { timeout, .. } | Self::Prompt { timeout, .. } => timeout,
        };
        timeout.as_ref().map_or(Ok(default_timeout), |timeout| {
            humantime::parse_duration(timeout)
                .map_err(|err| format!("invalid step timeout `{timeout}`: {err}"))
        })
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Run { .. } => "run",
            Self::Prompt { .. } => "prompt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowJobConfig {
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) needs: Vec<String>,
    pub(crate) context_strategy: WorkflowContextStrategy,
    pub(crate) execution_strategy: WorkflowExecutionStrategy,
    #[serde(default)]
    pub(crate) response: WorkflowResponseMode,
    #[serde(default)]
    pub(crate) steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct WorkflowFile {
    name: String,
    #[serde(default)]
    triggers: Vec<WorkflowTriggerConfig>,
    #[serde(default)]
    jobs: IndexMap<String, WorkflowJobConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct WorkflowTriggerConfig {
    #[serde(default)]
    id: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    jobs: Vec<String>,
    #[serde(flatten)]
    kind: WorkflowTriggerKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WorkflowTriggerKind {
    Manual,
    BeforeTurn,
    AfterTurn {
        #[serde(default)]
        condition: WorkflowAfterTurnCondition,
    },
    FileWatch,
    Idle {
        after: String,
    },
    Interval {
        every: String,
    },
    Cron {
        cron: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedWorkflowRegistry {
    pub(crate) files: Vec<LoadedWorkflowFile>,
    pub(crate) jobs: BTreeMap<String, LoadedWorkflowJob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedWorkflowFile {
    pub(crate) name: String,
    pub(crate) source_path: PathBuf,
    pub(crate) triggers: Vec<LoadedWorkflowTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedWorkflowTrigger {
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) jobs: Vec<String>,
    pub(crate) kind: WorkflowTriggerKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedWorkflowJob {
    pub(crate) name: String,
    pub(crate) workflow_name: String,
    pub(crate) workflow_path: PathBuf,
    pub(crate) definition_index: usize,
    pub(crate) config: WorkflowJobConfig,
}

pub(crate) fn load_workflow_registry(
    cwd: &Path,
) -> Result<LoadedWorkflowRegistry, WorkflowDefinitionError> {
    let workflow_dir = cwd.join(".codex").join(WORKFLOW_DIR_NAME);
    if !workflow_dir.exists() {
        return Ok(LoadedWorkflowRegistry {
            files: Vec::new(),
            jobs: BTreeMap::new(),
        });
    }

    let mut files = fs::read_dir(&workflow_dir)
        .map_err(|err| {
            WorkflowDefinitionError::Io(format!(
                "failed to read workflow directory `{}`: {err}",
                workflow_dir.display()
            ))
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .collect::<Vec<_>>();
    files.sort();

    let mut workflow_names = BTreeSet::new();
    let mut loaded_files = Vec::new();
    let mut jobs = BTreeMap::new();
    let mut next_job_index = 0usize;

    for path in files {
        let contents = fs::read_to_string(&path).map_err(|err| {
            WorkflowDefinitionError::Io(format!(
                "failed to read workflow file `{}`: {err}",
                path.display()
            ))
        })?;
        let file: WorkflowFile = serde_yaml::from_str(&contents).map_err(|err| {
            WorkflowDefinitionError::Invalid(format!(
                "failed to parse workflow file `{}`: {err}",
                path.display()
            ))
        })?;

        let workflow_name = file.name.trim();
        if workflow_name.is_empty() {
            return Err(WorkflowDefinitionError::Invalid(format!(
                "workflow file `{}` must define a non-empty `name`",
                path.display()
            )));
        }
        if !workflow_names.insert(workflow_name.to_string()) {
            return Err(WorkflowDefinitionError::Invalid(format!(
                "duplicate workflow name `{workflow_name}` detected in `{}`",
                path.display()
            )));
        }

        for (job_name, job) in &file.jobs {
            let job_name = job_name.trim();
            if job_name.is_empty() {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{workflow_name}` in `{}` contains an empty job name",
                    path.display()
                )));
            }
            if jobs.contains_key(job_name) {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "duplicate job name `{job_name}` detected while loading `{}`",
                    path.display()
                )));
            }
            if job.steps.is_empty() {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{workflow_name}` job `{job_name}` in `{}` must define at least one step",
                    path.display()
                )));
            }
            for step in &job.steps {
                if let Err(err) = step.timeout(Duration::from_secs(30)) {
                    return Err(WorkflowDefinitionError::Invalid(format!(
                        "workflow `{workflow_name}` job `{job_name}` in `{}` has invalid {} step timeout: {err}",
                        path.display(),
                        step.kind()
                    )));
                }
            }
            if matches!(
                job.context_strategy,
                WorkflowContextStrategy::Embed | WorkflowContextStrategy::EmbedCompact
            ) && job
                .steps
                .iter()
                .any(|step| matches!(step, WorkflowStep::Run { .. }))
            {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{workflow_name}` job `{job_name}` in `{}` cannot use `run` steps when `context_strategy` is `{}`",
                    path.display(),
                    job.context_strategy.as_str()
                )));
            }
            jobs.insert(
                job_name.to_string(),
                LoadedWorkflowJob {
                    name: job_name.to_string(),
                    workflow_name: workflow_name.to_string(),
                    workflow_path: path.clone(),
                    definition_index: next_job_index,
                    config: job.clone(),
                },
            );
            next_job_index = next_job_index.saturating_add(1);
        }

        let mut trigger_ids = BTreeSet::new();
        let mut triggers = Vec::new();
        for (index, trigger) in file.triggers.iter().enumerate() {
            if trigger.jobs.is_empty() {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{workflow_name}` trigger #{index} in `{}` must reference at least one job",
                    path.display()
                )));
            }
            for job_name in &trigger.jobs {
                if !file.jobs.contains_key(job_name) {
                    return Err(WorkflowDefinitionError::Invalid(format!(
                        "workflow `{workflow_name}` trigger `{}` in `{}` references missing job `{job_name}`",
                        trigger.id.as_deref().unwrap_or("<generated>"),
                        path.display()
                    )));
                }
            }
            let trigger_id = trigger
                .id
                .clone()
                .unwrap_or_else(|| format!("trigger-{}", index + 1));
            if !trigger_ids.insert(trigger_id.clone()) {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{workflow_name}` in `{}` contains duplicate trigger id `{trigger_id}`",
                    path.display()
                )));
            }
            triggers.push(LoadedWorkflowTrigger {
                id: trigger_id,
                enabled: trigger.enabled,
                jobs: trigger.jobs.clone(),
                kind: trigger.kind.clone(),
            });
        }

        loaded_files.push(LoadedWorkflowFile {
            name: workflow_name.to_string(),
            source_path: path,
            triggers,
        });
    }

    for job in jobs.values() {
        for dependency in &job.config.needs {
            if !jobs.contains_key(dependency) {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{}` job `{}` references missing dependency `{dependency}`",
                    job.workflow_name, job.name
                )));
            }
        }
    }

    let registry = LoadedWorkflowRegistry {
        files: loaded_files,
        jobs,
    };
    validate_before_turn_context_strategies(&registry)?;
    Ok(registry)
}

fn validate_before_turn_context_strategies(
    registry: &LoadedWorkflowRegistry,
) -> Result<(), WorkflowDefinitionError> {
    for workflow in &registry.files {
        for trigger in &workflow.triggers {
            if !matches!(trigger.kind, WorkflowTriggerKind::BeforeTurn) {
                continue;
            }

            let ordered_jobs = ordered_jobs_for_roots(registry, &trigger.jobs)?;
            if let Some(job) = ordered_jobs
                .iter()
                .filter_map(|job_name| registry.jobs.get(job_name))
                .find(|job| job.config.context_strategy == WorkflowContextStrategy::EmbedCompact)
            {
                return Err(WorkflowDefinitionError::Invalid(format!(
                    "workflow `{}` trigger `{}` cannot use job `{}` with `context_strategy: {}` because before_turn workflows cannot compact the main thread inline",
                    workflow.name,
                    trigger.id,
                    job.name,
                    job.config.context_strategy.as_str()
                )));
            }
        }
    }

    Ok(())
}

impl WorkflowContextStrategy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Embed => "embed",
            Self::EmbedCompact => "embed_compact",
            Self::ThreadAuto => "thread_auto",
            Self::ThreadNew => "thread_new",
            Self::ThreadFork => "thread_fork",
            Self::ThreadForkCompact => "thread_fork_compact",
        }
    }
}

impl WorkflowExecutionStrategy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InheritSession => "inherit_session",
            Self::OverrideYolo => "override_yolo",
        }
    }
}

pub(crate) fn ordered_jobs_for_roots(
    registry: &LoadedWorkflowRegistry,
    root_jobs: &[String],
) -> Result<Vec<String>, WorkflowDefinitionError> {
    let mut reachable = BTreeSet::new();
    let mut stack = root_jobs.to_vec();
    while let Some(job_name) = stack.pop() {
        let job = registry.jobs.get(&job_name).ok_or_else(|| {
            WorkflowDefinitionError::Invalid(format!(
                "workflow execution root references missing job `{job_name}`"
            ))
        })?;
        if reachable.insert(job_name.clone()) {
            stack.extend(job.config.needs.iter().cloned());
        }
    }

    let mut indegree = reachable
        .iter()
        .map(|job_name| (job_name.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut dependents = BTreeMap::<String, Vec<String>>::new();
    for job_name in &reachable {
        let Some(job) = registry.jobs.get(job_name) else {
            return Err(WorkflowDefinitionError::Invalid(format!(
                "reachable workflow job `{job_name}` is missing from registry"
            )));
        };
        for dependency in &job.config.needs {
            if !reachable.contains(dependency) {
                continue;
            }
            *indegree.entry(job_name.clone()).or_default() += 1;
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(job_name.clone());
        }
    }

    let mut ready = reachable
        .iter()
        .filter(|job_name| indegree.get(*job_name) == Some(&0))
        .cloned()
        .collect::<VecDeque<_>>();
    let mut ordered = Vec::new();
    while let Some(job_name) = pop_next_job(&mut ready, registry) {
        ordered.push(job_name.clone());
        for dependent in dependents.get(&job_name).into_iter().flatten() {
            if let Some(entry) = indegree.get_mut(dependent) {
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    ready.push_back(dependent.clone());
                }
            }
        }
    }

    if ordered.len() != reachable.len() {
        return Err(WorkflowDefinitionError::Invalid(
            "workflow job selection contains a cyclic dependency graph".to_string(),
        ));
    }

    Ok(ordered)
}

fn pop_next_job(ready: &mut VecDeque<String>, registry: &LoadedWorkflowRegistry) -> Option<String> {
    let best_index = ready
        .iter()
        .enumerate()
        .filter_map(|(index, job_name)| {
            registry
                .jobs
                .get(job_name)
                .map(|job| (index, job.definition_index))
        })
        .min_by_key(|(_, definition_index)| *definition_index)
        .map(|(index, _)| index)?;
    ready.remove(best_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn workflow_step_timeout_uses_default_when_unset() {
        let step = WorkflowStep::Prompt {
            prompt: "summarize".to_string(),
            retry: None,
            timeout: None,
        };

        assert_eq!(
            step.timeout(Duration::from_secs(30)).unwrap(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn load_workflow_registry_rejects_invalid_step_timeout() {
        let dir = tempdir().unwrap();
        let workflows_dir = dir.path().join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        let workflow_path = workflows_dir.join("workflow.yaml");
        fs::write(
            &workflow_path,
            r#"name: director

jobs:
  notify:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the changes
        timeout: not-a-duration
"#,
        )
        .unwrap();

        let error = load_workflow_registry(dir.path()).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "workflow `director` job `notify` in `{}` has invalid prompt step timeout",
                workflow_path.display()
            )),
            "unexpected error message: {message}"
        );
        assert!(
            message.contains("invalid step timeout `not-a-duration`"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn load_workflow_registry_defaults_after_turn_condition_to_turn_succeeded() {
        let dir = tempdir().unwrap();
        let workflows_dir = dir.path().join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        fs::write(
            workflows_dir.join("workflow.yaml"),
            r#"name: director

triggers:
  - type: after_turn
    id: followup
    jobs: [notify]

jobs:
  notify:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the changes
"#,
        )
        .unwrap();

        let registry = load_workflow_registry(dir.path()).unwrap();
        assert_eq!(
            registry.files[0].triggers[0].kind,
            WorkflowTriggerKind::AfterTurn {
                condition: WorkflowAfterTurnCondition::TurnSucceeded,
            }
        );
    }

    #[test]
    fn load_workflow_registry_rejects_legacy_context_field() {
        let dir = tempdir().unwrap();
        let workflows_dir = dir.path().join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        let workflow_path = workflows_dir.join("workflow.yaml");
        fs::write(
            &workflow_path,
            r#"name: director

jobs:
  notify:
    context: ephemeral
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the changes
"#,
        )
        .unwrap();

        let error = load_workflow_registry(dir.path()).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "failed to parse workflow file `{}`",
                workflow_path.display()
            )),
            "unexpected error message: {message}"
        );
        assert!(
            message.contains("unknown field `context`"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn load_workflow_registry_rejects_missing_execution_strategy() {
        let dir = tempdir().unwrap();
        let workflows_dir = dir.path().join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        let workflow_path = workflows_dir.join("workflow.yaml");
        fs::write(
            &workflow_path,
            r#"name: director

jobs:
  notify:
    context_strategy: thread_auto
    steps:
      - prompt: summarize the changes
"#,
        )
        .unwrap();

        let error = load_workflow_registry(dir.path()).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "failed to parse workflow file `{}`",
                workflow_path.display()
            )),
            "unexpected error message: {message}"
        );
        assert!(
            message.contains("missing field `execution_strategy`"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn load_workflow_registry_rejects_before_turn_embed_compact() {
        let dir = tempdir().unwrap();
        let workflows_dir = dir.path().join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        let workflow_path = workflows_dir.join("workflow.yaml");
        fs::write(
            &workflow_path,
            r#"name: director

triggers:
  - type: before_turn
    id: followup
    jobs: [notify]

jobs:
  notify:
    context_strategy: embed_compact
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the changes
"#,
        )
        .unwrap();

        let error = load_workflow_registry(dir.path()).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(
                "workflow `director` trigger `followup` cannot use job `notify` with `context_strategy: embed_compact`"
            ),
            "unexpected error message: {message}"
        );
        assert!(
            message.contains("before_turn workflows cannot compact the main thread inline"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn load_workflow_registry_parses_after_turn_condition() {
        let dir = tempdir().unwrap();
        let workflows_dir = dir.path().join(".codex/workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        fs::write(
            workflows_dir.join("workflow.yaml"),
            r#"name: director

triggers:
  - type: after_turn
    id: followup
    condition: turn_succeeded
    jobs: [notify]

jobs:
  notify:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the changes
"#,
        )
        .unwrap();

        let registry = load_workflow_registry(dir.path()).unwrap();
        assert_eq!(
            registry.files[0].triggers[0].kind,
            WorkflowTriggerKind::AfterTurn {
                condition: WorkflowAfterTurnCondition::TurnSucceeded,
            }
        );
    }
}
