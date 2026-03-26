use super::App;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell::AgentMessageCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::UserHistoryCell;
use crate::history_cell::new_info_event;
use crate::markdown::append_markdown;
use chrono::DateTime;
use chrono::TimeZone;
use chrono::Utc;
use codex_core::CodexThread;
use codex_core::RolloutRecorder;
use codex_core::config::types::TuiLoopCompletionMirrorMode;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReadOnlyAccess;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use cron::Schedule;
use ratatui::text::Line;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

const LOOP_TIMERS_VIEW_ID: &str = "fork-loop-timers-panel";
const LOOP_TIMER_ACTIONS_VIEW_ID: &str = "fork-loop-timer-actions-panel";
const LOOP_CONTEXT_BUDGET_TOKENS: usize = 2_000;
const LOOP_DEVELOPER_INSTRUCTIONS: &str = concat!(
    "This is a hidden `/loop` execution thread. ",
    "Use the current main-thread context only as read-only background. ",
    "Do not write files, apply patches, spawn agents, or perform side-effectful actions. ",
    "Return only the answer for this scheduled prompt."
);
const LOOP_TIMER_FILE_NAME: &str = "loop_timers.json";

#[derive(Default)]
pub(crate) struct LoopTimersState {
    workspace_cwd: Option<PathBuf>,
    timers: BTreeMap<String, PersistedLoopTimer>,
    scheduler_tasks: HashMap<String, JoinHandle<()>>,
    active_runs: HashMap<String, ActiveLoopRun>,
    pub(super) thread_history_cells: HashMap<ThreadId, Vec<Arc<dyn HistoryCell>>>,
}

struct ActiveLoopRun {
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    listener_handle: JoinHandle<()>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedLoopTimersFile {
    timers: Vec<PersistedLoopTimer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedLoopTimer {
    id: String,
    prompt: String,
    schedule: LoopSchedule,
    enabled: bool,
    created_at_unix_seconds: i64,
    last_scheduled_at_unix_seconds: Option<i64>,
    last_completed_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LoopSchedule {
    Interval { display: String, seconds: u64 },
    Cron { display: String, normalized: String },
}

impl LoopSchedule {
    fn display(&self) -> &str {
        match self {
            Self::Interval { display, .. } | Self::Cron { display, .. } => display,
        }
    }

    fn next_due_after(
        &self,
        last_scheduled_at_unix_seconds: i64,
        now: DateTime<Utc>,
    ) -> DateTime<Utc> {
        match self {
            Self::Interval { seconds, .. } => {
                let interval = i64::try_from(*seconds).unwrap_or(i64::MAX).max(1);
                let next = if last_scheduled_at_unix_seconds >= now.timestamp() {
                    last_scheduled_at_unix_seconds.saturating_add(interval)
                } else {
                    let elapsed = now
                        .timestamp()
                        .saturating_sub(last_scheduled_at_unix_seconds);
                    let skipped_intervals = elapsed / interval;
                    last_scheduled_at_unix_seconds.saturating_add(
                        skipped_intervals.saturating_add(1).saturating_mul(interval),
                    )
                };
                unix_seconds_to_utc(next).unwrap_or(now)
            }
            Self::Cron { normalized, .. } => Schedule::from_str(normalized)
                .ok()
                .and_then(|schedule| {
                    unix_seconds_to_utc(last_scheduled_at_unix_seconds)
                        .and_then(|last| schedule.after(&last).next())
                })
                .map(|next| next.with_timezone(&Utc))
                .filter(|next| *next > now)
                .unwrap_or(now),
        }
    }
}

impl App {
    pub(crate) fn open_loop_timers_panel(&mut self) {
        self.ensure_loop_timers_loaded();

        let path = loop_timers_path(self.config.cwd.as_path());
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(LOOP_TIMERS_VIEW_ID);
        let subtitle = Some(format!(
            "{} timer(s) configured for {}.",
            self.loop_timers.timers.len(),
            self.config.cwd.display()
        ));

        let mut items = self
            .loop_timers
            .timers
            .values()
            .map(loop_timer_selection_item)
            .collect::<Vec<_>>();

        if items.is_empty() {
            items.push(SelectionItem {
                name: "No loop timers yet".to_string(),
                description: Some(
                    "Create one with `/loop 5m <prompt>` or `/loop <cron> <prompt>`.".to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            });
        }

        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TIMERS_VIEW_ID),
            title: Some("Loop Timers".to_string()),
            subtitle,
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(path.display().to_string()),
            initial_selected_idx,
            items,
            ..Default::default()
        });
    }

    pub(crate) fn create_loop_timer(&mut self, spec: String) {
        self.ensure_loop_timers_loaded();

        let parsed = match parse_loop_spec(spec.trim()) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to create `/loop`: {err}"));
                return;
            }
        };

