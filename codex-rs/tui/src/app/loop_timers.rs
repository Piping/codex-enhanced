use super::App;
use super::loop_timer_command::LoopCommand;
use super::loop_timer_command::LoopDeliveryMode;
use super::loop_timer_command::LoopMode;
use super::loop_timer_command::LoopSchedule;
use super::loop_timer_command::parse_loop_command;
use super::loop_timer_command::parse_loop_schedule;
use crate::app_event::AppEvent;
use crate::app_event::LoopTimerTriggerSource;
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
use codex_core::content_items_to_text;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReadOnlyAccess;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use ratatui::text::Line;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
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
    #[serde(default)]
    mode: LoopMode,
    prompt: String,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    delivery_mode: Option<LoopDeliveryMode>,
    schedule: LoopSchedule,
    enabled: bool,
    #[serde(default)]
    rollout_path: Option<PathBuf>,
    created_at_unix_seconds: i64,
    last_scheduled_at_unix_seconds: Option<i64>,
    last_completed_at_unix_seconds: Option<i64>,
}

pub(crate) struct LoopTimerCompletion {
    pub(crate) cells: Vec<Arc<dyn HistoryCell>>,
    pub(crate) followup_user_message: Option<String>,
}

impl App {
    fn loop_timers_panel_params(&self, initial_selected_idx: Option<usize>) -> SelectionViewParams {
        let path = loop_timers_path(self.config.cwd.as_path());
        let subtitle = Some(format!(
            "{} timer(s) configured for {}.",
            self.loop_timers.timers.len(),
            self.config.cwd.display()
        ));

        let mut items = self
            .loop_timers
            .timers
            .values()
            .map(|timer| {
                loop_timer_selection_item(
                    timer,
                    self.loop_timers.active_runs.contains_key(&timer.id),
                )
            })
            .collect::<Vec<_>>();

        if items.is_empty() {
            items.push(SelectionItem {
                name: "No loop timers yet".to_string(),
                description: Some(
                    "Create one with `/loop 5m <prompt>` or `/loop <id> 30m <prompt>`.".to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            });
        }

        SelectionViewParams {
            view_id: Some(LOOP_TIMERS_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle,
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(path.display().to_string()),
            initial_selected_idx,
            items,
            ..Default::default()
        }
    }

    pub(crate) fn open_loop_timers_panel(&mut self) {
        self.ensure_loop_timers_loaded();

        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(LOOP_TIMERS_VIEW_ID);
        if !self.chat_widget.replace_selection_view_if_active(
            LOOP_TIMERS_VIEW_ID,
            self.loop_timers_panel_params(initial_selected_idx),
        ) {
            self.chat_widget
                .show_selection_view(self.loop_timers_panel_params(initial_selected_idx));
        }
    }

    fn refresh_loop_timers_panel_if_active(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(LOOP_TIMERS_VIEW_ID);
        let _ = self.chat_widget.replace_selection_view_if_active(
            LOOP_TIMERS_VIEW_ID,
            self.loop_timers_panel_params(initial_selected_idx),
        );
    }

    pub(crate) fn create_loop_timer(&mut self, spec: String) {
        self.ensure_loop_timers_loaded();

        let parsed = match parse_loop_command(spec.trim()) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to create `/loop`: {err}"));
                return;
            }
        };

        let now = Utc::now();
        let (timer_id, message) = match parsed {
            LoopCommand::Focus { id } => {
                if self.loop_timers.timers.contains_key(&id) {
                    self.open_loop_timer_actions(id);
                } else {
                    self.chat_widget.add_error_message(format!(
                        "Unknown loop `{id}`. Create it with `/loop {id} <time> <prompt>`."
                    ));
                }
                return;
            }
            LoopCommand::Create {
                id,
                schedule,
                prompt,
            } => {
                let prompt = prompt.trim().to_string();
                if prompt.is_empty() {
                    self.chat_widget.add_error_message(
                        "Failed to create `/loop`: expected a prompt.".to_string(),
                    );
                    return;
                }
                let mode = if id.is_some() {
                    LoopMode::Persistent
                } else {
                    LoopMode::OneShot
                };
                let timer_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let existing = self.loop_timers.timers.get(&timer_id).cloned();
                let timer = PersistedLoopTimer {
                    id: timer_id.clone(),
                    mode,
                    prompt: prompt.clone(),
                    action: existing.as_ref().and_then(|timer| timer.action.clone()),
                    delivery_mode: existing.as_ref().and_then(|timer| timer.delivery_mode),
                    schedule: schedule.clone(),
                    enabled: true,
                    rollout_path: existing
                        .as_ref()
                        .and_then(|timer| timer.rollout_path.clone()),
                    created_at_unix_seconds: existing
                        .as_ref()
                        .map_or(now.timestamp(), |timer| timer.created_at_unix_seconds),
                    last_scheduled_at_unix_seconds: None,
                    last_completed_at_unix_seconds: existing
                        .as_ref()
                        .and_then(|timer| timer.last_completed_at_unix_seconds),
                };
                self.loop_timers.timers.insert(timer.id.clone(), timer);
                let summary = match mode {
                    LoopMode::OneShot => {
                        format!(
                            "Created one-shot `/loop`: {} -> {prompt}",
                            schedule.display()
                        )
                    }
                    LoopMode::Persistent => {
                        let verb = if existing.is_some() {
                            "Updated"
                        } else {
                            "Created"
                        };
                        format!(
                            "{verb} persistent `/loop {timer_id}`: {} -> {prompt}",
                            schedule.display()
                        )
                    }
                };
                (timer_id, summary)
            }
        };
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to persist `/loop` timer: {err}"));
            self.loop_timers.timers.remove(&timer_id);
            return;
        }

