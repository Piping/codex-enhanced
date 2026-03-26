use super::App;
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
use chrono::Utc;
use codex_core::CodexThread;
use codex_core::RolloutRecorder;
use codex_core::config::types::TuiLoopCompletionMirrorMode;
use codex_core::content_items_to_text;
use codex_loop::LoopCommand;
use codex_loop::LoopDeliveryMode;
use codex_loop::LoopMode;
use codex_loop::PersistedLoopExecutionSettings;
use codex_loop::PersistedLoopTimer;
use codex_loop::PersistedLoopTimersFile;
use codex_loop::apply_loop_execution_settings;
use codex_loop::build_loop_result_user_message_with_action;
use codex_loop::build_loop_run_input;
use codex_loop::cwd_editor_text;
use codex_loop::effective_loop_delivery_mode;
use codex_loop::format_timestamp;
use codex_loop::load_loop_timers;
use codex_loop::loop_execution_summary;
use codex_loop::loop_id_prefix;
use codex_loop::loop_item_name;
use codex_loop::loop_timers_path;
use codex_loop::next_due_for_timer;
use codex_loop::parse_loop_command;
use codex_loop::parse_loop_cwd;
use codex_loop::parse_loop_schedule;
use codex_loop::parse_loop_writable_roots;
use codex_loop::prompt_prefix;
use codex_loop::timer_descriptor;
use codex_loop::writable_roots_editor_text;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use ratatui::text::Line;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