        let now = Utc::now();
        let timer = PersistedLoopTimer {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: parsed.prompt,
            schedule: parsed.schedule,
            enabled: true,
            created_at_unix_seconds: now.timestamp(),
            last_scheduled_at_unix_seconds: None,
            last_completed_at_unix_seconds: None,
        };

        let timer_id = timer.id.clone();
        let prompt = timer.prompt.clone();
        let schedule_display = timer.schedule.display().to_string();
        self.loop_timers.timers.insert(timer.id.clone(), timer);
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to persist `/loop` timer: {err}"));
            self.loop_timers.timers.remove(&timer_id);
            return;
        }

        self.schedule_loop_timer(&timer_id, now);
        self.chat_widget.add_info_message(
            format!("Created `/loop` timer: {schedule_display} -> {prompt}"),
            /*hint*/ None,
        );
        self.app_event_tx.send(AppEvent::TriggerLoopTimer {
            timer_id,
            scheduled_for_unix_seconds: now.timestamp(),
        });
    }

    pub(crate) fn open_loop_timer_actions(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };

        let enabled_action = if timer.enabled {
            SelectionItem {
                name: "Disable".to_string(),
                description: Some("Stop future scheduled executions.".to_string()),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::DisableLoopTimer {
                        timer_id: timer_id.clone(),
                    })
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        } else {
            SelectionItem {
                name: "Enable".to_string(),
                description: Some("Resume future scheduled executions.".to_string()),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::EnableLoopTimer {
                        timer_id: timer_id.clone(),
                    })
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        };

        let timer_id_for_delete = timer.id.clone();
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TIMER_ACTIONS_VIEW_ID),
            title: Some("Loop Timer".to_string()),
            subtitle: Some(format!("{} -> {}", timer.schedule.display(), timer.prompt)),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                enabled_action,
                SelectionItem {
                    name: "Delete".to_string(),
                    description: Some("Remove this timer from the current workspace.".to_string()),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::DeleteLoopTimer {
                            timer_id: timer_id_for_delete.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenLoopTimersPanel))),
            ..Default::default()
        });
    }

    pub(crate) fn set_loop_timer_enabled(&mut self, timer_id: String, enabled: bool) {
        self.ensure_loop_timers_loaded();
        let next_due = {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                return;
            };
            timer.enabled = enabled;
            enabled.then(|| timer.clone())
        };
        if !enabled {
            self.stop_loop_timer_scheduler(&timer_id);
            self.stop_loop_timer_run(&timer_id);
        } else if let Some(timer) = next_due
            && let Some(due) = self.next_due_for_timer(&timer, Utc::now())
        {
            self.schedule_loop_timer(&timer_id, due);
        }
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer: {err}"));
        }
        self.open_loop_timers_panel();
    }

    pub(crate) fn delete_loop_timer(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        self.stop_loop_timer_scheduler(&timer_id);
        self.stop_loop_timer_run(&timer_id);
        self.loop_timers.timers.remove(&timer_id);
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to delete loop timer: {err}"));
        }
        self.open_loop_timers_panel();
    }

    pub(crate) async fn trigger_loop_timer(
        &mut self,
        timer_id: String,
        scheduled_for_unix_seconds: i64,
    ) {
        self.ensure_loop_timers_loaded();
        let now = Utc::now();
        let timer = {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                return;
            };
            if !timer.enabled {
                return;
            }
            timer.last_scheduled_at_unix_seconds = Some(scheduled_for_unix_seconds);
            timer.clone()
        };
        let next_due = self.next_due_for_timer(&timer, now);
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer schedule: {err}"));
        }
        if let Some(next_due) = next_due {
            self.schedule_loop_timer(&timer_id, next_due);
        }

        if self.loop_timers.active_runs.contains_key(&timer_id) {
            return;
        };
        let prompt = timer.prompt.clone();
        let mut loop_config = self.config.clone();
        loop_config.ephemeral = true;
        loop_config.include_apply_patch_tool = false;
        if let Err(err) = loop_config
            .permissions
            .approval_policy
            .set(AskForApproval::Never)
        {
            self.chat_widget
                .add_error_message(format!("Failed to configure `/loop` approvals: {err}"));
            return;
        }
        if let Err(err) = loop_config
            .permissions
            .sandbox_policy
            .set(SandboxPolicy::ReadOnly {
                access: ReadOnlyAccess::default(),
                network_access: false,
            })
        {
            self.chat_widget
                .add_error_message(format!("Failed to configure `/loop` sandbox: {err}"));
            return;
        }
        loop_config.developer_instructions =
            Some(match loop_config.developer_instructions.take() {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{existing}\n\n{LOOP_DEVELOPER_INSTRUCTIONS}")
                }
                _ => LOOP_DEVELOPER_INSTRUCTIONS.to_string(),
            });

        let initial_history = build_loop_initial_history(
            self.primary_session_configured
                .as_ref()
                .and_then(|event| event.rollout_path.as_deref()),
        )
        .await;

        let new_thread = match self
            .server
            .start_thread_with_history_and_source(
                loop_config,
                initial_history,
                SessionSource::SubAgent(SubAgentSource::Other("loop".to_string())),
            )
            .await
        {
            Ok(new_thread) => new_thread,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to start `/loop` execution: {err}"));
                return;
            }
        };

        let thread_id = new_thread.thread_id;
        let thread = new_thread.thread;
        let app_event_tx = self.app_event_tx.clone();
        let listener_thread = Arc::clone(&thread);
        let timer_id_for_event = timer_id.clone();
        let prompt_for_event = prompt.clone();
        let listener_handle = tokio::spawn(async move {
            let mut last_agent_message = None;
            loop {
                match listener_thread.next_event().await {
                    Ok(event) => match event.msg {
                        EventMsg::ItemCompleted(item_completed) => {
                            if let TurnItem::AgentMessage(message) = item_completed.item {
                                let text = message
                                    .content
                                    .into_iter()
                                    .map(|content| match content {
                                        AgentMessageContent::Text { text } => text,
                                    })
                                    .collect::<String>();
                                if !text.trim().is_empty() {
                                    last_agent_message = Some(text);
                                }
                            }
                        }
                        EventMsg::TurnComplete(turn_complete) => {
                            let result = turn_complete
                                .last_agent_message
                                .or(last_agent_message)
                                .ok_or_else(|| {
                                    "Scheduled loop execution finished without a final answer."
                                        .to_string()
                                });
                            app_event_tx.send(AppEvent::LoopTimerCompleted {
                                timer_id: timer_id_for_event.clone(),
                                prompt: prompt_for_event.clone(),
                                result,
                            });
                            break;
                        }
                        EventMsg::Error(error) => {
                            app_event_tx.send(AppEvent::LoopTimerCompleted {
                                timer_id: timer_id_for_event.clone(),
                                prompt: prompt_for_event.clone(),
                                result: Err(error.message),
                            });
                            break;
                        }
                        EventMsg::ShutdownComplete => {
                            break;
                        }
                        _ => {}
                    },
                    Err(err) => {
                        app_event_tx.send(AppEvent::LoopTimerCompleted {
                            timer_id: timer_id_for_event.clone(),
                            prompt: prompt_for_event.clone(),
                            result: Err(format!("Scheduled loop execution failed: {err}")),
                        });
                        break;
                    }
                }
            }
        });

        self.loop_timers.active_runs.insert(
            timer_id.clone(),
            ActiveLoopRun {
                thread_id,
                thread: Arc::clone(&thread),
                listener_handle,
            },
        );

        if let Err(err) = thread
            .submit(Op::UserInput {
                items: vec![codex_protocol::user_input::UserInput::Text {
                    text: prompt,
                    text_elements: Vec::new(),
                }],
                final_output_json_schema: None,
            })
            .await
        {
            self.chat_widget
                .add_error_message(format!("Failed to submit `/loop` prompt: {err}"));
            self.stop_loop_timer_run(&timer_id);
        }
    }

    pub(crate) fn finish_loop_timer(
        &mut self,
        timer_id: String,
        prompt: String,
        result: Result<String, String>,
    ) -> Vec<Arc<dyn HistoryCell>> {
        self.ensure_loop_timers_loaded();
        self.stop_loop_timer_run(&timer_id);

        if let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) {
            timer.last_completed_at_unix_seconds = Some(Utc::now().timestamp());
            if let Err(err) = self.persist_loop_timers() {
                self.chat_widget
                    .add_error_message(format!("Failed to persist loop timer completion: {err}"));
            }
        }

        match result {
            Ok(message) => {
                let Some(primary_thread_id) = self.primary_thread_id else {
                    return Vec::new();
                };
                let timer_summary = self
                    .loop_timers
                    .timers
                    .get(&timer_id)
                    .map(|timer| {
                        let prompt_prefix = prompt.chars().take(48).collect::<String>();
                        let prompt_prefix = if prompt.chars().count() > 48 {
                            format!("{prompt_prefix}...")
                        } else {
                            prompt_prefix
                        };
                        let loop_id_prefix = timer.id.chars().take(8).collect::<String>();
                        format!(
                            "Loop {loop_id_prefix} ({}) ran: {prompt_prefix}",
                            timer.schedule.display()
                        )
                    })
                    .unwrap_or_else(|| {
                        let prompt_prefix = prompt.chars().take(48).collect::<String>();
                        let prompt_prefix = if prompt.chars().count() > 48 {
                            format!("{prompt_prefix}...")
                        } else {
                            prompt_prefix
                        };
                        let loop_id_prefix = timer_id.chars().take(8).collect::<String>();
                        format!("Loop {loop_id_prefix} ran: {prompt_prefix}")
                    });
                let summary_cell: Arc<dyn HistoryCell> =
                    Arc::new(new_info_event(timer_summary, /*hint*/ None));
                let assistant_cell: Arc<dyn HistoryCell> =
                    Arc::new(loop_result_cell(&message, self.config.cwd.as_path()));
                let mut mirrored_cells = Vec::new();
                self.record_thread_history_cell(primary_thread_id, summary_cell.clone());
                mirrored_cells.push(summary_cell);
                match self.config.tui_loop_completion_mirror_mode {
                    TuiLoopCompletionMirrorMode::PromptAndResponse => {
                        // Mirror only the scheduled prompt and the latest final answer back into
                        // the main thread. The hidden `/loop` execution history stays private.
                        let user_cell: Arc<dyn HistoryCell> = Arc::new(UserHistoryCell {
                            message: format!("/loop {prompt}"),
                            text_elements: Vec::new(),
                            local_image_paths: Vec::new(),
                            remote_image_urls: Vec::new(),
                        });
                        self.record_thread_history_cell(primary_thread_id, user_cell.clone());
                        mirrored_cells.push(user_cell);
                    }
                    TuiLoopCompletionMirrorMode::ResponseOnly => {}
                }
                self.record_thread_history_cell(primary_thread_id, assistant_cell.clone());
                mirrored_cells.push(assistant_cell);
                if self.active_thread_id == Some(primary_thread_id) {
                    mirrored_cells
                } else {
                    Vec::new()
                }
            }
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("A `/loop` run failed: {err}"));
                Vec::new()
            }
        }
    }

    pub(crate) fn replay_loop_history_cells_for_active_thread(&mut self) {
        let Some(thread_id) = self.active_thread_id else {
            return;
        };
        let Some(cells) = self.loop_timers.thread_history_cells.get(&thread_id) else {
            return;
        };
        let width = 80;
        for cell in cells {
            self.transcript_cells.push(cell.clone());
            let mut display = cell.display_lines(width);
            if !display.is_empty() {
                if !cell.is_stream_continuation() {
                    if self.has_emitted_history_lines {
                        display.insert(0, Line::default());
                    } else {
                        self.has_emitted_history_lines = true;
                    }
                }
                self.deferred_history_lines.extend(display);
            }
        }
    }

    pub(crate) fn ensure_loop_timers_loaded(&mut self) {
        if self.loop_timers.workspace_cwd.as_deref() == Some(self.config.cwd.as_path()) {
            return;
        }

        self.stop_all_loop_timer_tasks();
        self.loop_timers.workspace_cwd = Some(self.config.cwd.clone());
        self.loop_timers.thread_history_cells.clear();

        let loaded = load_loop_timers(self.config.cwd.as_path())
            .unwrap_or_default()
            .timers
            .into_iter()
            .map(|timer| (timer.id.clone(), timer))
            .collect::<BTreeMap<_, _>>();
        self.loop_timers.timers = loaded;

        let now = Utc::now();
        let due_entries = self
            .loop_timers
            .timers
            .values()
            .filter(|timer| timer.enabled)
            .filter_map(|timer| {
                self.next_due_for_timer(timer, now)
                    .map(|due| (timer.id.clone(), due))
            })
            .collect::<Vec<_>>();
        for (timer_id, due) in due_entries {
            self.schedule_loop_timer(&timer_id, due);
        }
    }

    fn next_due_for_timer(
        &self,
        timer: &PersistedLoopTimer,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        if !timer.enabled {
            return None;
        }
        match timer.last_scheduled_at_unix_seconds {
            Some(last_scheduled_at) => Some(timer.schedule.next_due_after(last_scheduled_at, now)),
            None => Some(now),
        }
    }

    fn schedule_loop_timer(&mut self, timer_id: &str, due_at: DateTime<Utc>) {
        self.stop_loop_timer_scheduler(timer_id);
        let timer_id = timer_id.to_string();
        let timer_id_for_event = timer_id.clone();
        let app_event_tx = self.app_event_tx.clone();
        let handle = tokio::spawn(async move {
            let now = Utc::now();
            let delay = due_at
                .signed_duration_since(now)
                .to_std()
                .unwrap_or(Duration::ZERO);
            tokio::time::sleep(delay).await;
            app_event_tx.send(AppEvent::TriggerLoopTimer {
                timer_id: timer_id_for_event,
                scheduled_for_unix_seconds: due_at.timestamp(),
            });
        });
        self.loop_timers.scheduler_tasks.insert(timer_id, handle);
    }

    fn stop_loop_timer_scheduler(&mut self, timer_id: &str) {
        if let Some(handle) = self.loop_timers.scheduler_tasks.remove(timer_id) {
            handle.abort();
        }
    }

    fn stop_loop_timer_run(&mut self, timer_id: &str) {
        let Some(run) = self.loop_timers.active_runs.remove(timer_id) else {
            return;
        };
        run.listener_handle.abort();
        let server = Arc::clone(&self.server);
        tokio::spawn(async move {
            let _ = run.thread.shutdown_and_wait().await;
            let _ = server.remove_thread(&run.thread_id).await;
        });
    }

    fn stop_all_loop_timer_tasks(&mut self) {
        for handle in self
            .loop_timers
            .scheduler_tasks
            .drain()
            .map(|(_, handle)| handle)
        {
            handle.abort();
        }
        let active_ids = self
            .loop_timers
            .active_runs
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for timer_id in active_ids {
            self.stop_loop_timer_run(&timer_id);
        }
    }

    fn persist_loop_timers(&self) -> std::io::Result<()> {
        let path = loop_timers_path(self.config.cwd.as_path());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = PersistedLoopTimersFile {
            timers: self.loop_timers.timers.values().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        fs::write(path, json)
    }

    fn record_thread_history_cell(&mut self, thread_id: ThreadId, cell: Arc<dyn HistoryCell>) {
        self.loop_timers
            .thread_history_cells
            .entry(thread_id)
            .or_default()
            .push(cell.clone());
    }
}

