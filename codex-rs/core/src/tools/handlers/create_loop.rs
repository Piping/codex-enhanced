use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use async_trait::async_trait;
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
use codex_loop::load_loop_timers;
use codex_loop::load_loop_trigger_queues;
use codex_loop::loop_timers_path;
use codex_loop::loop_trigger_queues_path;
use codex_loop::parse_loop_cwd;
use codex_loop::parse_loop_schedule;
use codex_loop::parse_loop_writable_roots;
use codex_loop::sync_trigger_queues_with_timers;
use codex_loop::validate_loop_id;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct CreateLoopArgs {
    id: Option<String>,
    prompt: String,
    action: Option<String>,
    context_mode: LoopContextMode,
    #[serde(default)]
    response_mode: LoopResponseMode,
    #[serde(default)]
    security_mode: LoopSecurityMode,
    cwd: Option<String>,
    writable_roots: Option<Vec<String>>,
    trigger: CreateLoopTriggerArgs,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CreateLoopTriggerArgs {
    Timer { schedule: String },
    BeforeTurn,
    AfterTurn,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct CreateLoopResult {
    id: String,
    context_mode: LoopContextMode,
    response_mode: LoopResponseMode,
    security_mode: LoopSecurityMode,
    trigger_kind: String,
    timers_path: String,
    trigger_queue_path: String,
}

pub struct CreateLoopHandler;

#[async_trait]
impl ToolHandler for CreateLoopHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { payload, turn, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "create_loop handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CreateLoopArgs = parse_arguments(&arguments)?;
        let result = create_loop_in_workspace(args, turn.cwd.as_path())?;
        let text = serde_json::to_string(&result).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize create_loop response: {err}"))
        })?;
        Ok(FunctionToolOutput::from_text(text, Some(true)))
    }
}

