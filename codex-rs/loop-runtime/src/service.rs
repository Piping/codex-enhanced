use chrono::Utc;
use codex_loop::LoopContextMode;
use codex_loop::LoopMode;
use codex_loop::LoopResponseMode;
use codex_loop::LoopSchedule;
use codex_loop::LoopSecurityMode;
use codex_loop::LoopTriggerBinding;
use codex_loop::LoopTriggerKind;
use codex_loop::PersistedLoopExecutionSettings;
use codex_loop::PersistedLoopTimer;
use codex_loop::PersistedLoopTimersFile;
use codex_loop::PersistedLoopTriggerQueuesFile;
use codex_loop::load_loop_timers;
use codex_loop::load_loop_trigger_queues;
use codex_loop::loop_timers_path;
use codex_loop::loop_trigger_queues_path;
use codex_loop::parse_loop_cwd;
use codex_loop::parse_loop_idle_after;
use codex_loop::parse_loop_schedule;
use codex_loop::parse_loop_writable_roots;
use codex_loop::sync_trigger_queues_with_timers;
use codex_loop::validate_loop_id;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CreateLoopRequest {
    pub id: Option<String>,
    pub prompt: String,
    pub action: Option<String>,
    pub context_mode: LoopContextMode,
    pub response_mode: LoopResponseMode,
    pub security_mode: LoopSecurityMode,
    pub cwd: Option<String>,
    #[serde(default)]
    pub writable_roots: Vec<String>,
    pub trigger: CreateLoopTriggerRequest,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreateLoopTriggerRequest {
    Timer { schedule: String },
    Idle { after: String },
    BeforeTurn,
    AfterTurn,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CreateLoopResult {
    pub id: String,
    pub context_mode: LoopContextMode,
    pub response_mode: LoopResponseMode,
    pub security_mode: LoopSecurityMode,
    pub trigger_kind: String,
    pub timers_path: String,
    pub trigger_queue_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateLoopServiceError {
    InvalidRequest(String),
    Io(String),
    Internal(String),
}

impl fmt::Display for CreateLoopServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) | Self::Io(message) | Self::Internal(message) => {
                write!(f, "{message}")
            }
        }
    }
}

impl std::error::Error for CreateLoopServiceError {}