const LOOP_TIMERS_VIEW_ID: &str = "fork-loop-timers-panel";
const LOOP_CREATE_VIEW_ID: &str = "fork-loop-create-panel";
const LOOP_TIMER_ACTIONS_VIEW_ID: &str = "fork-loop-timer-actions-panel";
const LOOP_EXECUTION_VIEW_ID: &str = "fork-loop-execution-panel";
const LOOP_CONTEXT_BUDGET_TOKENS: usize = 2_000;

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

        items.insert(
            0,
            SelectionItem {
                name: "Create Loop Agent".to_string(),
                description: Some(
                    "Create a one-shot or persistent `/loop` entry from a guided form.".to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenCreateLoopTimerMenu))],
                dismiss_on_select: true,
                ..Default::default()
            },
        );

        if self.loop_timers.timers.is_empty() {
            items.push(SelectionItem {
                name: "No loop timers yet".to_string(),
                description: Some(
                    "Use Create Loop Agent or `/loop 5m <prompt>` to add one.".to_string(),
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

    pub(crate) fn open_create_loop_timer_menu(&mut self) {
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_CREATE_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some("Create loop agent".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "One-Shot Loop".to_string(),
                    description: Some(
                        "Schedule a loop that keeps firing, but uses a fresh hidden thread each run."
                            .to_string(),
                    ),
                    actions: vec![Box::new(|tx| tx.send(AppEvent::OpenCreateOneShotLoopPrompt))],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Persistent Loop".to_string(),
                    description: Some(
                        "Schedule a loop with a stable id and a private long-lived hidden context."
                            .to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::OpenCreatePersistentLoopPrompt)
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenLoopTimersPanel))),
            ..Default::default()
        });
    }

    fn loop_execution_panel_params(
        &self,
        timer_id: &str,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let Some(timer) = self.loop_timers.timers.get(timer_id) else {
            return SelectionViewParams::default();
        };
        let items = vec![
            SelectionItem {
                name: "Working Directory".to_string(),
                description: Some(loop_execution_summary(&timer.execution, self.config.cwd.as_path())),
                actions: vec![Box::new({
                    let timer_id = timer_id.to_string();
                    move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerCwd {
                            timer_id: timer_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Use Session Working Directory".to_string(),
                description: Some(
                    "Clear the per-loop cwd override and inherit the main thread working directory."
                        .to_string(),
                ),
                is_disabled: timer.execution.cwd.is_none(),
                actions: vec![Box::new({
                    let timer_id = timer_id.to_string();
                    move |tx| {
                        tx.send(AppEvent::ResetLoopTimerCwd {
                            timer_id: timer_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Writable Directories".to_string(),
                description: Some(
                    "Restrict loop file writes to specific directories. Leave empty to inherit the session scope."
                        .to_string(),
                ),
                actions: vec![Box::new({
                    let timer_id = timer_id.to_string();
                    move |tx| {
                        tx.send(AppEvent::OpenEditLoopWritableRoots {
                            timer_id: timer_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Use Session Writable Scope".to_string(),
                description: Some(
                    "Clear the per-loop writable-directory override and inherit the main thread sandbox scope."
                        .to_string(),
                ),
                is_disabled: timer.execution.writable_roots.is_empty(),
                actions: vec![Box::new({
                    let timer_id = timer_id.to_string();
                    move |tx| {
                        tx.send(AppEvent::ResetLoopWritableRoots {
                            timer_id: timer_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        SelectionViewParams {
            view_id: Some(LOOP_EXECUTION_VIEW_ID),
            title: Some("Loop Execution".to_string()),
            subtitle: Some(format!("Execution settings · {}", timer_descriptor(timer))),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            on_cancel: Some(Box::new({
                let timer_id = timer_id.to_string();
                move |tx| {
                    tx.send(AppEvent::OpenLoopTimerActions {
                        timer_id: timer_id.clone(),
                    })
                }
            })),
            ..Default::default()
        }
    }

    pub(crate) fn open_loop_execution_panel(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();

        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(LOOP_EXECUTION_VIEW_ID);
        if !self.chat_widget.replace_selection_view_if_active(
            LOOP_EXECUTION_VIEW_ID,
            self.loop_execution_panel_params(&timer_id, initial_selected_idx),
        ) {
            self.chat_widget.show_selection_view(
                self.loop_execution_panel_params(&timer_id, initial_selected_idx),
            );
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
                    execution: existing
                        .as_ref()
                        .map_or_else(PersistedLoopExecutionSettings::default, |timer| {
                            timer.execution.clone()
                        }),
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
            && let Some(due) = next_due_for_timer(&timer, now)
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
        let timer_id_for_execution_settings = timer.id.clone();
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
                SelectionItem {
                    name: "Execution Settings".to_string(),
                    description: Some(loop_execution_summary(
                        &timer.execution,
                        self.config.cwd.as_path(),
                    )),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenLoopExecutionPanel {
                            timer_id: timer_id_for_execution_settings.clone(),
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

    pub(crate) fn open_loop_writable_roots_editor(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        self.chat_widget.open_loop_writable_roots_editor(
            timer_id,
            writable_roots_editor_text(&timer.execution),
        );
    }

    pub(crate) fn open_loop_timer_cwd_editor(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        self.chat_widget.open_loop_timer_cwd_editor(
            timer_id,
            cwd_editor_text(&timer.execution, self.config.cwd.as_path()),
        );
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
            && let Some(due) = next_due_for_timer(&updated_timer, Utc::now())
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

    pub(crate) fn save_loop_writable_roots(&mut self, timer_id: String, writable_roots: String) {
        self.ensure_loop_timers_loaded();
        let writable_roots =
            match parse_loop_writable_roots(&writable_roots, self.config.cwd.as_path()) {
                Ok(writable_roots) => writable_roots,
                Err(err) => {
                    self.chat_widget.add_error_message(format!(
                        "Failed to update `/loop` writable directories: {err}"
                    ));
                    return;
                }
            };
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.execution.writable_roots = writable_roots;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget.add_error_message(format!(
                "Failed to persist `/loop` execution settings: {err}"
            ));
        }
        self.open_loop_execution_panel(timer_id);
    }

    pub(crate) fn save_loop_timer_cwd(&mut self, timer_id: String, cwd: String) {
        self.ensure_loop_timers_loaded();
        let cwd = match parse_loop_cwd(&cwd, self.config.cwd.as_path()) {
            Ok(cwd) => cwd,
            Err(err) => {
                self.chat_widget.add_error_message(format!(
                    "Failed to update `/loop` working directory: {err}"
                ));
                return;
            }
        };
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.execution.cwd = Some(cwd);
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget.add_error_message(format!(
                "Failed to persist `/loop` execution settings: {err}"
            ));
        }
        self.open_loop_execution_panel(timer_id);
    }

    pub(crate) fn reset_loop_timer_cwd(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.execution.cwd = None;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget.add_error_message(format!(
                "Failed to persist `/loop` execution settings: {err}"
            ));
        }
        self.open_loop_execution_panel(timer_id);
    }

    pub(crate) fn reset_loop_writable_roots(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.execution.writable_roots.clear();
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget.add_error_message(format!(
                "Failed to persist `/loop` execution settings: {err}"
            ));
        }
        self.open_loop_execution_panel(timer_id);
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
            && let Some(due) = next_due_for_timer(&timer, Utc::now())
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
            .then(|| next_due_for_timer(&timer, now))
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
        let developer_instructions = match apply_loop_execution_settings(
            &mut loop_config,
            &timer.execution,
            self.config.cwd.as_path(),
        ) {
            Ok(instructions) => instructions,
            Err(err) => {
                self.chat_widget.add_error_message(err);
                return Vec::new();
            }
        };
        loop_config.developer_instructions =
            Some(match loop_config.developer_instructions.take() {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{existing}\n\n{developer_instructions}")
                }
                _ => developer_instructions,
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
        Vec::new()
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
        if let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) {
            timer.last_completed_at_unix_seconds = Some(Utc::now().timestamp());
            completed_timer = Some(timer.clone());
        }
        if completed_timer.is_some()
            && let Err(err) = self.persist_loop_timers()
        {
            self.chat_widget
                .add_error_message(format!("Failed to persist loop timer completion: {err}"));
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
                    LoopDeliveryMode::ResultAsUser => Some(message),
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
        self.loop_timers.workspace_cwd = Some(self.config.cwd.to_path_buf());
        self.loop_timers.thread_history_cells.clear();
        self.sync_background_loop_status();

        let loaded = load_loop_timers(self.config.cwd.as_path()).unwrap_or_default();
        let loaded = loaded
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
            .filter_map(|timer| next_due_for_timer(timer, now).map(|due| (timer.id.clone(), due)))
            .collect::<Vec<_>>();
        for (timer_id, due) in due_entries {
            self.schedule_loop_timer(&timer_id, due);
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

    pub(crate) fn stop_active_loop_runs(&mut self) -> usize {
        let active_ids = self
            .loop_timers
            .active_runs
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let stopped_count = active_ids.len();
        for timer_id in active_ids {
            self.stop_loop_timer_run(&timer_id);
        }
        self.sync_background_loop_status();
        self.refresh_loop_timers_panel_if_active();
        stopped_count
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

fn loop_result_cell(message: &str, cwd: &Path) -> AgentMessageCell {
    let mut rendered = vec![Line::default()];
    append_markdown(message, /*width*/ None, Some(cwd), &mut rendered);
    AgentMessageCell::new(rendered, /*is_first_line*/ false)
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

#[cfg(test)]
mod tests {
    use super::super::AgentNavigationState;
    use super::super::App;
    use super::super::BacktrackState;
    use super::super::KeyChordState;
    use super::super::WindowsSandboxState;
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
}