struct ParsedLoopSpec {
    schedule: LoopSchedule,
    prompt: String,
}

fn parse_loop_spec(spec: &str) -> Result<ParsedLoopSpec, String> {
    let tokens = spec.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 2 {
        return Err("expected a schedule followed by a prompt".to_string());
    }

    if let Some(seconds) = parse_interval_seconds(tokens[0]) {
        let prompt = spec[tokens[0].len()..].trim();
        if prompt.is_empty() {
            return Err("expected a prompt after the interval".to_string());
        }
        return Ok(ParsedLoopSpec {
            schedule: LoopSchedule::Interval {
                display: tokens[0].to_string(),
                seconds,
            },
            prompt: prompt.to_string(),
        });
    }

    for field_count in [7usize, 6, 5] {
        if tokens.len() <= field_count {
            continue;
        }
        let display = tokens[..field_count].join(" ");
        let normalized = normalize_cron_expression(&display, field_count);
        if Schedule::from_str(&normalized).is_ok() {
            let prompt = tokens[field_count..].join(" ");
            if !prompt.trim().is_empty() {
                return Ok(ParsedLoopSpec {
                    schedule: LoopSchedule::Cron {
                        display,
                        normalized,
                    },
                    prompt,
                });
            }
        }
    }

    Err(
        "could not parse the schedule; use `5m`-style intervals or a 5/6/7-field cron expression"
            .to_string(),
    )
}

