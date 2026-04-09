use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use chrono::DateTime;
use chrono::Utc;
use codex_core::config::Config;
use codex_protocol::ThreadId;
use codex_protocol::protocol::DynamicToolCallResponseEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandStatus;
use codex_protocol::protocol::PatchApplyStatus;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::RolloutLine;
use codex_protocol::protocol::SessionMeta;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::protocol::USER_MESSAGE_BEGIN;
use codex_rollout::ARCHIVED_SESSIONS_SUBDIR;
use codex_rollout::SESSIONS_SUBDIR;

use super::types::AggregateMetrics;
use super::types::CollectedThread;
use super::types::CollectionResult;

pub(crate) async fn collect_sessions(config: &Config) -> anyhow::Result<CollectionResult> {
    let mut rollout_paths = Vec::new();
    collect_rollout_paths(
        config.codex_home.join(SESSIONS_SUBDIR),
        /*archived*/ false,
        &mut rollout_paths,
    )
    .await?;
    collect_rollout_paths(
        config.codex_home.join(ARCHIVED_SESSIONS_SUBDIR),
        /*archived*/ true,
        &mut rollout_paths,
    )
    .await?;

    let scanned_files = rollout_paths.len();
    let mut skipped_files = 0usize;
    let mut threads = Vec::new();
    for (path, archived) in rollout_paths {
        match analyze_rollout(path.clone(), archived).await {
            Ok(Some(thread)) => threads.push(thread),
            Ok(None) => skipped_files = skipped_files.saturating_add(1),
            Err(err) => {
                tracing::warn!("failed to analyze rollout {}: {err}", path.display());
                skipped_files = skipped_files.saturating_add(1);
            }
        }
    }

    Ok(CollectionResult {
        threads,
        scanned_files,
        skipped_files,
    })
}

async fn collect_rollout_paths(
    root: PathBuf,
    archived: bool,
    paths: &mut Vec<(PathBuf, bool)>,
) -> anyhow::Result<()> {
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", dir.display()));
            }
        };
        while let Some(entry) = entries.next_entry().await? {
            let entry_type = entry.file_type().await?;
            let path = entry.path();
            if entry_type.is_dir() {
                pending.push(path);
            } else if entry_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
            {
                paths.push((path, archived));
            }
        }
    }
    Ok(())
}

