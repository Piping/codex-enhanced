use chrono::DateTime;
use chrono::TimeZone;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::command::LoopMode;
use crate::command::LoopSchedule;
use crate::execution::PersistedLoopExecutionSettings;
use crate::trigger::LoopContextMode;
use crate::trigger::LoopResponseMode;
use crate::trigger::LoopSecurityMode;
use crate::trigger::LoopTriggerBinding;
use crate::trigger::LoopTriggerKind;

const LOOP_TIMER_FILE_NAME: &str = "loop_timers.json";
const LOOP_METADATA_DIR_NAME: &str = "loop";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedLoopTimersFile {
    #[serde(default)]
    pub timers: Vec<PersistedLoopTimer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedLoopTimer {
    pub id: String,
    #[serde(default)]
    pub mode: LoopMode,
    pub prompt: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub context_mode: LoopContextMode,
    #[serde(default)]
    pub response_mode: LoopResponseMode,
    #[serde(default)]
    pub security_mode: LoopSecurityMode,
    #[serde(default)]
    pub execution: PersistedLoopExecutionSettings,
    /// Legacy top-level timer schedule retained for backward compatibility while loops migrate to
    /// explicit trigger bindings.
    pub schedule: LoopSchedule,
    #[serde(default)]
    pub trigger_bindings: Vec<LoopTriggerBinding>,
    pub enabled: bool,
    #[serde(default)]
    pub rollout_path: Option<PathBuf>,
    pub created_at_unix_seconds: i64,
    pub last_scheduled_at_unix_seconds: Option<i64>,
    pub last_completed_at_unix_seconds: Option<i64>,
}

pub fn effective_loop_context_mode(timer: Option<&PersistedLoopTimer>) -> LoopContextMode {
    timer.map_or(LoopContextMode::default(), |timer| timer.context_mode)
}

pub fn effective_loop_response_mode(timer: Option<&PersistedLoopTimer>) -> LoopResponseMode {
    timer.map_or(LoopResponseMode::default(), |timer| timer.response_mode)
}

pub fn effective_loop_security_mode(timer: Option<&PersistedLoopTimer>) -> LoopSecurityMode {
    timer.map_or(LoopSecurityMode::default(), |timer| timer.security_mode)
}

pub fn trigger_bindings(timer: &PersistedLoopTimer) -> Vec<LoopTriggerBinding> {
    timer.trigger_bindings.clone()
}

pub fn effective_timer_schedule(timer: &PersistedLoopTimer) -> Option<LoopSchedule> {
    timer
        .trigger_bindings
        .iter()
        .find_map(|binding| match &binding.kind {
            LoopTriggerKind::Timer { schedule } if binding.enabled => Some(schedule.clone()),
            _ => None,
        })
}

pub fn timer_descriptor(timer: &PersistedLoopTimer) -> &'static str {
    match timer.context_mode {
        LoopContextMode::Embed => "embed",
        LoopContextMode::Ephemeral => "ephemeral",
        LoopContextMode::Persistent => "persistent",
    }
}

pub fn loop_item_name(timer: &PersistedLoopTimer) -> String {
    match timer.context_mode {
        LoopContextMode::Embed => format!("embed #{}", loop_id_prefix(&timer.id)),
        LoopContextMode::Ephemeral => format!("ephemeral #{}", loop_id_prefix(&timer.id)),
        LoopContextMode::Persistent => timer.id.clone(),
    }
}

pub fn loop_id_prefix(id: &str) -> String {
    id.chars().take(8).collect()
}

pub fn prompt_prefix(prompt: &str) -> String {
    let prefix = prompt.chars().take(48).collect::<String>();
    if prompt.chars().count() > 48 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

pub fn build_loop_result_user_message_with_action(result: &str, action: Option<&str>) -> String {
    let Some(action) = action.map(str::trim).filter(|action| !action.is_empty()) else {
        return result.to_string();
    };
    format!("{result}\n\nAdditional action:\n{action}")
}

pub fn next_due_for_timer(timer: &PersistedLoopTimer, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if !timer.enabled {
        return None;
    }
    let schedule = effective_timer_schedule(timer)?;
    match timer.last_scheduled_at_unix_seconds {
        Some(last_scheduled_at) => Some(schedule.next_due_after(last_scheduled_at, now)),
        None => Some(schedule.first_due_after_creation(now)),
    }
}

pub fn load_loop_timers(cwd: &Path) -> std::io::Result<PersistedLoopTimersFile> {
    let path = loop_timers_path(cwd);
    if !path.exists() {
        return Ok(PersistedLoopTimersFile { timers: Vec::new() });
    }
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(std::io::Error::other)
}

pub fn loop_timers_path(cwd: &Path) -> PathBuf {
    cwd.join(".codex")
        .join(LOOP_METADATA_DIR_NAME)
        .join(LOOP_TIMER_FILE_NAME)
}

pub fn format_timestamp(unix_seconds: i64) -> String {
    unix_seconds_to_utc(unix_seconds)
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| unix_seconds.to_string())
}

fn unix_seconds_to_utc(unix_seconds: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(unix_seconds, 0).single()
}