fn parse_interval_seconds(token: &str) -> Option<u64> {
    let mut index = 0usize;
    let mut total = 0u64;
    let bytes = token.as_bytes();
    while index < bytes.len() {
        let digits_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if digits_start == index || index >= bytes.len() {
            return None;
        }
        let value = token[digits_start..index].parse::<u64>().ok()?;
        let multiplier = match bytes[index] as char {
            's' => 1,
            'm' => 60,
            'h' => 60 * 60,
            'd' => 60 * 60 * 24,
            _ => return None,
        };
        total = total.checked_add(value.checked_mul(multiplier)?)?;
        index += 1;
    }
    (total > 0).then_some(total)
}

fn normalize_cron_expression(expression: &str, field_count: usize) -> String {
    match field_count {
        5 => format!("0 {expression} *"),
        6 => format!("{expression} *"),
        _ => expression.to_string(),
    }
}

fn loop_timer_selection_item(timer: &PersistedLoopTimer) -> SelectionItem {
    let timer_id = timer.id.clone();
    let mut description_parts = vec![timer.schedule.display().to_string()];
    description_parts.push(timer.prompt.clone());
    if !timer.enabled {
        description_parts.push("disabled".to_string());
    }
    if let Some(last_completed_at) = timer.last_completed_at_unix_seconds {
        description_parts.push(format!(
            "last completed {}",
            format_timestamp(last_completed_at)
        ));
    }
    SelectionItem {
        name: timer.prompt.clone(),
        description: Some(description_parts.join(" · ")),
        is_disabled: false,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenLoopTimerActions {
                timer_id: timer_id.clone(),
            })
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}

