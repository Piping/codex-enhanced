use codex_loop::LoopContextMode;
use codex_loop::LoopResponseMode;
use codex_loop::LoopSchedule;
use codex_loop::LoopSecurityMode;
use codex_loop::LoopTriggerBinding;
use codex_loop::LoopTriggerKind;
use codex_loop::PersistedLoopTimer;
use codex_loop::load_loop_timers;
use codex_loop::load_loop_trigger_queues;
use codex_loop::loop_item_name;
use codex_loop::loop_timers_path;
use codex_loop::loop_trigger_queues_path;
use codex_loop::parse_loop_cwd;
use codex_loop::parse_loop_idle_after;
use codex_loop::parse_loop_schedule;
use codex_loop::parse_loop_writable_roots;
use codex_loop::prompt_prefix;
use codex_loop::sync_trigger_queues_with_timers;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::CreateLoopServiceError;
use crate::CreateLoopTriggerRequest;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopSummary {
    pub id: String,
    pub display_name: String,
    pub prompt_prefix: String,
    pub context_mode: LoopContextMode,
    pub response_mode: LoopResponseMode,
    pub security_mode: LoopSecurityMode,
    pub enabled: bool,
    pub triggers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopInfo {
    pub timer: PersistedLoopTimer,
    pub timers_path: String,
    pub trigger_queue_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteLoopResult {
    pub id: String,
    pub timers_path: String,
    pub trigger_queue_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateLoopRequest {
    pub id: String,
    pub prompt: Option<String>,
    pub action: Option<Option<String>>,
    pub context_mode: Option<LoopContextMode>,
    pub response_mode: Option<LoopResponseMode>,
    pub security_mode: Option<LoopSecurityMode>,
    pub cwd: Option<Option<String>>,
    pub writable_roots: Option<Vec<String>>,
    pub enabled: Option<bool>,
    pub trigger_bindings: Option<Vec<CreateLoopTriggerRequest>>,
}

pub fn list_loops(workspace_cwd: &Path) -> Result<Vec<LoopSummary>, CreateLoopServiceError> {
    let timers = load_loop_timers(workspace_cwd).map_err(io_error)?;
    Ok(timers
        .timers
        .into_iter()
        .map(|timer| LoopSummary {
            id: timer.id.clone(),
            display_name: loop_item_name(&timer),
            prompt_prefix: prompt_prefix(&timer.prompt),
            context_mode: timer.context_mode,
            response_mode: timer.response_mode,
            security_mode: timer.security_mode,
            enabled: timer.enabled,
            triggers: timer
                .trigger_bindings
                .iter()
                .map(LoopTriggerBinding::selection_name)
                .collect(),
        })
        .collect())
}

pub fn get_loop(id: &str, workspace_cwd: &Path) -> Result<LoopInfo, CreateLoopServiceError> {
    let timers = load_loop_timers(workspace_cwd).map_err(io_error)?;
    let Some(timer) = timers.timers.into_iter().find(|timer| timer.id == id) else {
        return Err(CreateLoopServiceError::InvalidRequest(format!(
            "loop `{id}` does not exist"
        )));
    };
    Ok(LoopInfo {
        timer,
        timers_path: loop_timers_path(workspace_cwd).display().to_string(),
        trigger_queue_path: loop_trigger_queues_path(workspace_cwd)
            .display()
            .to_string(),
    })
}

pub fn delete_loop(
    id: &str,
    workspace_cwd: &Path,
) -> Result<DeleteLoopResult, CreateLoopServiceError> {
    let mut timers_file = load_loop_timers(workspace_cwd).map_err(io_error)?;
    let original_len = timers_file.timers.len();
    timers_file.timers.retain(|timer| timer.id != id);
    if timers_file.timers.len() == original_len {
        return Err(CreateLoopServiceError::InvalidRequest(format!(
            "loop `{id}` does not exist"
        )));
    }

    persist_timers_and_queues(workspace_cwd, timers_file.timers)?;

    Ok(DeleteLoopResult {
        id: id.to_string(),
        timers_path: loop_timers_path(workspace_cwd).display().to_string(),
        trigger_queue_path: loop_trigger_queues_path(workspace_cwd)
            .display()
            .to_string(),
    })
}

pub fn update_loop(
    request: UpdateLoopRequest,
    workspace_cwd: &Path,
) -> Result<LoopInfo, CreateLoopServiceError> {
    let mut timers_file = load_loop_timers(workspace_cwd).map_err(io_error)?;
    let Some(timer) = timers_file
        .timers
        .iter_mut()
        .find(|timer| timer.id == request.id)
    else {
        return Err(CreateLoopServiceError::InvalidRequest(format!(
            "loop `{}` does not exist",
            request.id
        )));
    };

    if let Some(prompt) = request.prompt {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(CreateLoopServiceError::InvalidRequest(
                "prompt must not be empty".to_string(),
            ));
        }
        timer.prompt = prompt;
    }

    if let Some(action) = request.action {
        timer.action = action
            .as_deref()
            .map(str::trim)
            .filter(|action| !action.is_empty())
            .map(ToOwned::to_owned);
    }

    if let Some(context_mode) = request.context_mode {
        timer.context_mode = context_mode;
        timer.mode = if matches!(context_mode, LoopContextMode::Persistent) {
            codex_loop::LoopMode::Persistent
        } else {
            codex_loop::LoopMode::OneShot
        };
    }
    if let Some(response_mode) = request.response_mode {
        timer.response_mode = response_mode;
    }
    if let Some(enabled) = request.enabled {
        timer.enabled = enabled;
    }

    let mut execution = timer.execution.clone();
    if let Some(cwd) = request.cwd {
        execution.cwd = cwd
            .as_deref()
            .filter(|cwd| !cwd.trim().is_empty())
            .map(|cwd| parse_loop_cwd(cwd, workspace_cwd))
            .transpose()
            .map_err(CreateLoopServiceError::InvalidRequest)?;
    }
    if let Some(writable_roots) = request.writable_roots {
        execution.writable_roots = if writable_roots.is_empty() {
            Vec::new()
        } else {
            parse_loop_writable_roots(&writable_roots.join("\n"), workspace_cwd)
                .map_err(CreateLoopServiceError::InvalidRequest)?
        };
    }

    if let Some(security_mode) = request.security_mode {
        timer.security_mode = security_mode;
    }
    match timer.security_mode {
        LoopSecurityMode::Inherited => {
            if !execution.writable_roots.is_empty() {
                return Err(CreateLoopServiceError::InvalidRequest(
                    "writable_roots requires security_mode set to specified_directory".to_string(),
                ));
            }
        }
        LoopSecurityMode::SpecifiedDirectory => {
            if execution.writable_roots.is_empty() {
                return Err(CreateLoopServiceError::InvalidRequest(
                    "specified_directory requires at least one writable root".to_string(),
                ));
            }
        }
    }
    timer.execution = execution;

    if let Some(trigger_bindings) = request.trigger_bindings {
        if trigger_bindings.is_empty() {
            return Err(CreateLoopServiceError::InvalidRequest(
                "trigger_bindings must not be empty".to_string(),
            ));
        }
        timer.trigger_bindings = trigger_bindings
            .into_iter()
            .enumerate()
            .map(|(index, trigger)| {
                let kind = match trigger {
                    CreateLoopTriggerRequest::Timer { schedule } => LoopTriggerKind::Timer {
                        schedule: parse_loop_schedule(schedule.trim())
                            .map_err(CreateLoopServiceError::InvalidRequest)?,
                    },
                    CreateLoopTriggerRequest::Idle { after } => LoopTriggerKind::Idle {
                        after: parse_loop_idle_after(after.trim())
                            .map_err(CreateLoopServiceError::InvalidRequest)?,
                    },
                    CreateLoopTriggerRequest::BeforeTurn => LoopTriggerKind::BeforeTurn,
                    CreateLoopTriggerRequest::AfterTurn => LoopTriggerKind::AfterTurn,
                };
                Ok(LoopTriggerBinding {
                    id: format!("trigger-{}", index + 1),
                    enabled: true,
                    kind,
                })
            })
            .collect::<Result<Vec<_>, CreateLoopServiceError>>()?;
    }

    timer.schedule = timer
        .trigger_bindings
        .iter()
        .find_map(|binding| match &binding.kind {
            LoopTriggerKind::Timer { schedule } => Some(schedule.clone()),
            LoopTriggerKind::Idle { after } => Some(after.clone()),
            LoopTriggerKind::BeforeTurn | LoopTriggerKind::AfterTurn => None,
        })
        .unwrap_or(LoopSchedule::Interval {
            display: "1h".to_string(),
            seconds: 60 * 60,
        });

    let updated_timer = timer.clone();
    persist_timers_and_queues(workspace_cwd, timers_file.timers)?;

    Ok(LoopInfo {
        timer: updated_timer,
        timers_path: loop_timers_path(workspace_cwd).display().to_string(),
        trigger_queue_path: loop_trigger_queues_path(workspace_cwd)
            .display()
            .to_string(),
    })
}

fn persist_timers_and_queues(
    workspace_cwd: &Path,
    mut timers: Vec<PersistedLoopTimer>,
) -> Result<(), CreateLoopServiceError> {
    timers.sort_by(|left, right| left.id.cmp(&right.id));
    let mut queues = load_loop_trigger_queues(workspace_cwd).map_err(io_error)?;
    let timers_by_id = timers
        .iter()
        .cloned()
        .map(|timer| (timer.id.clone(), timer))
        .collect::<BTreeMap<_, _>>();
    sync_trigger_queues_with_timers(&mut queues, &timers_by_id);

    let timers_path = loop_timers_path(workspace_cwd);
    let trigger_queues_path = loop_trigger_queues_path(workspace_cwd);
    if let Some(parent) = timers_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            CreateLoopServiceError::Io(format!(
                "failed to create loop workspace metadata directory: {err}"
            ))
        })?;
    }
    let timers_json = serde_json::to_string_pretty(&codex_loop::PersistedLoopTimersFile { timers })
        .map_err(|err| {
            CreateLoopServiceError::Internal(format!("failed to serialize loop timers: {err}"))
        })?;
    fs::write(&timers_path, timers_json).map_err(|err| {
        CreateLoopServiceError::Io(format!("failed to persist loop timers: {err}"))
    })?;

    let queues_json = serde_json::to_string_pretty(&queues).map_err(|err| {
        CreateLoopServiceError::Internal(format!("failed to serialize loop trigger queues: {err}"))
    })?;
    fs::write(&trigger_queues_path, queues_json).map_err(|err| {
        CreateLoopServiceError::Io(format!("failed to persist loop trigger queues: {err}"))
    })?;
    Ok(())
}

