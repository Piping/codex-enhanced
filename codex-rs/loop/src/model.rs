use chrono::DateTime;
use chrono::TimeZone;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::command::LoopDeliveryMode;
use crate::command::LoopMode;
use crate::command::LoopSchedule;
use crate::execution::PersistedLoopExecutionSettings;

const LOOP_TIMER_FILE_NAME: &str = "loop_timers.json";

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
    pub delivery_mode: Option<LoopDeliveryMode>,
    #[serde(default)]
    pub execution: PersistedLoopExecutionSettings,
    pub schedule: LoopSchedule,
    pub enabled: bool,
    #[serde(default)]
    pub rollout_path: Option<PathBuf>,
    pub created_at_unix_seconds: i64,
    pub last_scheduled_at_unix_seconds: Option<i64>,
    pub last_completed_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopTimerCompletionPlan {
    pub summary_message: String,
    pub mirror_prompt: bool,
    pub followup_user_message: Option<String>,
}

pub fn effective_loop_delivery_mode(timer: Option<&PersistedLoopTimer>) -> LoopDeliveryMode {
    timer
        .and_then(|timer| timer.delivery_mode)
        .unwrap_or_default()
}

pub fn timer_descriptor(timer: &PersistedLoopTimer) -> &'static str {
    match timer.mode {
        LoopMode::OneShot => "one-shot",
        LoopMode::Persistent => "persistent",
    }
}

pub fn loop_item_name(timer: &PersistedLoopTimer) -> String {
    match timer.mode {
        LoopMode::OneShot => format!("one-shot {}", prompt_prefix(&timer.prompt)),
        LoopMode::Persistent => timer.id.clone(),
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

pub fn build_loop_run_input(prompt: &str, recent_main_messages: &[String]) -> String {
    if recent_main_messages.is_empty() {
        return prompt.to_string();
    }
    let recent_messages = recent_main_messages.join("\n\n");
    format!("Recent main-thread messages:\n{recent_messages}\n\nOriginal loop prompt:\n{prompt}")
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
    match timer.last_scheduled_at_unix_seconds {
        Some(last_scheduled_at) => Some(timer.schedule.next_due_after(last_scheduled_at, now)),
        None => Some(timer.schedule.first_due_after_creation(now)),
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
    cwd.join(".codex").join(LOOP_TIMER_FILE_NAME)
}

pub fn format_timestamp(unix_seconds: i64) -> String {
    unix_seconds_to_utc(unix_seconds)
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| unix_seconds.to_string())
}

fn unix_seconds_to_utc(unix_seconds: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(unix_seconds, 0).single()
}