fn loop_result_cell(message: &str, cwd: &Path) -> AgentMessageCell {
    let mut rendered = vec![Line::default()];
    append_markdown(message, /*width*/ None, Some(cwd), &mut rendered);
    AgentMessageCell::new(rendered, /*is_first_line*/ false)
}

async fn build_loop_initial_history(rollout_path: Option<&Path>) -> InitialHistory {
    let Some(rollout_path) = rollout_path else {
        return InitialHistory::New;
    };
    let Ok(history) = RolloutRecorder::get_rollout_history(rollout_path).await else {
        return InitialHistory::New;
    };
    let items = history.get_rollout_items();
    if items.is_empty() {
        return InitialHistory::New;
    }

    let session_meta = items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(_) => Some(item.clone()),
        _ => None,
    });
    let latest_turn_context_index = items
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, item)| matches!(item, RolloutItem::TurnContext(_)).then_some(index));
    let latest_turn_context = latest_turn_context_index.map(|index| items[index].clone());

    let mut used_tokens = 0usize;
    let mut selected_tail = Vec::new();
    for (index, item) in items.iter().enumerate().rev() {
        if matches!(item, RolloutItem::SessionMeta(_)) || Some(index) == latest_turn_context_index {
            continue;
        }
        let item_tokens = serde_json::to_string(item)
            .ok()
            .map(|text| text.len().saturating_add(3) / 4)
            .unwrap_or(0);
        if !selected_tail.is_empty()
            && used_tokens.saturating_add(item_tokens) > LOOP_CONTEXT_BUDGET_TOKENS
        {
            break;
        }
        used_tokens = used_tokens.saturating_add(item_tokens);
        selected_tail.push(item.clone());
    }
    selected_tail.reverse();

    let mut selected = Vec::new();
    if let Some(session_meta) = session_meta {
        selected.push(session_meta);
    }
    if let Some(turn_context) = latest_turn_context {
        selected.push(turn_context);
    }
    selected.extend(selected_tail);

    if selected.is_empty() {
        InitialHistory::New
    } else {
        InitialHistory::Forked(selected)
    }
}