        if let Some(timer) = self.loop_timers.timers.get(&timer_id).cloned()
            && let Some(due) = self.next_due_for_timer(&timer, now)
        {
            self.schedule_loop_timer(&timer_id, due);
        }
        self.chat_widget.add_info_message(message, /*hint*/ None);
        self.refresh_loop_timers_panel_if_active();
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
        let timer_id_for_run_now = timer.id.clone();
        let timer_id_for_edit_prompt = timer.id.clone();
        let timer_id_for_edit_schedule = timer.id.clone();
        let timer_id_for_edit_action = timer.id.clone();
        let timer_id_for_edit_delivery_mode = timer.id.clone();
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TIMER_ACTIONS_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!(
                "{} · {}",
                timer_descriptor(timer),
                timer.schedule.display()
            )),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "Run Now".to_string(),
                    description: Some("Trigger this loop immediately.".to_string()),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::TriggerLoopTimer {
                            timer_id: timer_id_for_run_now.clone(),
                            scheduled_for_unix_seconds: Utc::now().timestamp(),
                            source: LoopTimerTriggerSource::Manual,
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Edit Prompt".to_string(),
                    description: Some(
                        "Update the task this loop should stay aligned to.".to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerPrompt {
                            timer_id: timer_id_for_edit_prompt.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Edit Schedule".to_string(),
                    description: Some("Change the interval or cron expression.".to_string()),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerSchedule {
                            timer_id: timer_id_for_edit_schedule.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Edit Action".to_string(),
                    description: Some(
                        "Set the optional text appended in `As User Message + Action` mode."
                            .to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerAction {
                            timer_id: timer_id_for_edit_action.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Delivery Mode".to_string(),
                    description: Some(format!(
                        "Currently {}. Adjust how the loop reply feeds back into the main thread.",
                        effective_loop_delivery_mode(Some(timer)).short_label()
                    )),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerDeliveryMode {
                            timer_id: timer_id_for_edit_delivery_mode.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
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

    pub(crate) fn open_loop_timer_prompt_editor(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        self.chat_widget
            .open_loop_timer_prompt_editor(timer_id, timer.prompt.clone());
    }

    pub(crate) fn open_loop_timer_schedule_editor(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        self.chat_widget
            .open_loop_timer_schedule_editor(timer_id, timer.schedule.display().to_string());
    }

    pub(crate) fn open_loop_timer_action_editor(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        self.chat_widget
            .open_loop_timer_action_editor(timer_id, timer.action.clone().unwrap_or_default());
    }

    pub(crate) fn open_loop_timer_delivery_mode_menu(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };

        let current_mode = effective_loop_delivery_mode(Some(timer));
        let items = LoopDeliveryMode::USER_SELECTABLE
            .into_iter()
            .map(|mode| {
                let timer_id = timer_id.clone();
                SelectionItem {
                    name: mode.title().to_string(),
                    description: Some(mode.description().to_string()),
                    is_current: current_mode == mode,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SaveLoopTimerDeliveryMode {
                            timer_id: timer_id.clone(),
                            delivery_mode: (mode != LoopDeliveryMode::AssistantOnly)
                                .then_some(mode),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Delivery mode · {}", timer_descriptor(timer))),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new(move |tx| {
                tx.send(AppEvent::OpenLoopTimerActions {
                    timer_id: timer_id.clone(),
                })
            })),
            ..Default::default()
        });
    }

    pub(crate) fn save_loop_timer_prompt(&mut self, timer_id: String, prompt: String) {
        self.ensure_loop_timers_loaded();
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            self.chat_widget
                .add_error_message("Loop prompt cannot be empty.".to_string());
            return;
        }
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.prompt = prompt;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer prompt: {err}"));
        }
        self.open_loop_timer_actions(timer_id);
    }

    pub(crate) fn save_loop_timer_schedule(&mut self, timer_id: String, schedule: String) {
        self.ensure_loop_timers_loaded();
        let schedule = match parse_loop_schedule(schedule.trim()) {
            Ok(schedule) => schedule,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to update loop schedule: {err}"));
                return;
            }
        };
        let updated_timer = {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                self.chat_widget
                    .add_error_message("That loop timer no longer exists.".to_string());
                self.open_loop_timers_panel();
                return;
            };
            timer.schedule = schedule;
            timer.last_scheduled_at_unix_seconds = None;
            timer.clone()
        };
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer schedule: {err}"));
        }
        if updated_timer.enabled
            && let Some(due) = self.next_due_for_timer(&updated_timer, Utc::now())
        {
            self.schedule_loop_timer(&timer_id, due);
        }
        self.open_loop_timer_actions(timer_id);
    }

    pub(crate) fn save_loop_timer_action(&mut self, timer_id: String, action: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        let trimmed = action.trim().to_string();
        timer.action = (!trimmed.is_empty()).then_some(trimmed);
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer action: {err}"));
        }
        self.open_loop_timer_actions(timer_id);
    }

    pub(crate) fn save_loop_timer_delivery_mode(
        &mut self,
        timer_id: String,
        delivery_mode: Option<LoopDeliveryMode>,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.delivery_mode = delivery_mode;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer delivery mode: {err}"));
        }
        self.open_loop_timer_actions(timer_id);
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
        source: LoopTimerTriggerSource,
    ) -> Vec<Arc<dyn HistoryCell>> {
        self.ensure_loop_timers_loaded();
        let now = Utc::now();
        let timer = {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                return Vec::new();
            };
            if matches!(source, LoopTimerTriggerSource::Scheduled) && !timer.enabled {
                return Vec::new();
            }
            timer.last_scheduled_at_unix_seconds = Some(scheduled_for_unix_seconds);
            timer.clone()
        };
        let next_due = timer
            .enabled
            .then(|| self.next_due_for_timer(&timer, now))
            .flatten();
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer schedule: {err}"));
        }
        if let Some(next_due) = next_due {
            self.schedule_loop_timer(&timer_id, next_due);
        }

        if self.loop_timers.active_runs.contains_key(&timer_id) {
            self.sync_background_loop_status();
            self.refresh_loop_timers_panel_if_active();
            return Vec::new();
        }
        let prompt = timer.prompt.clone();
        let mut loop_config = self.config.clone();
        loop_config.ephemeral = matches!(timer.mode, LoopMode::OneShot);
        loop_config.include_apply_patch_tool = false;
        if let Err(err) = loop_config
            .permissions
            .approval_policy
            .set(AskForApproval::Never)
        {
            self.chat_widget
                .add_error_message(format!("Failed to configure `/loop` approvals: {err}"));
            return Vec::new();
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
            return Vec::new();
        }
        loop_config.developer_instructions =
            Some(match loop_config.developer_instructions.take() {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{existing}\n\n{LOOP_DEVELOPER_INSTRUCTIONS}")
                }
                _ => LOOP_DEVELOPER_INSTRUCTIONS.to_string(),
            });

        let primary_rollout_path = self
            .primary_session_configured
            .as_ref()
            .and_then(|event| event.rollout_path.as_deref());
        let recent_main_messages = load_recent_main_thread_messages(primary_rollout_path, 3).await;
        let loop_input = build_loop_run_input(&prompt, &recent_main_messages);

        let new_thread = match timer.mode {
            LoopMode::OneShot => {
                let initial_history = build_loop_initial_history(primary_rollout_path).await;
                self.server
                    .start_thread_with_history_and_source(
                        loop_config,
                        initial_history,
                        SessionSource::SubAgent(SubAgentSource::Other("loop".to_string())),
                    )
                    .await
            }
            LoopMode::Persistent => {
                let rollout_path = timer
                    .rollout_path
                    .as_ref()
                    .filter(|path| path.exists())
                    .cloned();
                match rollout_path {
                    Some(rollout_path) => {
                        self.server
                            .resume_thread_from_rollout(
                                loop_config,
                                rollout_path,
                                Arc::clone(&self.auth_manager),
                                /*parent_trace*/ None,
                            )
                            .await
                    }
                    None => {
                        let initial_history =
                            build_loop_initial_history(primary_rollout_path).await;
                        self.server
                            .start_thread_with_history_and_source(
                                loop_config,
                                initial_history,
                                SessionSource::SubAgent(SubAgentSource::Other("loop".to_string())),
                            )
                            .await
                    }
                }
            }
        };
        let new_thread = match new_thread {
            Ok(new_thread) => new_thread,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to start `/loop` execution: {err}"));
                return Vec::new();
            }
        };

        let thread_id = new_thread.thread_id;
        let thread = new_thread.thread;
        let rollout_path = new_thread.session_configured.rollout_path.clone();
        if matches!(timer.mode, LoopMode::Persistent)
            && let Some(timer) = self.loop_timers.timers.get_mut(&timer_id)
        {
            timer.rollout_path = rollout_path;
            if let Err(err) = self.persist_loop_timers() {
                self.chat_widget
                    .add_error_message(format!("Failed to persist `/loop` thread state: {err}"));
            }
        }
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
        self.sync_background_loop_status();

        if let Err(err) = thread
            .submit(Op::UserInput {
                items: vec![codex_protocol::user_input::UserInput::Text {
                    text: loop_input,
                    text_elements: Vec::new(),
                }],
                final_output_json_schema: None,
            })
            .await
        {
            self.chat_widget
                .add_error_message(format!("Failed to submit `/loop` prompt: {err}"));
            self.stop_loop_timer_run(&timer_id);
            self.sync_background_loop_status();
            self.refresh_loop_timers_panel_if_active();
            return Vec::new();
        }

        self.refresh_loop_timers_panel_if_active();
        self.record_loop_timer_started(&timer_id, &timer.prompt)
    }

    pub(crate) fn finish_loop_timer(
        &mut self,
        timer_id: String,
        prompt: String,
        result: Result<String, String>,
    ) -> LoopTimerCompletion {
        self.ensure_loop_timers_loaded();
        self.stop_loop_timer_run(&timer_id);
        self.sync_background_loop_status();
        let mut completed_timer = None;
        let mut remove_after_completion = false;
        if let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) {
            timer.last_completed_at_unix_seconds = Some(Utc::now().timestamp());
            remove_after_completion = matches!(timer.mode, LoopMode::OneShot);
            completed_timer = Some(timer.clone());
        }
        if remove_after_completion {
            self.loop_timers.timers.remove(&timer_id);
        }
        if completed_timer.is_some() || remove_after_completion {
            if let Err(err) = self.persist_loop_timers() {
                self.chat_widget
                    .add_error_message(format!("Failed to persist loop timer completion: {err}"));
            }
        }
        self.refresh_loop_timers_panel_if_active();

        match result {
            Ok(message) => {
                let Some(primary_thread_id) = self.primary_thread_id else {
                    return LoopTimerCompletion {
                        cells: Vec::new(),
                        followup_user_message: None,
                    };
                };
                let timer_summary = completed_timer
                    .as_ref()
                    .map(|timer| {
                        format!(
                            "Loop {} ({}) ran: {}",
                            loop_id_prefix(&timer.id),
                            timer.schedule.display(),
                            prompt_prefix(&prompt),
                        )
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "Loop {} ran: {}",
                            loop_id_prefix(&timer_id),
                            prompt_prefix(&prompt),
                        )
                    });
                let summary_cell: Arc<dyn HistoryCell> =
                    Arc::new(new_info_event(timer_summary, /*hint*/ None));
                let assistant_cell: Arc<dyn HistoryCell> =
                    Arc::new(loop_result_cell(&message, self.config.cwd.as_path()));
                let mut mirrored_cells = Vec::new();
                let delivery_mode = effective_loop_delivery_mode(completed_timer.as_ref());
                let followup_user_message = match delivery_mode {
                    LoopDeliveryMode::AssistantOnly => None,
                    LoopDeliveryMode::ResultAsUser => Some(message.clone()),
                    LoopDeliveryMode::AssistantThenActionUser => {
                        Some(build_loop_result_user_message_with_action(
                            &message,
                            completed_timer
                                .as_ref()
                                .and_then(|timer| timer.action.as_deref()),
                        ))
                    }
                };
                self.record_thread_history_cell(primary_thread_id, summary_cell.clone());
                mirrored_cells.push(summary_cell);
                if matches!(delivery_mode, LoopDeliveryMode::AssistantOnly) {
                    match self.config.tui_loop_completion_mirror_mode {
                        TuiLoopCompletionMirrorMode::PromptAndResponse => {
                            // Mirror only the scheduled prompt and the latest final answer back
                            // into the main thread. The hidden `/loop` execution history stays
                            // private.
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
                }
                if self.active_thread_id == Some(primary_thread_id) {
                    LoopTimerCompletion {
                        cells: mirrored_cells,
                        followup_user_message,
                    }
                } else {
                    LoopTimerCompletion {
                        cells: Vec::new(),
                        followup_user_message,
                    }
                }
            }
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("A `/loop` run failed: {err}"));
                LoopTimerCompletion {
                    cells: Vec::new(),
                    followup_user_message: None,
                }
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
        self.sync_background_loop_status();

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
        if matches!(timer.mode, LoopMode::OneShot) && timer.last_scheduled_at_unix_seconds.is_some()
        {
            return None;
        }
        match timer.last_scheduled_at_unix_seconds {
            Some(last_scheduled_at) => Some(timer.schedule.next_due_after(last_scheduled_at, now)),
            None => Some(timer.schedule.first_due_after_creation(now)),
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
                source: LoopTimerTriggerSource::Scheduled,
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
        self.sync_background_loop_status();
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

    fn record_loop_timer_started(
        &mut self,
        timer_id: &str,
        prompt: &str,
    ) -> Vec<Arc<dyn HistoryCell>> {
        let Some(primary_thread_id) = self.primary_thread_id else {
            return Vec::new();
        };

        let message = self
            .loop_timers
            .timers
            .get(timer_id)
            .map(|timer| {
                format!(
                    "Loop {} ({}) is running in background: {}",
                    loop_id_prefix(&timer.id),
                    timer.schedule.display(),
                    prompt_prefix(prompt),
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "Loop {} is running in background: {}",
                    loop_id_prefix(timer_id),
                    prompt_prefix(prompt),
                )
            });
        let cell: Arc<dyn HistoryCell> = Arc::new(new_info_event(message, /*hint*/ None));
        self.record_thread_history_cell(primary_thread_id, cell.clone());
        if self.active_thread_id == Some(primary_thread_id) {
            vec![cell]
        } else {
            Vec::new()
        }
    }

    fn sync_background_loop_status(&mut self) {
        let running_loops = self
            .loop_timers
            .active_runs
            .keys()
            .filter_map(|timer_id| {
                self.loop_timers.timers.get(timer_id).map(|timer| {
                    format!(
                        "{} ({}) · {}",
                        loop_item_name(timer),
                        timer.schedule.display(),
                        prompt_prefix(&timer.prompt),
                    )
                })
            })
            .collect::<Vec<_>>();
        self.chat_widget.sync_background_loop_status(running_loops);
    }
}

fn loop_timer_selection_item(timer: &PersistedLoopTimer, is_running: bool) -> SelectionItem {
    let timer_id = timer.id.clone();
    let mut description_parts = vec![
        timer_descriptor(timer).to_string(),
        timer.schedule.display().to_string(),
        prompt_prefix(&timer.prompt),
        effective_loop_delivery_mode(Some(timer))
            .short_label()
            .to_string(),
    ];
    if matches!(
        effective_loop_delivery_mode(Some(timer)),
        LoopDeliveryMode::AssistantThenActionUser
    ) && timer
        .action
        .as_ref()
        .is_some_and(|action| !action.trim().is_empty())
    {
        description_parts.push("has action".to_string());
    }
    if is_running {
        description_parts.push("running now".to_string());
    }
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
        name: loop_item_name(timer),
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

fn loop_item_name(timer: &PersistedLoopTimer) -> String {
    match timer.mode {
        LoopMode::OneShot => format!("one-shot {}", prompt_prefix(&timer.prompt)),
        LoopMode::Persistent => timer.id.clone(),
    }
}

fn timer_descriptor(timer: &PersistedLoopTimer) -> &'static str {
    match timer.mode {
        LoopMode::OneShot => "one-shot",
        LoopMode::Persistent => "persistent",
    }
}

fn loop_result_cell(message: &str, cwd: &Path) -> AgentMessageCell {
    let mut rendered = vec![Line::default()];
    append_markdown(message, /*width*/ None, Some(cwd), &mut rendered);
    AgentMessageCell::new(rendered, /*is_first_line*/ false)
}

fn effective_loop_delivery_mode(timer: Option<&PersistedLoopTimer>) -> LoopDeliveryMode {
    timer
        .and_then(|timer| timer.delivery_mode)
        .unwrap_or_default()
}

fn loop_id_prefix(id: &str) -> String {
    id.chars().take(8).collect()
}

fn prompt_prefix(prompt: &str) -> String {
    let prefix = prompt.chars().take(48).collect::<String>();
    if prompt.chars().count() > 48 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn build_loop_run_input(prompt: &str, recent_main_messages: &[String]) -> String {
    if recent_main_messages.is_empty() {
        return prompt.to_string();
    }
    let recent_messages = recent_main_messages.join("\n\n");
    format!("Recent main-thread messages:\n{recent_messages}\n\nOriginal loop prompt:\n{prompt}")
}

fn build_loop_result_user_message_with_action(result: &str, action: Option<&str>) -> String {
    let Some(action) = action.map(str::trim).filter(|action| !action.is_empty()) else {
        return result.to_string();
    };
    format!("{result}\n\nAdditional action:\n{action}")
}

async fn load_recent_main_thread_messages(
    rollout_path: Option<&Path>,
    limit: usize,
) -> Vec<String> {
    let Some(rollout_path) = rollout_path else {
        return Vec::new();
    };
    let Ok(history) = RolloutRecorder::get_rollout_history(rollout_path).await else {
        return Vec::new();
    };
    let mut messages = history
        .get_rollout_items()
        .iter()
        .filter_map(|item| match item {
            RolloutItem::ResponseItem(ResponseItem::Message { role, content, .. })
                if role == "user" || role == "assistant" =>
            {
                content_items_to_text(content)
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
                    .map(|text| format!("{role}: {text}"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if messages.len() > limit {
        messages.drain(..messages.len().saturating_sub(limit));
    }
    messages
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
    use super::LoopMode;
    use super::LoopSchedule;
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
    use insta::assert_snapshot;
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

        let completion = app.finish_loop_timer(
            "timer-1".to_string(),
            "check status".to_string(),
            Ok("latest answer only".to_string()),
        );
        let cells = completion.cells;

        assert_eq!(cells.len(), 3);
        assert_eq!(completion.followup_user_message, None);

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

        let completion = app.finish_loop_timer(
            "timer-1".to_string(),
            "check status".to_string(),
            Ok("latest answer only".to_string()),
        );
        let cells = completion.cells;

        assert_eq!(cells.len(), 2);
        assert_eq!(completion.followup_user_message, None);

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

    #[tokio::test]
    async fn record_loop_timer_started_replays_background_notice() {
        let mut app = make_test_app().await;
        let primary_thread_id = ThreadId::new();
        app.primary_thread_id = Some(primary_thread_id);
        app.active_thread_id = Some(primary_thread_id);
        app.loop_timers.timers.insert(
            "timer-1".to_string(),
            super::PersistedLoopTimer {
                id: "timer-1".to_string(),
                mode: LoopMode::Persistent,
                prompt: "check status".to_string(),
                action: None,
                schedule: LoopSchedule::Interval {
                    display: "5m".to_string(),
                    seconds: 300,
                },
                enabled: true,
                rollout_path: None,
                created_at_unix_seconds: 0,
                last_scheduled_at_unix_seconds: None,
                last_completed_at_unix_seconds: None,
            },
        );

        let cells = app.record_loop_timer_started("timer-1", "check status");
        assert_eq!(cells.len(), 1);

        let rendered = cells[0]
            .display_lines(80)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_snapshot!("loop_timer_background_notice", rendered);
    }
}