pub fn create_loop(
    request: CreateLoopRequest,
    workspace_cwd: &Path,
) -> Result<CreateLoopResult, CreateLoopServiceError> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(CreateLoopServiceError::InvalidRequest(
            "prompt must not be empty".to_string(),
        ));
    }

    let id = request
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    match request.context_mode {
        LoopContextMode::Persistent => {
            let Some(id) = id else {
                return Err(CreateLoopServiceError::InvalidRequest(
                    "persistent loops require an id".to_string(),
                ));
            };
            validate_loop_id(id).map_err(CreateLoopServiceError::InvalidRequest)?;
        }
        LoopContextMode::Embed | LoopContextMode::Ephemeral => {
            if id.is_some() {
                return Err(CreateLoopServiceError::InvalidRequest(
                    "only persistent loops may set id".to_string(),
                ));
            }
        }
    }

    let trigger_kind = match request.trigger {
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

    let execution = PersistedLoopExecutionSettings {
        cwd: request
            .cwd
            .as_deref()
            .filter(|cwd| !cwd.trim().is_empty())
            .map(|cwd| parse_loop_cwd(cwd, workspace_cwd))
            .transpose()
            .map_err(CreateLoopServiceError::InvalidRequest)?,
        writable_roots: if request.writable_roots.is_empty() {
            Vec::new()
        } else {
            parse_loop_writable_roots(&request.writable_roots.join("\n"), workspace_cwd)
                .map_err(CreateLoopServiceError::InvalidRequest)?
        },
    };

    match request.security_mode {
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

    let timer_id = id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut timers_file = load_loop_timers(workspace_cwd)
        .map_err(|err| CreateLoopServiceError::Io(err.to_string()))?;
    if timers_file.timers.iter().any(|timer| timer.id == timer_id) {
        return Err(CreateLoopServiceError::InvalidRequest(format!(
            "loop `{timer_id}` already exists"
        )));
    }

    let created_at = Utc::now().timestamp();
    timers_file.timers.push(PersistedLoopTimer {
        id: timer_id.clone(),
        mode: if matches!(request.context_mode, LoopContextMode::Persistent) {
            LoopMode::Persistent
        } else {
            LoopMode::OneShot
        },
        prompt: prompt.to_string(),
        action: request
            .action
            .as_deref()
            .map(str::trim)
            .filter(|action| !action.is_empty())
            .map(ToOwned::to_owned),
        context_mode: request.context_mode,
        response_mode: request.response_mode,
        security_mode: request.security_mode,
        execution,
        schedule: match &trigger_kind {
            LoopTriggerKind::Timer { schedule } => schedule.clone(),
            LoopTriggerKind::Idle { after } => after.clone(),
            LoopTriggerKind::BeforeTurn | LoopTriggerKind::AfterTurn => LoopSchedule::Interval {
                display: "1h".to_string(),
                seconds: 60 * 60,
            },
        },
        trigger_bindings: vec![LoopTriggerBinding {
            id: "trigger-1".to_string(),
            enabled: true,
            kind: trigger_kind.clone(),
        }],
        enabled: true,
        rollout_path: None,
        created_at_unix_seconds: created_at,
        last_scheduled_at_unix_seconds: None,
        last_completed_at_unix_seconds: None,
    });
    timers_file
        .timers
        .sort_by(|left, right| left.id.cmp(&right.id));

    let mut queues = load_loop_trigger_queues(workspace_cwd)
        .map_err(|err| CreateLoopServiceError::Io(err.to_string()))?;
    let timers_by_id = timers_file
        .timers
        .iter()
        .cloned()
        .map(|timer| (timer.id.clone(), timer))
        .collect::<BTreeMap<_, _>>();
    sync_trigger_queues_with_timers(&mut queues, &timers_by_id);

    persist_loop_files(workspace_cwd, &timers_file, &queues)?;

    Ok(CreateLoopResult {
        id: timer_id,
        context_mode: request.context_mode,
        response_mode: request.response_mode,
        security_mode: request.security_mode,
        trigger_kind: match trigger_kind {
            LoopTriggerKind::Timer { .. } => "timer".to_string(),
            LoopTriggerKind::Idle { .. } => "idle".to_string(),
            LoopTriggerKind::BeforeTurn => "before_turn".to_string(),
            LoopTriggerKind::AfterTurn => "after_turn".to_string(),
        },
        timers_path: loop_timers_path(workspace_cwd).display().to_string(),
        trigger_queue_path: loop_trigger_queues_path(workspace_cwd)
            .display()
            .to_string(),
    })
}

fn persist_loop_files(
    workspace_cwd: &Path,
    timers_file: &PersistedLoopTimersFile,
    queues: &PersistedLoopTriggerQueuesFile,
) -> Result<(), CreateLoopServiceError> {
    let timers_path = loop_timers_path(workspace_cwd);
    let trigger_queues_path = loop_trigger_queues_path(workspace_cwd);
    if let Some(parent) = timers_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            CreateLoopServiceError::Io(format!(
                "failed to create loop workspace metadata directory: {err}"
            ))
        })?;
    }
    let timers_json = serde_json::to_string_pretty(timers_file).map_err(|err| {
        CreateLoopServiceError::Internal(format!("failed to serialize loop timers: {err}"))
    })?;
    fs::write(&timers_path, timers_json).map_err(|err| {
        CreateLoopServiceError::Io(format!("failed to persist loop timers: {err}"))
    })?;

    let queues_json = serde_json::to_string_pretty(queues).map_err(|err| {
        CreateLoopServiceError::Internal(format!("failed to serialize loop trigger queues: {err}"))
    })?;
    fs::write(&trigger_queues_path, queues_json).map_err(|err| {
        CreateLoopServiceError::Io(format!("failed to persist loop trigger queues: {err}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CreateLoopRequest;
    use super::CreateLoopResult;
    use super::CreateLoopServiceError;
    use super::CreateLoopTriggerRequest;
    use super::create_loop;
    use codex_loop::LoopContextMode;
    use codex_loop::LoopResponseMode;
    use codex_loop::LoopSecurityMode;
    use codex_loop::LoopTriggerPhase;
    use codex_loop::load_loop_timers;
    use codex_loop::load_loop_trigger_queues;
    use codex_loop::loop_timers_path;
    use codex_loop::loop_trigger_queues_path;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn create_persistent_loop_writes_timer_and_queue() {
        let temp = tempdir().expect("create tempdir");
        let docs = temp.path().join("docs");
        fs::create_dir_all(&docs).expect("create docs");

        let result = create_loop(
            CreateLoopRequest {
                id: Some("director".to_string()),
                prompt: "review progress".to_string(),
                action: None,
                context_mode: LoopContextMode::Persistent,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::SpecifiedDirectory,
                cwd: Some("docs".to_string()),
                writable_roots: vec!["docs".to_string()],
                trigger: CreateLoopTriggerRequest::AfterTurn,
            },
            temp.path(),
        )
        .expect("create loop");

        assert_eq!(
            result,
            CreateLoopResult {
                id: "director".to_string(),
                context_mode: LoopContextMode::Persistent,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::SpecifiedDirectory,
                trigger_kind: "after_turn".to_string(),
                timers_path: loop_timers_path(temp.path()).display().to_string(),
                trigger_queue_path: loop_trigger_queues_path(temp.path()).display().to_string(),
            }
        );

        let timers = load_loop_timers(temp.path()).expect("load timers");
        assert_eq!(timers.timers.len(), 1);
        assert_eq!(timers.timers[0].id, "director");
        assert_eq!(timers.timers[0].context_mode, LoopContextMode::Persistent);
        assert_eq!(
            timers.timers[0].execution.cwd,
            Some(std::path::PathBuf::from("docs"))
        );
        assert_eq!(
            timers.timers[0].execution.writable_roots,
            vec![std::path::PathBuf::from("docs")]
        );

        let queues = load_loop_trigger_queues(temp.path()).expect("load queues");
        assert_eq!(queues.queues.len(), 4);
        assert_eq!(
            codex_loop::queue_entries_for_phase(&queues, LoopTriggerPhase::AfterTurn)
                .iter()
                .map(|entry| (entry.loop_id.clone(), entry.binding_id.clone()))
                .collect::<Vec<_>>(),
            vec![("director".to_string(), "trigger-1".to_string())]
        );
    }

    #[test]
    fn create_loop_rejects_non_persistent_ids() {
        let temp = tempdir().expect("create tempdir");
        let err = create_loop(
            CreateLoopRequest {
                id: Some("temp-worker".to_string()),
                prompt: "check status".to_string(),
                action: None,
                context_mode: LoopContextMode::Ephemeral,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::Inherited,
                cwd: None,
                writable_roots: Vec::new(),
                trigger: CreateLoopTriggerRequest::BeforeTurn,
            },
            temp.path(),
        )
        .expect_err("reject id");

        assert_eq!(
            err,
            CreateLoopServiceError::InvalidRequest("only persistent loops may set id".to_string())
        );
    }

    #[test]
    fn create_loop_rejects_missing_specified_directory_roots() {
        let temp = tempdir().expect("create tempdir");
        let err = create_loop(
            CreateLoopRequest {
                id: None,
                prompt: "check status".to_string(),
                action: None,
                context_mode: LoopContextMode::Embed,
                response_mode: LoopResponseMode::User,
                security_mode: LoopSecurityMode::SpecifiedDirectory,
                cwd: None,
                writable_roots: Vec::new(),
                trigger: CreateLoopTriggerRequest::BeforeTurn,
            },
            temp.path(),
        )
        .expect_err("reject missing roots");

        assert_eq!(
            err,
            CreateLoopServiceError::InvalidRequest(
                "specified_directory requires at least one writable root".to_string()
            )
        );
    }

    #[test]
    fn create_loop_accepts_idle_trigger() {
        let temp = tempdir().expect("create tempdir");
        let result = create_loop(
            CreateLoopRequest {
                id: None,
                prompt: "summarize what changed".to_string(),
                action: None,
                context_mode: LoopContextMode::Ephemeral,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::Inherited,
                cwd: None,
                writable_roots: Vec::new(),
                trigger: CreateLoopTriggerRequest::Idle {
                    after: "15m".to_string(),
                },
            },
            temp.path(),
        )
        .expect("create idle loop");

        assert_eq!("idle", result.trigger_kind);
        let timers = load_loop_timers(temp.path()).expect("load timers");
        assert_eq!(
            "idle · 15m",
            timers.timers[0].trigger_bindings[0].selection_name()
        );
        let queues = load_loop_trigger_queues(temp.path()).expect("load queues");
        assert_eq!(
            codex_loop::queue_entries_for_phase(&queues, LoopTriggerPhase::Idle)
                .iter()
                .map(|entry| (entry.loop_id.clone(), entry.binding_id.clone()))
                .count(),
            1
        );
    }
}