fn load_loop_timers(cwd: &Path) -> std::io::Result<PersistedLoopTimersFile> {
    let path = loop_timers_path(cwd);
    if !path.exists() {
        return Ok(PersistedLoopTimersFile { timers: Vec::new() });
    }
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(std::io::Error::other)
}

fn loop_timers_path(cwd: &Path) -> PathBuf {
    cwd.join(".codex").join(LOOP_TIMER_FILE_NAME)
}

fn unix_seconds_to_utc(unix_seconds: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(unix_seconds, 0).single()
}

fn format_timestamp(unix_seconds: i64) -> String {
    unix_seconds_to_utc(unix_seconds)
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| unix_seconds.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::AgentNavigationState;
    use super::super::App;
    use super::super::BacktrackState;
    use super::super::KeyChordState;
    use super::super::WindowsSandboxState;
    use super::LoopSchedule;
    use super::parse_interval_seconds;
    use super::parse_loop_spec;
    use crate::bottom_pane::FeedbackAudience;
    use crate::chatwidget::tests::make_chatwidget_manual_with_sender;
    use crate::display_preferences::DisplayPreferences;
    use crate::file_search::FileSearchManager;
    use crate::history_cell::PlainHistoryCell;
    use codex_arg0::Arg0DispatchPaths;
    use codex_core::CodexAuth;
    use codex_core::config::Config;
    use codex_core::config::ConfigOverrides;
    use codex_core::config::types::TuiLoopCompletionMirrorMode;
    use codex_core::config_loader::CloudRequirementsLoader;
    use codex_core::config_loader::LoaderOverrides;
    use codex_otel::SessionTelemetry;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::SessionSource;
    use pretty_assertions::assert_eq;
    use ratatui::text::Line;
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn test_session_telemetry(config: &Config, model: &str) -> SessionTelemetry {
        let model_info = codex_core::test_support::construct_model_info_offline(model, config);
        SessionTelemetry::new(
            ThreadId::new(),
            model,
            model_info.slug.as_str(),
            None,
            None,
            None,
            "test_originator".to_string(),
            false,
            "test".to_string(),
            SessionSource::Cli,
        )
    }

    async fn make_test_app() -> App {
        let (chat_widget, app_event_tx, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
        let config = chat_widget.config_ref().clone();
        let server = Arc::new(
            codex_core::test_support::thread_manager_with_models_provider(
                CodexAuth::from_api_key("Test API Key"),
                config.model_provider.clone(),
            ),
        );
        let auth_manager = codex_core::test_support::auth_manager_from_auth(
            CodexAuth::from_api_key("Test API Key"),
        );
        let file_search = FileSearchManager::new(config.cwd.clone(), app_event_tx.clone());
        let model = codex_core::test_support::get_model_offline(config.model.as_deref());
        let session_telemetry = test_session_telemetry(&config, model.as_str());

        App {
            server,
            session_telemetry,
            app_event_tx,
            chat_widget,
            auth_manager,
            config,
            active_profile: None,
            cli_kv_overrides: Vec::new(),
            arg0_paths: Arg0DispatchPaths::default(),
            loader_overrides: LoaderOverrides::default(),
            cloud_requirements: CloudRequirementsLoader::default(),
            harness_overrides: ConfigOverrides::default(),
            runtime_approval_policy_override: None,
            runtime_sandbox_policy_override: None,
            display_preferences: DisplayPreferences::default(),
            file_search,
            transcript_cells: Vec::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            enhanced_keys_supported: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            status_line_invalid_items_warned: Arc::new(AtomicBool::new(false)),
            terminal_title_invalid_items_warned: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            key_chord: KeyChordState::default(),
            backtrack_render_pending: false,
            feedback: codex_feedback::CodexFeedback::new(),
            feedback_audience: FeedbackAudience::External,
            pending_update_action: None,
            suppress_shutdown_complete: false,
            pending_shutdown_exit_thread_id: None,
            windows_sandbox: WindowsSandboxState::default(),
            btw_session: None,
            loop_timers: super::LoopTimersState::default(),
            thread_event_channels: HashMap::new(),
            thread_event_listener_tasks: HashMap::new(),
            agent_navigation: AgentNavigationState::default(),
            active_thread_id: None,
            active_thread_rx: None,
            primary_thread_id: None,
            primary_session_configured: None,
            pending_primary_events: VecDeque::new(),
        }
    }

    #[test]
    fn parse_loop_spec_accepts_interval_prompt() {
        let parsed = parse_loop_spec("5m check status").expect("interval should parse");
        assert_eq!(
            parsed.schedule,
            LoopSchedule::Interval {
                display: "5m".to_string(),
                seconds: 300,
            }
        );
        assert_eq!(parsed.prompt, "check status");
    }

    #[test]
    fn parse_loop_spec_accepts_five_field_cron_prompt() {
        let parsed = parse_loop_spec("*/5 * * * * summarize").expect("cron should parse");
        assert_eq!(
            parsed.schedule,
            LoopSchedule::Cron {
                display: "*/5 * * * *".to_string(),
                normalized: "0 */5 * * * * *".to_string(),
            }
        );
        assert_eq!(parsed.prompt, "summarize");
    }

    #[test]
    fn parse_interval_seconds_accepts_compound_values() {
        assert_eq!(parse_interval_seconds("1h30m"), Some(5_400));
    }

    #[tokio::test]
    async fn finish_loop_timer_only_replays_prompt_and_latest_answer() {
        let mut app = make_test_app().await;
        let primary_thread_id = ThreadId::new();
        app.primary_thread_id = Some(primary_thread_id);
        app.active_thread_id = Some(primary_thread_id);
        app.loop_timers.thread_history_cells.insert(
            primary_thread_id,
            vec![Arc::new(PlainHistoryCell::new(vec![Line::from(
                "earlier hidden context",
            )]))],
        );

        let cells = app.finish_loop_timer(
            "timer-1".to_string(),
            "check status".to_string(),
            Ok("latest answer only".to_string()),
        );

        assert_eq!(cells.len(), 3);

        let rendered = cells
            .iter()
            .map(|cell| {
                cell.display_lines(80)
                    .into_iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>();
        assert!(
            rendered[0].contains("Loop timer-1 ran: check status"),
            "expected loop summary, got: {}",
            rendered[0]
        );
        assert!(
            rendered[1].contains("/loop check status"),
            "expected mirrored loop prompt, got: {}",
            rendered[1]
        );
        assert!(
            rendered[2].contains("latest answer only"),
            "expected final assistant message, got: {}",
            rendered[2]
        );

        let stored = app
            .loop_timers
            .thread_history_cells
            .get(&primary_thread_id)
            .expect("primary thread history should be recorded");
        assert_eq!(stored.len(), 3);
    }

    #[tokio::test]
    async fn finish_loop_timer_can_replay_only_latest_answer() {
        let mut app = make_test_app().await;
        app.config.tui_loop_completion_mirror_mode = TuiLoopCompletionMirrorMode::ResponseOnly;
        let primary_thread_id = ThreadId::new();
        app.primary_thread_id = Some(primary_thread_id);
        app.active_thread_id = Some(primary_thread_id);

        let cells = app.finish_loop_timer(
            "timer-1".to_string(),
            "check status".to_string(),
            Ok("latest answer only".to_string()),
        );

        assert_eq!(cells.len(), 2);

        let rendered = cells
            .iter()
            .map(|cell| {
                cell.display_lines(80)
                    .into_iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>();
        assert!(
            rendered[0].contains("Loop timer-1 ran: check status"),
            "expected loop summary, got: {}",
            rendered[0]
        );
        assert!(
            rendered[1].contains("latest answer only"),
            "expected final assistant message, got: {}",
            rendered[1]
        );
        assert!(
            !rendered[0].contains("/loop"),
            "did not expect loop prompt in loop summary mode: {}",
            rendered[0]
        );
        assert!(
            !rendered[1].contains("/loop"),
            "did not expect loop prompt in response-only mode: {}",
            rendered[1]
        );

        let stored = app
            .loop_timers
            .thread_history_cells
            .get(&primary_thread_id)
            .expect("primary thread history should be recorded");
        assert_eq!(stored.len(), 2);
    }
}