fn io_error(err: std::io::Error) -> CreateLoopServiceError {
    CreateLoopServiceError::Io(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::DeleteLoopResult;
    use super::UpdateLoopRequest;
    use super::delete_loop;
    use super::get_loop;
    use super::list_loops;
    use super::update_loop;
    use crate::CreateLoopRequest;
    use crate::CreateLoopTriggerRequest;
    use crate::create_loop;
    use codex_loop::LoopContextMode;
    use codex_loop::LoopResponseMode;
    use codex_loop::LoopSchedule;
    use codex_loop::LoopSecurityMode;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn seed_loop(temp: &tempfile::TempDir) {
        create_loop(
            CreateLoopRequest {
                id: Some("director".to_string()),
                prompt: "review progress".to_string(),
                action: None,
                context_mode: LoopContextMode::Persistent,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::Inherited,
                cwd: None,
                writable_roots: Vec::new(),
                trigger: CreateLoopTriggerRequest::AfterTurn,
            },
            temp.path(),
        )
        .expect("seed loop");
    }

    #[test]
    fn list_and_get_loop_return_persisted_loop() {
        let temp = tempdir().expect("tempdir");
        seed_loop(&temp);

        let listed = list_loops(temp.path()).expect("list loops");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "director");
        assert_eq!(listed[0].display_name, "director");

        let info = get_loop("director", temp.path()).expect("get loop");
        assert_eq!(info.timer.id, "director");
        assert_eq!(info.timer.prompt, "review progress");
    }

    #[test]
    fn update_loop_rewrites_prompt_and_triggers() {
        let temp = tempdir().expect("tempdir");
        seed_loop(&temp);

        let updated = update_loop(
            UpdateLoopRequest {
                id: "director".to_string(),
                prompt: Some("check blockers".to_string()),
                action: Some(Some("file a follow-up".to_string())),
                context_mode: Some(LoopContextMode::Ephemeral),
                response_mode: Some(LoopResponseMode::User),
                security_mode: Some(LoopSecurityMode::Inherited),
                cwd: Some(None),
                writable_roots: Some(Vec::new()),
                enabled: Some(false),
                trigger_bindings: Some(vec![CreateLoopTriggerRequest::Timer {
                    schedule: "10m".to_string(),
                }]),
            },
            temp.path(),
        )
        .expect("update loop");

        assert_eq!(updated.timer.prompt, "check blockers");
        assert_eq!(updated.timer.action.as_deref(), Some("file a follow-up"));
        assert_eq!(updated.timer.context_mode, LoopContextMode::Ephemeral);
        assert_eq!(updated.timer.response_mode, LoopResponseMode::User);
        assert_eq!(updated.timer.enabled, false);
        assert_eq!(updated.timer.trigger_bindings.len(), 1);
        assert_eq!(
            updated.timer.trigger_bindings[0].selection_name(),
            "timer · 10m"
        );
    }

    #[test]
    fn update_loop_supports_idle_trigger() {
        let temp = tempdir().expect("tempdir");
        seed_loop(&temp);

        let updated = update_loop(
            UpdateLoopRequest {
                id: "director".to_string(),
                prompt: None,
                action: None,
                context_mode: None,
                response_mode: None,
                security_mode: None,
                cwd: None,
                writable_roots: None,
                enabled: None,
                trigger_bindings: Some(vec![CreateLoopTriggerRequest::Idle {
                    after: "20m".to_string(),
                }]),
            },
            temp.path(),
        )
        .expect("update loop");

        assert_eq!(
            "idle · 20m",
            updated.timer.trigger_bindings[0].selection_name()
        );
        assert_eq!(
            updated.timer.schedule,
            LoopSchedule::Interval {
                display: "20m".to_string(),
                seconds: 1_200,
            }
        );
    }

    #[test]
    fn delete_loop_removes_persisted_loop() {
        let temp = tempdir().expect("tempdir");
        seed_loop(&temp);

        let result = delete_loop("director", temp.path()).expect("delete loop");
        assert_eq!(
            result,
            DeleteLoopResult {
                id: "director".to_string(),
                timers_path: codex_loop::loop_timers_path(temp.path())
                    .display()
                    .to_string(),
                trigger_queue_path: codex_loop::loop_trigger_queues_path(temp.path())
                    .display()
                    .to_string(),
            }
        );
        assert!(
            list_loops(temp.path())
                .expect("list after delete")
                .is_empty()
        );
    }
}