fn create_loop_in_workspace(
    args: CreateLoopArgs,
    workspace_cwd: &Path,
) -> Result<CreateLoopResult, FunctionCallError> {
    let prompt = args.prompt.trim();
    if prompt.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "prompt must not be empty".to_string(),
        ));
    }

    let id = args
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    match args.context_mode {
        LoopContextMode::Persistent => {
            let Some(id) = id else {
                return Err(FunctionCallError::RespondToModel(
                    "persistent loops require an id".to_string(),
                ));
            };
            validate_loop_id(id).map_err(FunctionCallError::RespondToModel)?;
        }
        LoopContextMode::Embed | LoopContextMode::Ephemeral => {
            if id.is_some() {
                return Err(FunctionCallError::RespondToModel(
                    "only persistent loops may set id".to_string(),
                ));
            }
        }
    }

    let trigger_kind = match args.trigger {
        CreateLoopTriggerArgs::Timer { schedule } => LoopTriggerKind::Timer {
            schedule: parse_loop_schedule(schedule.trim())
                .map_err(FunctionCallError::RespondToModel)?,
        },
        CreateLoopTriggerArgs::BeforeTurn => LoopTriggerKind::BeforeTurn,
        CreateLoopTriggerArgs::AfterTurn => LoopTriggerKind::AfterTurn,
    };

    let writable_roots = args.writable_roots.unwrap_or_default();
    let execution = PersistedLoopExecutionSettings {
        cwd: args
            .cwd
            .as_deref()
            .filter(|cwd| !cwd.trim().is_empty())
            .map(|cwd| parse_loop_cwd(cwd, workspace_cwd))
            .transpose()
            .map_err(FunctionCallError::RespondToModel)?,
        writable_roots: if writable_roots.is_empty() {
            Vec::new()
        } else {
            parse_loop_writable_roots(&writable_roots.join("\n"), workspace_cwd)
                .map_err(FunctionCallError::RespondToModel)?
        },
    };

    match args.security_mode {
        LoopSecurityMode::Inherited => {
            if !execution.writable_roots.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "writable_roots requires security_mode set to specified_directory".to_string(),
                ));
            }
        }
        LoopSecurityMode::SpecifiedDirectory => {
            if execution.writable_roots.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "specified_directory requires at least one writable root".to_string(),
                ));
            }
        }
    }

    let timer_id = id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut timers_file = load_loop_timers(workspace_cwd)
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
    if timers_file.timers.iter().any(|timer| timer.id == timer_id) {
        return Err(FunctionCallError::RespondToModel(format!(
            "loop `{timer_id}` already exists"
        )));
    }

    let created_at = Utc::now().timestamp();
    timers_file.timers.push(PersistedLoopTimer {
        id: timer_id.clone(),
        mode: if matches!(args.context_mode, LoopContextMode::Persistent) {
            LoopMode::Persistent
        } else {
            LoopMode::OneShot
        },
        prompt: prompt.to_string(),
        action: args
            .action
            .as_deref()
            .map(str::trim)
            .filter(|action| !action.is_empty())
            .map(ToOwned::to_owned),
        context_mode: args.context_mode,
        response_mode: args.response_mode,
        security_mode: args.security_mode,
        execution,
        schedule: match &trigger_kind {
            LoopTriggerKind::Timer { schedule } => schedule.clone(),
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
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
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
        context_mode: args.context_mode,
        response_mode: args.response_mode,
        security_mode: args.security_mode,
        trigger_kind: match trigger_kind {
            LoopTriggerKind::Timer { .. } => "timer".to_string(),
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
    queues: &codex_loop::PersistedLoopTriggerQueuesFile,
) -> Result<(), FunctionCallError> {
    let timers_path = loop_timers_path(workspace_cwd);
    let trigger_queues_path = loop_trigger_queues_path(workspace_cwd);
    if let Some(parent) = timers_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to create loop workspace metadata directory: {err}"
            ))
        })?;
    }
    let timers_json = serde_json::to_string_pretty(timers_file).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize loop timers: {err}"))
    })?;
    fs::write(&timers_path, timers_json).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to persist loop timers: {err}"))
    })?;

    let queues_json = serde_json::to_string_pretty(queues).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize loop trigger queues: {err}"))
    })?;
    fs::write(&trigger_queues_path, queues_json).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to persist loop trigger queues: {err}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn create_persistent_loop_writes_timer_and_queue() {
        let temp = tempdir().expect("create tempdir");
        let docs = temp.path().join("docs");
        fs::create_dir_all(&docs).expect("create docs");

        let result = create_loop_in_workspace(
            CreateLoopArgs {
                id: Some("director".to_string()),
                prompt: "review progress".to_string(),
                action: None,
                context_mode: LoopContextMode::Persistent,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::SpecifiedDirectory,
                cwd: Some("docs".to_string()),
                writable_roots: Some(vec!["docs".to_string()]),
                trigger: CreateLoopTriggerArgs::AfterTurn,
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
        assert_eq!(queues.queues.len(), 3);
        assert_eq!(
            queue_entries_for_phase_for_test(&queues, "after_turn"),
            vec![("director".to_string(), "trigger-1".to_string())]
        );
    }

    #[test]
    fn create_loop_rejects_non_persistent_ids() {
        let temp = tempdir().expect("create tempdir");
        let err = create_loop_in_workspace(
            CreateLoopArgs {
                id: Some("temp-worker".to_string()),
                prompt: "check status".to_string(),
                action: None,
                context_mode: LoopContextMode::Ephemeral,
                response_mode: LoopResponseMode::Assistant,
                security_mode: LoopSecurityMode::Inherited,
                cwd: None,
                writable_roots: None,
                trigger: CreateLoopTriggerArgs::BeforeTurn,
            },
            temp.path(),
        )
        .expect_err("reject id");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel("only persistent loops may set id".to_string())
        );
    }

    #[test]
    fn create_loop_rejects_missing_specified_directory_roots() {
        let temp = tempdir().expect("create tempdir");
        let err = create_loop_in_workspace(
            CreateLoopArgs {
                id: None,
                prompt: "check status".to_string(),
                action: None,
                context_mode: LoopContextMode::Embed,
                response_mode: LoopResponseMode::User,
                security_mode: LoopSecurityMode::SpecifiedDirectory,
                cwd: None,
                writable_roots: None,
                trigger: CreateLoopTriggerArgs::BeforeTurn,
            },
            temp.path(),
        )
        .expect_err("reject missing roots");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "specified_directory requires at least one writable root".to_string()
            )
        );
    }

    fn queue_entries_for_phase_for_test(
        queues: &codex_loop::PersistedLoopTriggerQueuesFile,
        phase: &str,
    ) -> Vec<(String, String)> {
        let phase = match phase {
            "timer" => codex_loop::LoopTriggerPhase::Timer,
            "before_turn" => codex_loop::LoopTriggerPhase::BeforeTurn,
            "after_turn" => codex_loop::LoopTriggerPhase::AfterTurn,
            _ => panic!("unexpected phase"),
        };
        codex_loop::queue_entries_for_phase(queues, phase)
            .iter()
            .map(|entry| (entry.loop_id.clone(), entry.binding_id.clone()))
            .collect()
    }
}