async fn analyze_rollout(path: PathBuf, archived: bool) -> anyhow::Result<Option<CollectedThread>> {
    let text = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(None);
    }

    let file_updated_at = tokio::fs::metadata(&path)
        .await
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(DateTime::<Utc>::from);

    let mut session_meta: Option<SessionMeta> = None;
    let mut metrics = AggregateMetrics::default();
    let mut first_event_at = None;
    let mut last_event_at = None;
    let mut last_turn_complete_at = None;
    let mut title = String::new();

    for raw_line in text.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }

        let rollout_line = match serde_json::from_str::<RolloutLine>(raw_line) {
            Ok(rollout_line) => rollout_line,
            Err(_) => {
                metrics.parse_errors = metrics.parse_errors.saturating_add(1);
                continue;
            }
        };

        let timestamp = parse_timestamp(rollout_line.timestamp.as_str());
        if first_event_at.is_none() {
            first_event_at = timestamp;
        }
        if timestamp.is_some() {
            last_event_at = timestamp;
        }

        match rollout_line.item {
            RolloutItem::SessionMeta(meta_line) => {
                if session_meta.is_none() {
                    session_meta = Some(meta_line.meta);
                }
            }
            RolloutItem::EventMsg(event) => match event {
                EventMsg::UserMessage(user) => {
                    metrics.user_message_count = metrics.user_message_count.saturating_add(1);
                    if title.is_empty() {
                        let stripped = strip_user_message_prefix(user.message.as_str());
                        if !stripped.is_empty() {
                            title = stripped.to_string();
                        }
                    }
                    if let (Some(turn_complete_at), Some(user_message_at)) =
                        (last_turn_complete_at.take(), timestamp)
                        && user_message_at > turn_complete_at
                    {
                        metrics.estimated_user_wait = metrics
                            .estimated_user_wait
                            .saturating_add((user_message_at - turn_complete_at).to_std()?);
                    }
                }
                EventMsg::TurnComplete(_) => {
                    metrics.completed_turn_count = metrics.completed_turn_count.saturating_add(1);
                    last_turn_complete_at = timestamp;
                }
                EventMsg::TokenCount(token_count) => {
                    if let Some(info) = token_count.info {
                        metrics.total_tokens = info.total_token_usage.total_tokens.max(0);
                        metrics.input_tokens = info.total_token_usage.input_tokens.max(0);
                        metrics.output_tokens = info.total_token_usage.output_tokens.max(0);
                        metrics.reasoning_output_tokens =
                            info.total_token_usage.reasoning_output_tokens.max(0);
                    }
                }
                EventMsg::ExecCommandEnd(exec) => {
                    let failed =
                        matches!(exec.status, ExecCommandStatus::Failed) || exec.exit_code != 0;
                    metrics.exec_commands.add_sample(failed, exec.duration);
                }
                EventMsg::McpToolCallEnd(tool_call) => {
                    metrics
                        .mcp_tool_calls
                        .add_sample(!tool_call.is_success(), tool_call.duration);
                }
                EventMsg::DynamicToolCallResponse(DynamicToolCallResponseEvent {
                    success,
                    duration,
                    ..
                }) => {
                    metrics.dynamic_tool_calls.add_sample(!success, duration);
                }
                EventMsg::WebSearchEnd(_) => {
                    metrics.web_search_count = metrics.web_search_count.saturating_add(1);
                }
                EventMsg::ImageGenerationEnd(_) => {
                    metrics.image_generation_count =
                        metrics.image_generation_count.saturating_add(1);
                }
                EventMsg::ViewImageToolCall(_) => {
                    metrics.view_image_count = metrics.view_image_count.saturating_add(1);
                }
                EventMsg::PatchApplyEnd(patch) => {
                    let failed = matches!(patch.status, PatchApplyStatus::Failed) || !patch.success;
                    metrics.patches.add_sample(failed, patch.changes.len());
                }
                EventMsg::Error(_) | EventMsg::StreamError(_) => {
                    metrics.api_error_count = metrics.api_error_count.saturating_add(1);
                }
                _ => {}
            },
            RolloutItem::ResponseItem(_)
            | RolloutItem::Compacted(_)
            | RolloutItem::TurnContext(_) => {}
        }
    }

    let Some(session_meta) = session_meta else {
        return Ok(None);
    };

    let created_at = parse_timestamp(session_meta.timestamp.as_str())
        .or(first_event_at)
        .or(file_updated_at)
        .unwrap_or_else(Utc::now);
    let updated_at = last_event_at.or(file_updated_at).unwrap_or(created_at);
    let wall_clock_span = match (first_event_at, last_event_at) {
        (Some(first), Some(last)) if last > first => (last - first).to_std()?,
        _ => Duration::ZERO,
    };
    metrics.cumulative_thread_span = wall_clock_span;
    metrics.residual_runtime_estimate = wall_clock_span
        .saturating_sub(metrics.exact_tool_runtime())
        .saturating_sub(metrics.estimated_user_wait);

    let (parent_thread_id, depth, source_label, agent_nickname, agent_role, agent_path) =
        source_fields(&session_meta.source);

    Ok(Some(CollectedThread {
        thread_id: session_meta.id,
        parent_thread_id,
        depth,
        title: if title.is_empty() {
            fallback_title(&path)
        } else {
            title
        },
        cwd: session_meta.cwd,
        rollout_path: path,
        archived,
        source_label,
        agent_nickname,
        agent_role,
        agent_path,
        created_at,
        updated_at,
        first_event_at,
        last_event_at,
        metrics,
    }))
}

fn source_fields(
    source: &SessionSource,
) -> (
    Option<ThreadId>,
    Option<i32>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    match source {
        SessionSource::Cli => (None, None, "cli".to_string(), None, None, None),
        SessionSource::VSCode => (None, None, "vscode".to_string(), None, None, None),
        SessionSource::Exec => (None, None, "exec".to_string(), None, None, None),
        SessionSource::Mcp => (None, None, "mcp".to_string(), None, None, None),
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id,
            depth,
            agent_path,
            agent_nickname,
            agent_role,
        }) => (
            Some(*parent_thread_id),
            Some(*depth),
            format!("subagent/thread_spawn:d{depth}"),
            agent_nickname.clone(),
            agent_role.clone(),
            agent_path.clone().map(|path| path.to_string()),
        ),
        SessionSource::SubAgent(other) => (
            None,
            None,
            format!("subagent/{other}"),
            source.get_nickname(),
            source.get_agent_role(),
            source.get_agent_path().map(|path| path.to_string()),
        ),
        SessionSource::Custom(other) => (None, None, other.clone(), None, None, None),
        SessionSource::Unknown => (None, None, "unknown".to_string(), None, None, None),
    }
}

fn parse_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn strip_user_message_prefix(text: &str) -> &str {
    match text.find(USER_MESSAGE_BEGIN) {
        Some(index) => text[index + USER_MESSAGE_BEGIN.len()..].trim(),
        None => text.trim(),
    }
}

fn fallback_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("untitled")
        .to_string()
}
