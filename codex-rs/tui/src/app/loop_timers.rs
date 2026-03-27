use super::App;
use super::loop_create::LoopCreateDraft;
use crate::app_event::AppEvent;
use crate::app_event::LoopTimerTriggerSource;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell::AgentMessageCell;
use crate::history_cell::HistoryCell;
use crate::markdown::append_markdown;
use chrono::DateTime;
use chrono::Utc;
use codex_core::CodexThread;
use codex_core::RolloutRecorder;
use codex_core::content_items_to_text;
use codex_loop::LoopCommand;
use codex_loop::LoopContextMode;
use codex_loop::LoopMode;
use codex_loop::LoopResponseMode;
use codex_loop::LoopSecurityMode;
use codex_loop::LoopTriggerBinding;
use codex_loop::LoopTriggerKind;
use codex_loop::LoopTriggerPhase;
use codex_loop::PersistedLoopExecutionSettings;
use codex_loop::PersistedLoopTimer;
use codex_loop::PersistedLoopTimersFile;
use codex_loop::PersistedLoopTriggerQueuesFile;
use codex_loop::build_loop_result_user_message_with_action;
use codex_loop::cwd_editor_text;
use codex_loop::effective_timer_schedule;
use codex_loop::format_timestamp;
use codex_loop::load_loop_timers;
use codex_loop::load_loop_trigger_queues;
use codex_loop::loop_execution_summary;
use codex_loop::loop_item_name;
use codex_loop::loop_timers_path;
use codex_loop::loop_trigger_queues_path;
use codex_loop::move_trigger_queue_entry;
use codex_loop::next_due_for_timer;
use codex_loop::next_trigger_binding_id;
use codex_loop::parse_loop_command;
use codex_loop::parse_loop_cwd;
use codex_loop::parse_loop_schedule;
use codex_loop::parse_loop_writable_roots;
use codex_loop::prompt_prefix;
use codex_loop::queue_entries_for_phase;
use codex_loop::sync_trigger_queues_with_timers;
use codex_loop::timer_descriptor;
use codex_loop::trigger_bindings;
use codex_loop::writable_roots_editor_text;
use codex_loop_runtime::build_loop_phase_input;
use codex_loop_runtime::build_loop_runtime_overrides;
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
const LOOP_TRIGGER_BINDINGS_VIEW_ID: &str = "fork-loop-trigger-bindings-panel";
const LOOP_TRIGGER_CREATE_VIEW_ID: &str = "fork-loop-trigger-create-panel";
const LOOP_TRIGGER_QUEUE_VIEW_ID: &str = "fork-loop-trigger-queue-panel";
const LOOP_TRIGGER_PHASE_VIEW_ID: &str = "fork-loop-trigger-phase-panel";
const LOOP_TRIGGER_ACTIONS_VIEW_ID: &str = "fork-loop-trigger-actions-panel";
const LOOP_CONTEXT_BUDGET_TOKENS: usize = 2_000;

#[derive(Default)]
pub(crate) struct LoopTimersState {
    workspace_cwd: Option<PathBuf>,
    timers: BTreeMap<String, PersistedLoopTimer>,
    trigger_queues: PersistedLoopTriggerQueuesFile,
    pub(crate) create_draft: Option<LoopCreateDraft>,
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

struct StartedLoopThread {
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    rollout_path: Option<PathBuf>,
}

struct LoopHookOutput {
    loop_id: String,
    response_mode: LoopResponseMode,
    message: Option<String>,
    action: Option<String>,
}

impl App {
    fn loop_trigger_queues_panel_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        SelectionViewParams {
            view_id: Some(LOOP_TRIGGER_QUEUE_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some("Trigger Queue".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(
                loop_trigger_queues_path(self.config.cwd.as_path())
                    .display()
                    .to_string(),
            ),
            initial_selected_idx,
            items: LoopTriggerPhase::USER_SELECTABLE
                .into_iter()
                .map(|phase| SelectionItem {
                    name: phase.title().to_string(),
                    description: Some(phase.description().to_string()),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenLoopTriggerQueuePhase { phase })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                })
                .collect(),
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenLoopTimersPanel))),
            ..Default::default()
        }
    }

    pub(crate) fn open_loop_trigger_queues_panel(&mut self) {
        self.ensure_loop_timers_loaded();
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(LOOP_TRIGGER_QUEUE_VIEW_ID);
        if !self.chat_widget.replace_selection_view_if_active(
            LOOP_TRIGGER_QUEUE_VIEW_ID,
            self.loop_trigger_queues_panel_params(initial_selected_idx),
        ) {
            self.chat_widget
                .show_selection_view(self.loop_trigger_queues_panel_params(initial_selected_idx));
        }
    }

    pub(crate) fn open_loop_trigger_queue_phase_panel(&mut self, phase: LoopTriggerPhase) {
        self.ensure_loop_timers_loaded();
        let entries = queue_entries_for_phase(&self.loop_timers.trigger_queues, phase)
            .iter()
            .filter_map(|entry| {
                let timer = self.loop_timers.timers.get(&entry.loop_id)?;
                let binding = trigger_bindings(timer)
                    .into_iter()
                    .find(|binding| binding.id == entry.binding_id)?;
                Some(SelectionItem {
                    name: format!("{} / {}", loop_item_name(timer), binding.selection_name()),
                    description: Some(prompt_prefix(&timer.prompt)),
                    actions: vec![Box::new({
                        let loop_id = entry.loop_id.clone();
                        let binding_id = entry.binding_id.clone();
                        move |tx| {
                            tx.send(AppEvent::OpenLoopTriggerQueueEntryActions {
                                phase,
                                loop_id: loop_id.clone(),
                                binding_id: binding_id.clone(),
                            })
                        }
                    })],
                    dismiss_on_select: true,
                    is_disabled: !binding.enabled || !timer.enabled,
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();

        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TRIGGER_PHASE_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Trigger Queue · {}", phase.title())),
            footer_hint: Some(standard_popup_hint_line()),
            items: if entries.is_empty() {
                vec![SelectionItem {
                    name: "No triggers in this queue".to_string(),
                    description: Some(
                        "Add triggers inside a loop, then reorder them here across loops."
                            .to_string(),
                    ),
                    is_disabled: true,
                    ..Default::default()
                }]
            } else {
                entries
            },
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenLoopTriggerQueuesPanel))),
            ..Default::default()
        });
    }

    pub(crate) fn open_loop_trigger_queue_entry_actions(
        &mut self,
        phase: LoopTriggerPhase,
        loop_id: String,
        binding_id: String,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&loop_id) else {
            self.open_loop_trigger_queue_phase_panel(phase);
            return;
        };
        let Some(binding) = trigger_bindings(timer)
            .into_iter()
            .find(|binding| binding.id == binding_id)
        else {
            self.open_loop_trigger_queue_phase_panel(phase);
            return;
        };

        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TRIGGER_ACTIONS_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!(
                "Trigger Queue · {} / {}",
                loop_item_name(timer),
                binding.selection_name()
            )),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "Move Up".to_string(),
                    description: Some("Run this trigger earlier within the queue.".to_string()),
                    actions: vec![Box::new({
                        let loop_id = loop_id.clone();
                        let binding_id = binding_id.clone();
                        move |tx| {
                            tx.send(AppEvent::MoveLoopTriggerQueueEntry {
                                phase,
                                loop_id: loop_id.clone(),
                                binding_id: binding_id.clone(),
                                move_up: true,
                            })
                        }
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Move Down".to_string(),
                    description: Some("Run this trigger later within the queue.".to_string()),
                    actions: vec![Box::new({
                        let loop_id = loop_id.clone();
                        move |tx| {
                            tx.send(AppEvent::MoveLoopTriggerQueueEntry {
                                phase,
                                loop_id: loop_id.clone(),
                                binding_id: binding_id.clone(),
                                move_up: false,
                            })
                        }
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Open Loop".to_string(),
                    description: Some(
                        "Jump back to this loop's configuration and triggers.".to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenLoopTimerActions {
                            timer_id: loop_id.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            on_cancel: Some(Box::new(move |tx| {
                tx.send(AppEvent::OpenLoopTriggerQueuePhase { phase })
            })),
            ..Default::default()
        });
    }

    pub(crate) fn move_loop_trigger_queue_entry(
        &mut self,
        phase: LoopTriggerPhase,
        loop_id: String,
        binding_id: String,
        move_up: bool,
    ) {
        self.ensure_loop_timers_loaded();
        let moved = move_trigger_queue_entry(
            &mut self.loop_timers.trigger_queues,
            phase,
            &loop_id,
            &binding_id,
            if move_up {
                codex_loop::QueueMoveDirection::Up
            } else {
                codex_loop::QueueMoveDirection::Down
            },
        );
        if moved && let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update trigger queue: {err}"));
        }
        self.open_loop_trigger_queue_phase_panel(phase);
    }

    fn loop_timers_panel_params(&self, initial_selected_idx: Option<usize>) -> SelectionViewParams {
        let path = loop_timers_path(self.config.cwd.as_path());
        let subtitle = Some(format!(
            "{} loop agent(s) configured for {}.",
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
                name: "Trigger Queue".to_string(),
                description: Some(
                    "Reorder cross-loop execution for timer, before-turn, and after-turn triggers."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenLoopTriggerQueuesPanel))],
                dismiss_on_select: true,
                ..Default::default()
            },
        );

        items.insert(
            0,
            SelectionItem {
                name: "Create Loop Agent".to_string(),
                description: Some(
                    "Create an embed, ephemeral, or persistent `/loop` from a guided form."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenCreateLoopTimerMenu))],
                dismiss_on_select: true,
                ..Default::default()
            },
        );

        if self.loop_timers.timers.is_empty() {
            items.push(SelectionItem {
                name: "No loop agents yet".to_string(),
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
                    name: "Embed".to_string(),
                    description: Some(
                        "Run directly in the main-thread execution path.".to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::StartCreateLoopDraft {
                            context_mode: LoopContextMode::Embed,
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Ephemeral".to_string(),
                    description: Some(
                        "Run in a hidden thread that is discarded after each trigger.".to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::StartCreateLoopDraft {
                            context_mode: LoopContextMode::Ephemeral,
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Persistent".to_string(),
                    description: Some(
                        "Run in a hidden thread with a stable id and a retained rollout."
                            .to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::StartCreateLoopDraft {
                            context_mode: LoopContextMode::Persistent,
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
            } => match self.upsert_loop_timer(
                id,
                prompt,
                LoopTriggerKind::Timer { schedule },
                /*draft*/ None,
                /*message_prefix*/ None,
                now,
            ) {
                Ok(result) => result,
                Err(err) => {
                    self.chat_widget
                        .add_error_message(format!("Failed to create `/loop`: {err}"));
                    return;
                }
            },
        };
        sync_trigger_queues_with_timers(
            &mut self.loop_timers.trigger_queues,
            &self.loop_timers.timers,
        );
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

    pub(crate) fn finalize_loop_create_draft(&mut self) {
        self.ensure_loop_timers_loaded();
        let now = Utc::now();
        let Some(draft) = self.loop_timers.create_draft.take() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let Some(trigger_kind) = draft.trigger_kind.clone() else {
            self.chat_widget
                .add_error_message("Loop creation is missing an initial trigger.".to_string());
            return;
        };
        match self.upsert_loop_timer(
            draft.id.clone(),
            draft.prompt.clone().unwrap_or_default(),
            trigger_kind,
            Some(draft),
            Some("Created loop agent".to_string()),
            now,
        ) {
            Ok((timer_id, message)) => {
                sync_trigger_queues_with_timers(
                    &mut self.loop_timers.trigger_queues,
                    &self.loop_timers.timers,
                );
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
                self.open_loop_timer_actions(timer_id);
            }
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to create `/loop`: {err}"));
            }
        }
    }

    fn upsert_loop_timer(
        &mut self,
        id: Option<String>,
        prompt: String,
        trigger_kind: LoopTriggerKind,
        draft: Option<LoopCreateDraft>,
        message_prefix: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<(String, String), String> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err("expected a prompt.".to_string());
        }
        let mode = if id.is_some() {
            LoopMode::Persistent
        } else {
            LoopMode::OneShot
        };
        let default_context_mode = match mode {
            LoopMode::OneShot => LoopContextMode::Ephemeral,
            LoopMode::Persistent => LoopContextMode::Persistent,
        };
        let timer_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing = self.loop_timers.timers.get(&timer_id).cloned();
        let trigger_binding = LoopTriggerBinding {
            id: "trigger-1".to_string(),
            enabled: true,
            kind: trigger_kind.clone(),
        };
        let mut execution = existing
            .as_ref()
            .map_or_else(PersistedLoopExecutionSettings::default, |timer| {
                timer.execution.clone()
            });
        let mut security_mode = existing
            .as_ref()
            .map_or(LoopSecurityMode::default(), |timer| timer.security_mode);
        let mut context_mode = existing
            .as_ref()
            .map_or(default_context_mode, |timer| timer.context_mode);
        let mut response_mode = existing
            .as_ref()
            .map_or(LoopResponseMode::default(), |timer| timer.response_mode);
        if let Some(draft) = draft.as_ref() {
            context_mode = draft.context_mode;
            response_mode = draft.response_mode;
            security_mode = draft.security_mode;
            if security_mode == LoopSecurityMode::SpecifiedDirectory {
                let writable_roots_input =
                    draft.writable_roots_input.as_deref().ok_or_else(|| {
                        "specified_directory requires writable directories.".to_string()
                    })?;
                let writable_roots =
                    parse_loop_writable_roots(writable_roots_input, self.config.cwd.as_path())
                        .map_err(|err| {
                            format!(
                                "specified_directory requires valid writable directories: {err}"
                            )
                        })?;
                if writable_roots.is_empty() {
                    return Err(
                        "specified_directory requires at least one writable directory.".to_string(),
                    );
                }
                execution.writable_roots = writable_roots;
            }
        }
        let schedule = match &trigger_kind {
            LoopTriggerKind::Timer { schedule } => schedule.clone(),
            LoopTriggerKind::BeforeTurn | LoopTriggerKind::AfterTurn => {
                codex_loop::LoopSchedule::Interval {
                    display: "1h".to_string(),
                    seconds: 60 * 60,
                }
            }
        };
        let timer = PersistedLoopTimer {
            id: timer_id.clone(),
            mode,
            prompt: prompt.clone(),
            action: existing.as_ref().and_then(|timer| timer.action.clone()),
            context_mode,
            response_mode,
            security_mode,
            execution,
            schedule,
            trigger_bindings: vec![trigger_binding],
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
        let trigger_summary = match &trigger_kind {
            LoopTriggerKind::Timer { schedule } => schedule.display().to_string(),
            LoopTriggerKind::BeforeTurn => "before turn".to_string(),
            LoopTriggerKind::AfterTurn => "after turn".to_string(),
        };
        let summary = match mode {
            LoopMode::OneShot => {
                if let Some(prefix) = message_prefix.as_deref() {
                    format!("{prefix}: {trigger_summary} -> {prompt}")
                } else {
                    format!(
                        "Created {} `/loop`: {trigger_summary} -> {prompt}",
                        context_mode.title().to_lowercase()
                    )
                }
            }
            LoopMode::Persistent => {
                let verb = if existing.is_some() {
                    "Updated"
                } else {
                    "Created"
                };
                format!(
                    "{verb} {} `/loop {timer_id}`: {trigger_summary} -> {prompt}",
                    context_mode.title().to_lowercase()
                )
            }
        };
        Ok((timer_id, summary))
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
        let timer_id_for_edit_action = timer.id.clone();
        let timer_id_for_context_mode = timer.id.clone();
        let timer_id_for_response_mode = timer.id.clone();
        let timer_id_for_security_mode = timer.id.clone();
        let timer_id_for_execution_settings = timer.id.clone();
        let timer_id_for_triggers = timer.id.clone();
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TIMER_ACTIONS_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!(
                "{} · {}",
                timer_descriptor(timer),
                effective_timer_schedule(timer)
                    .map(|schedule| schedule.display().to_string())
                    .unwrap_or_else(|| "no timer trigger".to_string())
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
                    name: "Triggers".to_string(),
                    description: Some(
                        "Add, remove, or edit trigger bindings for this loop.".to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenLoopTimerTriggers {
                            timer_id: timer_id_for_triggers.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Edit Action".to_string(),
                    description: Some(
                        "Set optional text appended when this loop emits a user message."
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
                    name: "Context Mode".to_string(),
                    description: Some(format!(
                        "Currently {}. Adjust whether this loop runs embedded, ephemeral, or persistent.",
                        timer.context_mode.title()
                    )),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerContextMode {
                            timer_id: timer_id_for_context_mode.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Response Mode".to_string(),
                    description: Some(format!(
                        "Currently {}. Adjust how this loop feeds back into the main thread.",
                        timer.response_mode.title()
                    )),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerResponseMode {
                            timer_id: timer_id_for_response_mode.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Security Mode".to_string(),
                    description: Some(format!(
                        "Currently {}. Adjust whether writes inherit the parent thread or are restricted to configured directories.",
                        timer.security_mode.title()
                    )),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenEditLoopTimerSecurityMode {
                            timer_id: timer_id_for_security_mode.clone(),
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

    pub(crate) fn open_loop_timer_triggers_panel(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };

        let mut items = trigger_bindings(timer)
            .into_iter()
            .map(|binding| SelectionItem {
                name: binding.selection_name(),
                description: Some(match &binding.kind {
                    LoopTriggerKind::Timer { schedule } => {
                        format!("Timer trigger · {}", schedule.display())
                    }
                    LoopTriggerKind::BeforeTurn => {
                        "Runs before a main-thread user turn is submitted.".to_string()
                    }
                    LoopTriggerKind::AfterTurn => {
                        "Runs after the assistant final response completes.".to_string()
                    }
                }),
                is_disabled: !binding.enabled,
                actions: vec![Box::new({
                    let timer_id = timer_id.clone();
                    let binding_id = binding.id.clone();
                    move |tx| {
                        tx.send(AppEvent::OpenLoopTriggerBindingActions {
                            timer_id: timer_id.clone(),
                            binding_id: binding_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            })
            .collect::<Vec<_>>();

        let timer_id_for_add_trigger = timer_id.clone();
        let timer_id_for_cancel = timer_id.clone();
        items.insert(
            0,
            SelectionItem {
                name: "Add Trigger".to_string(),
                description: Some(
                    "Attach a timer, before-turn, or after-turn trigger to this loop.".to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenCreateLoopTriggerMenu {
                        timer_id: timer_id_for_add_trigger.clone(),
                    })
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        );

        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TRIGGER_BINDINGS_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Triggers · {}", loop_item_name(timer))),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new(move |tx| {
                tx.send(AppEvent::OpenLoopTimerActions {
                    timer_id: timer_id_for_cancel.clone(),
                })
            })),
            ..Default::default()
        });
    }

    pub(crate) fn open_create_loop_trigger_menu(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let timer_id_for_timer_trigger = timer_id.clone();
        let timer_id_for_before_turn = timer_id.clone();
        let timer_id_for_after_turn = timer_id.clone();
        let timer_id_for_cancel = timer_id.clone();
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TRIGGER_CREATE_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Add Trigger · {timer_id}")),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "Timer Trigger".to_string(),
                    description: Some("Run this loop on an interval or cron schedule.".to_string()),
                    actions: vec![Box::new({
                        move |tx| {
                            tx.send(AppEvent::OpenCreateLoopTimerTriggerSchedule {
                                timer_id: timer_id_for_timer_trigger.clone(),
                            })
                        }
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Before Turn".to_string(),
                    description: Some(
                        "Run this loop before the next main-thread user turn is submitted."
                            .to_string(),
                    ),
                    actions: vec![Box::new({
                        move |tx| {
                            tx.send(AppEvent::AddLoopBeforeTurnTrigger {
                                timer_id: timer_id_for_before_turn.clone(),
                            })
                        }
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "After Turn".to_string(),
                    description: Some(
                        "Run this loop after the assistant final response completes.".to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::AddLoopAfterTurnTrigger {
                            timer_id: timer_id_for_after_turn.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            on_cancel: Some(Box::new(move |tx| {
                tx.send(AppEvent::OpenLoopTimerTriggers {
                    timer_id: timer_id_for_cancel.clone(),
                })
            })),
            ..Default::default()
        });
    }

    pub(crate) fn open_new_loop_timer_trigger_schedule_editor(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.open_loop_timers_panel();
            return;
        };
        let initial = effective_timer_schedule(timer)
            .map(|schedule| schedule.display().to_string())
            .unwrap_or_default();
        self.chat_widget
            .open_new_loop_trigger_schedule_editor(timer_id, initial);
    }

    pub(crate) fn open_loop_trigger_binding_schedule_editor(
        &mut self,
        timer_id: String,
        binding_id: String,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.open_loop_timers_panel();
            return;
        };
        let Some(binding) = trigger_bindings(timer)
            .into_iter()
            .find(|binding| binding.id == binding_id)
        else {
            self.open_loop_timer_triggers_panel(timer_id);
            return;
        };
        let LoopTriggerKind::Timer { schedule } = binding.kind else {
            self.open_loop_trigger_binding_actions(timer_id, binding_id);
            return;
        };
        self.chat_widget.open_loop_trigger_schedule_editor(
            timer_id,
            binding_id,
            schedule.display().to_string(),
        );
    }

    pub(crate) fn add_loop_trigger(&mut self, timer_id: String, kind: LoopTriggerKind) {
        self.ensure_loop_timers_loaded();
        let updated = {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                self.chat_widget
                    .add_error_message("That loop no longer exists.".to_string());
                self.open_loop_timers_panel();
                return;
            };
            if trigger_bindings(timer)
                .iter()
                .any(|binding| binding.kind == kind)
            {
                self.chat_widget
                    .add_error_message("That trigger already exists for this loop.".to_string());
                self.open_loop_timer_triggers_panel(timer_id);
                return;
            }
            timer.trigger_bindings.push(LoopTriggerBinding {
                id: next_trigger_binding_id(&trigger_bindings(timer)),
                enabled: true,
                kind,
            });
            timer.clone()
        };
        self.normalize_loop_after_trigger_change(&updated.id);
        self.open_loop_timer_triggers_panel(timer_id);
    }

    pub(crate) fn save_new_loop_timer_trigger_schedule(
        &mut self,
        timer_id: String,
        schedule: String,
    ) {
        let schedule = match parse_loop_schedule(schedule.trim()) {
            Ok(schedule) => schedule,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to add loop timer trigger: {err}"));
                return;
            }
        };
        self.add_loop_trigger(timer_id.clone(), LoopTriggerKind::Timer { schedule });
        self.open_loop_timer_triggers_panel(timer_id);
    }

    pub(crate) fn open_loop_trigger_binding_actions(
        &mut self,
        timer_id: String,
        binding_id: String,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.open_loop_timers_panel();
            return;
        };
        let Some(binding) = trigger_bindings(timer)
            .into_iter()
            .find(|binding| binding.id == binding_id)
        else {
            self.open_loop_timer_triggers_panel(timer_id);
            return;
        };
        let timer_id_for_delete = timer_id.clone();
        let timer_id_for_cancel = timer_id.clone();
        let mut items = Vec::new();
        if let LoopTriggerKind::Timer { schedule } = &binding.kind {
            items.push(SelectionItem {
                name: "Edit Schedule".to_string(),
                description: Some(format!("Current schedule: {}", schedule.display())),
                actions: vec![Box::new({
                    let timer_id = timer_id.clone();
                    let binding_id = binding.id.clone();
                    move |tx| {
                        tx.send(AppEvent::OpenEditLoopTriggerBindingSchedule {
                            timer_id: timer_id.clone(),
                            binding_id: binding_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
        items.push(if binding.enabled {
            SelectionItem {
                name: "Disable".to_string(),
                description: Some("Keep the trigger binding but stop using it.".to_string()),
                actions: vec![Box::new({
                    let timer_id = timer_id.clone();
                    let binding_id = binding.id.clone();
                    move |tx| {
                        tx.send(AppEvent::DisableLoopTriggerBinding {
                            timer_id: timer_id.clone(),
                            binding_id: binding_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        } else {
            SelectionItem {
                name: "Enable".to_string(),
                description: Some("Use this trigger binding again.".to_string()),
                actions: vec![Box::new({
                    let timer_id = timer_id.clone();
                    let binding_id = binding.id.clone();
                    move |tx| {
                        tx.send(AppEvent::EnableLoopTriggerBinding {
                            timer_id: timer_id.clone(),
                            binding_id: binding_id.clone(),
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        });
        items.push(SelectionItem {
            name: "Delete".to_string(),
            description: Some("Remove this trigger from the loop.".to_string()),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::DeleteLoopTriggerBinding {
                    timer_id: timer_id_for_delete.clone(),
                    binding_id: binding_id.clone(),
                })
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_TRIGGER_ACTIONS_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Trigger · {}", binding.selection_name())),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new(move |tx| {
                tx.send(AppEvent::OpenLoopTimerTriggers {
                    timer_id: timer_id_for_cancel.clone(),
                })
            })),
            ..Default::default()
        });
    }

    pub(crate) fn save_loop_trigger_binding_schedule(
        &mut self,
        timer_id: String,
        binding_id: String,
        schedule: String,
    ) {
        self.ensure_loop_timers_loaded();
        let schedule = match parse_loop_schedule(schedule.trim()) {
            Ok(schedule) => schedule,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to update loop trigger: {err}"));
                return;
            }
        };
        {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                self.open_loop_timers_panel();
                return;
            };
            let Some(binding) = timer
                .trigger_bindings
                .iter_mut()
                .find(|binding| binding.id == binding_id)
            else {
                self.open_loop_timer_triggers_panel(timer_id.clone());
                return;
            };
            binding.kind = LoopTriggerKind::Timer {
                schedule: schedule.clone(),
            };
            timer.schedule = schedule;
        }
        self.normalize_loop_after_trigger_change(&timer_id);
        self.open_loop_timer_triggers_panel(timer_id);
    }

    pub(crate) fn set_loop_trigger_binding_enabled(
        &mut self,
        timer_id: String,
        binding_id: String,
        enabled: bool,
    ) {
        self.ensure_loop_timers_loaded();
        {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                self.open_loop_timers_panel();
                return;
            };
            let Some(binding) = timer
                .trigger_bindings
                .iter_mut()
                .find(|binding| binding.id == binding_id)
            else {
                self.open_loop_timer_triggers_panel(timer_id.clone());
                return;
            };
            binding.enabled = enabled;
        }
        self.normalize_loop_after_trigger_change(&timer_id);
        self.open_loop_timer_triggers_panel(timer_id);
    }

    pub(crate) fn delete_loop_trigger_binding(&mut self, timer_id: String, binding_id: String) {
        self.ensure_loop_timers_loaded();
        {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                self.open_loop_timers_panel();
                return;
            };
            timer
                .trigger_bindings
                .retain(|binding| binding.id != binding_id);
        }
        self.normalize_loop_after_trigger_change(&timer_id);
        self.open_loop_timer_triggers_panel(timer_id);
    }

    fn normalize_loop_after_trigger_change(&mut self, timer_id: &str) {
        if let Some(timer) = self.loop_timers.timers.get_mut(timer_id)
            && let Some(schedule) = effective_timer_schedule(timer)
        {
            timer.schedule = schedule;
        }
        sync_trigger_queues_with_timers(
            &mut self.loop_timers.trigger_queues,
            &self.loop_timers.timers,
        );
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to persist loop trigger changes: {err}"));
        }
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

    pub(crate) fn open_loop_timer_context_mode_menu(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };

        let current_mode = timer.context_mode;
        let selectable_modes = match timer.mode {
            LoopMode::OneShot => [LoopContextMode::Embed, LoopContextMode::Ephemeral]
                .into_iter()
                .collect::<Vec<_>>(),
            LoopMode::Persistent => LoopContextMode::USER_SELECTABLE.into_iter().collect(),
        };
        let items = selectable_modes
            .into_iter()
            .map(|mode| {
                let timer_id = timer_id.clone();
                SelectionItem {
                    name: mode.title().to_string(),
                    description: Some(match mode {
                        LoopContextMode::Embed => {
                            "Run against the main-thread context without preserving private loop state.".to_string()
                        }
                        LoopContextMode::Ephemeral => {
                            "Run in a hidden short-lived thread that is discarded after each execution.".to_string()
                        }
                        LoopContextMode::Persistent => {
                            "Run in a hidden thread that keeps its own rollout and accumulates private state.".to_string()
                        }
                    }),
                    is_current: current_mode == mode,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SaveLoopTimerContextMode {
                            timer_id: timer_id.clone(),
                            context_mode: mode,
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Context mode · {}", timer_descriptor(timer))),
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

    pub(crate) fn open_loop_timer_response_mode_menu(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };

        let current_mode = timer.response_mode;
        let items = LoopResponseMode::USER_SELECTABLE
            .into_iter()
            .map(|mode| {
                let timer_id = timer_id.clone();
                SelectionItem {
                    name: mode.title().to_string(),
                    description: Some(mode.description().to_string()),
                    is_current: current_mode == mode,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SaveLoopTimerResponseMode {
                            timer_id: timer_id.clone(),
                            response_mode: mode,
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Response mode · {}", timer_descriptor(timer))),
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

    pub(crate) fn open_loop_timer_security_mode_menu(&mut self, timer_id: String) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };

        let current_mode = timer.security_mode;
        let items = LoopSecurityMode::USER_SELECTABLE
            .into_iter()
            .map(|mode| {
                let timer_id = timer_id.clone();
                SelectionItem {
                    name: mode.title().to_string(),
                    description: Some(match mode {
                        LoopSecurityMode::Inherited => {
                            "Use the parent thread's normal tool and filesystem permissions.".to_string()
                        }
                        LoopSecurityMode::SpecifiedDirectory => {
                            "Constrain writes to the configured writable directories while keeping the parent thread's other permissions.".to_string()
                        }
                    }),
                    is_current: current_mode == mode,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SaveLoopTimerSecurityMode {
                            timer_id: timer_id.clone(),
                            security_mode: mode,
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some("Loop Manager".to_string()),
            subtitle: Some(format!("Security mode · {}", timer_descriptor(timer))),
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

    pub(crate) fn save_loop_timer_context_mode(
        &mut self,
        timer_id: String,
        context_mode: LoopContextMode,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.context_mode = context_mode;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer context mode: {err}"));
        }
        self.open_loop_timer_actions(timer_id);
    }

    pub(crate) fn save_loop_timer_response_mode(
        &mut self,
        timer_id: String,
        response_mode: LoopResponseMode,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.response_mode = response_mode;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer response mode: {err}"));
        }
        self.open_loop_timer_actions(timer_id);
    }

    pub(crate) fn save_loop_timer_security_mode(
        &mut self,
        timer_id: String,
        security_mode: LoopSecurityMode,
    ) {
        self.ensure_loop_timers_loaded();
        let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
            self.chat_widget
                .add_error_message("That loop timer no longer exists.".to_string());
            self.open_loop_timers_panel();
            return;
        };
        timer.security_mode = security_mode;
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to update loop timer security mode: {err}"));
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
        sync_trigger_queues_with_timers(
            &mut self.loop_timers.trigger_queues,
            &self.loop_timers.timers,
        );
        if let Err(err) = self.persist_loop_timers() {
            self.chat_widget
                .add_error_message(format!("Failed to delete loop timer: {err}"));
        }
        self.open_loop_timers_panel();
    }

    pub(crate) async fn augment_primary_user_turn_with_before_turn_loops(
        &mut self,
        op: Op,
    ) -> (Op, Vec<Arc<dyn HistoryCell>>) {
        let Some(primary_thread_id) = self.primary_thread_id else {
            return (op, Vec::new());
        };
        if self.active_thread_id != Some(primary_thread_id) {
            return (op, Vec::new());
        }
        let Op::UserTurn {
            mut items,
            cwd,
            approval_policy,
            approvals_reviewer,
            sandbox_policy,
            model,
            effort,
            summary,
            service_tier,
            final_output_json_schema,
            collaboration_mode,
            personality,
        } = op
        else {
            return (op, Vec::new());
        };
        let current_user_turn = user_text_from_inputs(&items);
        if current_user_turn.trim().is_empty() {
            return (
                Op::UserTurn {
                    items,
                    cwd,
                    approval_policy,
                    approvals_reviewer,
                    sandbox_policy,
                    model,
                    effort,
                    summary,
                    service_tier,
                    final_output_json_schema,
                    collaboration_mode,
                    personality,
                },
                Vec::new(),
            );
        }

        let hook_outputs = self
            .run_loop_trigger_phase(
                LoopTriggerPhase::BeforeTurn,
                Some(current_user_turn.as_str()),
                /*last_assistant_message*/ None,
            )
            .await;
        let mut cells = Vec::new();
        for output in hook_outputs {
            match output.response_mode {
                LoopResponseMode::Assistant => {
                    if let Some(message) = output.message {
                        let rendered = format_loop_main_thread_message(&output.loop_id, &message);
                        let cell: Arc<dyn HistoryCell> =
                            Arc::new(loop_result_cell(&rendered, self.config.cwd.as_path()));
                        self.record_thread_history_cell(primary_thread_id, cell.clone());
                        cells.push(cell);
                    }
                }
                LoopResponseMode::User => {
                    if let Some(message) = output.message {
                        let rendered = format_loop_main_thread_message(
                            &output.loop_id,
                            &build_loop_result_user_message_with_action(
                                &message,
                                output.action.as_deref(),
                            ),
                        );
                        items.push(codex_protocol::user_input::UserInput::Text {
                            text: rendered.clone(),
                            text_elements: Vec::new(),
                        });
                    }
                }
            }
        }

        (
            Op::UserTurn {
                items,
                cwd,
                approval_policy,
                approvals_reviewer,
                sandbox_policy,
                model,
                effort,
                summary,
                service_tier,
                final_output_json_schema,
                collaboration_mode,
                personality,
            },
            cells,
        )
    }

    pub(crate) async fn handle_primary_thread_turn_complete_for_loops(
        &mut self,
        last_agent_message: Option<String>,
    ) {
        if self.primary_loop_generated_turn_in_flight {
            self.primary_loop_generated_turn_in_flight = false;
            return;
        }
        let Some(primary_thread_id) = self.primary_thread_id else {
            return;
        };
        let hook_outputs = self
            .run_loop_trigger_phase(
                LoopTriggerPhase::AfterTurn,
                /*current_user_turn*/ None,
                last_agent_message.as_deref(),
            )
            .await;
        for output in hook_outputs {
            match output.response_mode {
                LoopResponseMode::Assistant => {
                    if let Some(message) = output.message {
                        let rendered = format_loop_main_thread_message(&output.loop_id, &message);
                        let cell: Arc<dyn HistoryCell> =
                            Arc::new(loop_result_cell(&rendered, self.config.cwd.as_path()));
                        self.record_thread_history_cell(primary_thread_id, cell.clone());
                        if self.active_thread_id == Some(primary_thread_id) {
                            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                                loop_result_cell(&rendered, self.config.cwd.as_path()),
                            )));
                        }
                    }
                }
                LoopResponseMode::User => {
                    if let Some(message) = output.message {
                        let rendered = format_loop_main_thread_message(
                            &output.loop_id,
                            &build_loop_result_user_message_with_action(
                                &message,
                                output.action.as_deref(),
                            ),
                        );
                        self.submit_loop_user_message_to_primary(rendered).await;
                    }
                }
            }
        }
    }

    pub(crate) fn note_primary_thread_error_for_loops(&mut self) {
        self.primary_loop_generated_turn_in_flight = false;
    }

    pub(crate) async fn submit_loop_user_message_to_primary(&mut self, message: String) {
        let Some(primary_thread_id) = self.primary_thread_id else {
            return;
        };
        let trimmed = message.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let Ok(thread) = self.server.get_thread(primary_thread_id).await else {
            self.chat_widget.add_error_message(
                "Failed to find the main thread for loop follow-up.".to_string(),
            );
            return;
        };
        let config_snapshot = thread.config_snapshot().await;
        self.primary_loop_generated_turn_in_flight = true;
        self.submit_op_to_thread(
            primary_thread_id,
            Op::UserTurn {
                items: vec![codex_protocol::user_input::UserInput::Text {
                    text: trimmed,
                    text_elements: Vec::new(),
                }],
                cwd: config_snapshot.cwd,
                approval_policy: config_snapshot.approval_policy,
                approvals_reviewer: Some(config_snapshot.approvals_reviewer),
                sandbox_policy: config_snapshot.sandbox_policy,
                model: config_snapshot.model,
                effort: config_snapshot.reasoning_effort,
                summary: None,
                service_tier: config_snapshot.service_tier.map(Some),
                final_output_json_schema: None,
                collaboration_mode: None,
                personality: self.config.personality,
            },
        )
        .await;
    }

    async fn run_loop_trigger_phase(
        &mut self,
        phase: LoopTriggerPhase,
        current_user_turn: Option<&str>,
        last_assistant_message: Option<&str>,
    ) -> Vec<LoopHookOutput> {
        self.ensure_loop_timers_loaded();
        let queue_entries =
            queue_entries_for_phase(&self.loop_timers.trigger_queues, phase).to_vec();
        let mut outputs = Vec::new();
        for entry in queue_entries {
            let Some(timer) = self.loop_timers.timers.get(&entry.loop_id).cloned() else {
                continue;
            };
            if !timer.enabled {
                continue;
            }
            let Some(binding) = trigger_bindings(&timer)
                .into_iter()
                .find(|binding| binding.id == entry.binding_id)
            else {
                continue;
            };
            if !binding.enabled || binding.kind.phase() != phase {
                continue;
            }
            match self
                .run_inline_loop_binding(&timer, current_user_turn, last_assistant_message)
                .await
            {
                Ok(message) => outputs.push(LoopHookOutput {
                    loop_id: timer.id.clone(),
                    response_mode: timer.response_mode,
                    message,
                    action: timer.action.clone(),
                }),
                Err(err) => self.chat_widget.add_error_message(format!(
                    "Loop `{}` failed during {}: {err}",
                    timer.id,
                    phase.title()
                )),
            }
        }
        outputs
    }

    async fn run_inline_loop_binding(
        &mut self,
        timer: &PersistedLoopTimer,
        current_user_turn: Option<&str>,
        last_assistant_message: Option<&str>,
    ) -> Result<Option<String>, String> {
        if self.loop_timers.active_runs.contains_key(&timer.id) {
            return Ok(None);
        }
        let started = self.start_loop_thread(timer).await?;
        let recent_main_messages = load_recent_main_thread_messages(
            self.primary_session_configured
                .as_ref()
                .and_then(|event| event.rollout_path.as_deref()),
            /*limit*/ 3,
        )
        .await;
        let loop_input = build_loop_phase_input(
            timer.context_mode,
            &timer.prompt,
            &recent_main_messages,
            current_user_turn,
            last_assistant_message,
        );
        let result =
            run_loop_thread_until_completion(Arc::clone(&started.thread), loop_input).await;
        if matches!(timer.context_mode, LoopContextMode::Persistent)
            && let Some(timer_state) = self.loop_timers.timers.get_mut(&timer.id)
        {
            timer_state.rollout_path = started.rollout_path;
            timer_state.last_completed_at_unix_seconds = Some(Utc::now().timestamp());
            if let Err(err) = self.persist_loop_timers() {
                self.chat_widget.add_error_message(format!(
                    "Failed to persist loop state for `{}`: {err}",
                    timer.id
                ));
            }
        }
        let _ = started.thread.shutdown_and_wait().await;
        let _ = self.server.remove_thread(&started.thread_id).await;
        result
    }

    async fn start_loop_thread(
        &mut self,
        timer: &PersistedLoopTimer,
    ) -> Result<StartedLoopThread, String> {
        if matches!(timer.mode, LoopMode::OneShot)
            && matches!(timer.context_mode, LoopContextMode::Persistent)
        {
            return Err("Only persistent loops can use persistent context mode.".to_string());
        }

        let mut loop_config = self.config.clone();
        loop_config.ephemeral = !matches!(timer.context_mode, LoopContextMode::Persistent);
        let runtime_overrides = build_loop_runtime_overrides(
            timer.security_mode,
            &timer.execution,
            self.config.cwd.as_path(),
            loop_config
                .permissions
                .sandbox_policy
                .get()
                .has_full_network_access(),
        )?;
        if let Some(cwd) = runtime_overrides.cwd {
            loop_config.cwd = cwd;
        }
        if let Some(sandbox_policy) = runtime_overrides.sandbox_policy {
            loop_config
                .permissions
                .sandbox_policy
                .set(sandbox_policy)
                .map_err(|err| format!("Failed to configure `/loop` sandbox policy: {err}"))?;
        }
        loop_config.developer_instructions =
            Some(match loop_config.developer_instructions.take() {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{existing}\n\n{}", runtime_overrides.developer_instructions)
                }
                _ => runtime_overrides.developer_instructions,
            });

        let primary_rollout_path = self
            .primary_session_configured
            .as_ref()
            .and_then(|event| event.rollout_path.as_deref());
        let started = if matches!(timer.context_mode, LoopContextMode::Persistent) {
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
                    let initial_history = build_loop_initial_history(primary_rollout_path).await;
                    self.server
                        .start_thread_with_history_and_source(
                            loop_config,
                            initial_history,
                            SessionSource::SubAgent(SubAgentSource::Other("loop".to_string())),
                        )
                        .await
                }
            }
        } else {
            let initial_history = match timer.context_mode {
                LoopContextMode::Embed => build_loop_initial_history(primary_rollout_path).await,
                LoopContextMode::Ephemeral => InitialHistory::New,
                LoopContextMode::Persistent => unreachable!(),
            };
            self.server
                .start_thread_with_history_and_source(
                    loop_config,
                    initial_history,
                    SessionSource::SubAgent(SubAgentSource::Other("loop".to_string())),
                )
                .await
        }
        .map_err(|err| format!("Failed to start `/loop` execution: {err}"))?;

        Ok(StartedLoopThread {
            thread_id: started.thread_id,
            thread: started.thread,
            rollout_path: started.session_configured.rollout_path,
        })
    }

    pub(crate) async fn trigger_loop_timer(
        &mut self,
        timer_id: String,
        scheduled_for_unix_seconds: i64,
        source: LoopTimerTriggerSource,
    ) -> Vec<Arc<dyn HistoryCell>> {
        if matches!(source, LoopTimerTriggerSource::Scheduled) {
            self.trigger_due_timer_phase(scheduled_for_unix_seconds)
                .await;
            return Vec::new();
        }
        self.trigger_loop_timer_now(timer_id, scheduled_for_unix_seconds)
            .await
    }

    async fn trigger_loop_timer_now(
        &mut self,
        timer_id: String,
        scheduled_for_unix_seconds: i64,
    ) -> Vec<Arc<dyn HistoryCell>> {
        self.ensure_loop_timers_loaded();
        let now = Utc::now();
        let timer = {
            let Some(timer) = self.loop_timers.timers.get_mut(&timer_id) else {
                return Vec::new();
            };
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
        let started = match self.start_loop_thread(&timer).await {
            Ok(started) => started,
            Err(err) => {
                self.chat_widget.add_error_message(err);
                return Vec::new();
            }
        };
        if matches!(timer.context_mode, LoopContextMode::Persistent)
            && let Some(timer) = self.loop_timers.timers.get_mut(&timer_id)
        {
            timer.rollout_path = started.rollout_path.clone();
            if let Err(err) = self.persist_loop_timers() {
                self.chat_widget
                    .add_error_message(format!("Failed to persist `/loop` thread state: {err}"));
            }
        }
        let recent_main_messages = load_recent_main_thread_messages(
            self.primary_session_configured
                .as_ref()
                .and_then(|event| event.rollout_path.as_deref()),
            /*limit*/ 3,
        )
        .await;
        let loop_input = build_loop_phase_input(
            timer.context_mode,
            &prompt,
            &recent_main_messages,
            /*current_user_turn*/ None,
            /*last_assistant_message*/ None,
        );
        let app_event_tx = self.app_event_tx.clone();
        let thread_id = started.thread_id;
        let thread = Arc::clone(&started.thread);
        let listener_thread = Arc::clone(&started.thread);
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
                                .map(|text| text.trim().to_string())
                                .filter(|text| !text.is_empty());
                            app_event_tx.send(AppEvent::LoopTimerCompleted {
                                timer_id: timer_id_for_event.clone(),
                                prompt: prompt_for_event.clone(),
                                result: Ok(result),
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
                            app_event_tx.send(AppEvent::LoopTimerCompleted {
                                timer_id: timer_id_for_event.clone(),
                                prompt: prompt_for_event.clone(),
                                result: Ok(last_agent_message
                                    .map(|text| text.trim().to_string())
                                    .filter(|text| !text.is_empty())),
                            });
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
                thread: Arc::clone(&started.thread),
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

    async fn trigger_due_timer_phase(&mut self, scheduled_for_unix_seconds: i64) {
        self.ensure_loop_timers_loaded();
        let now = Utc::now();
        let due_loop_ids =
            queue_entries_for_phase(&self.loop_timers.trigger_queues, LoopTriggerPhase::Timer)
                .iter()
                .filter_map(|entry| {
                    let timer = self.loop_timers.timers.get(&entry.loop_id)?;
                    (timer.enabled
                        && timer.last_scheduled_at_unix_seconds.unwrap_or_default()
                            < scheduled_for_unix_seconds)
                        .then_some(timer)
                })
                .filter_map(|timer| {
                    next_due_for_timer(timer, now)
                        .filter(|due| due.timestamp() <= scheduled_for_unix_seconds)
                        .map(|_| timer.id.clone())
                })
                .collect::<Vec<_>>();
        for loop_id in due_loop_ids {
            let _ = self
                .trigger_loop_timer_now(loop_id, scheduled_for_unix_seconds)
                .await;
        }
    }

    pub(crate) fn finish_loop_timer(
        &mut self,
        timer_id: String,
        _prompt: String,
        result: Result<Option<String>, String>,
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
                let mut mirrored_cells = Vec::new();
                let loop_result = message.and_then(|message| {
                    let trimmed = message.trim().to_string();
                    (!trimmed.is_empty()).then_some(trimmed)
                });
                let response_mode = completed_timer
                    .as_ref()
                    .map_or(LoopResponseMode::default(), |timer| timer.response_mode);
                let followup_user_message = matches!(response_mode, LoopResponseMode::User)
                    .then(|| {
                        loop_result.as_ref().map(|message| {
                            build_loop_result_user_message_with_action(
                                message,
                                completed_timer
                                    .as_ref()
                                    .and_then(|timer| timer.action.as_deref()),
                            )
                        })
                    })
                    .flatten();
                if matches!(response_mode, LoopResponseMode::Assistant)
                    && let Some(message) = loop_result.as_deref()
                {
                    let assistant_cell: Arc<dyn HistoryCell> =
                        Arc::new(loop_result_cell(message, self.config.cwd.as_path()));
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
        self.loop_timers.trigger_queues =
            load_loop_trigger_queues(self.config.cwd.as_path()).unwrap_or_default();
        sync_trigger_queues_with_timers(
            &mut self.loop_timers.trigger_queues,
            &self.loop_timers.timers,
        );

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
        let queue_path = loop_trigger_queues_path(self.config.cwd.as_path());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = PersistedLoopTimersFile {
            timers: self.loop_timers.timers.values().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        fs::write(path, json)?;

        let queue_json = serde_json::to_string_pretty(&self.loop_timers.trigger_queues)?;
        fs::write(queue_path, queue_json)
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
                        effective_timer_schedule(timer)
                            .map(|schedule| schedule.display().to_string())
                            .unwrap_or_else(|| "no timer trigger".to_string()),
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
    let bindings = trigger_bindings(timer);
    let mut description_parts = vec![
        timer_descriptor(timer).to_string(),
        effective_timer_schedule(timer)
            .map(|schedule| schedule.display().to_string())
            .unwrap_or_else(|| "no timer trigger".to_string()),
        format!("{} trigger(s)", bindings.len()),
        prompt_prefix(&timer.prompt),
        timer.response_mode.title().to_string(),
    ];
    if timer
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

fn format_loop_main_thread_message(loop_id: &str, text: &str) -> String {
    format!("[loop {loop_id}]\n{text}")
}

fn user_text_from_inputs(items: &[codex_protocol::user_input::UserInput]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            codex_protocol::user_input::UserInput::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
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

async fn run_loop_thread_until_completion(
    thread: Arc<CodexThread>,
    loop_input: String,
) -> Result<Option<String>, String> {
    thread
        .submit(Op::UserInput {
            items: vec![codex_protocol::user_input::UserInput::Text {
                text: loop_input,
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await
        .map_err(|err| format!("Failed to submit `/loop` prompt: {err}"))?;

    let mut last_agent_message = None;
    loop {
        match thread.next_event().await {
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
                        .map(|text| text.trim().to_string())
                        .filter(|text| !text.is_empty());
                    return Ok(result);
                }
                EventMsg::Error(error) => {
                    return Err(error.message);
                }
                EventMsg::ShutdownComplete => {
                    return Ok(last_agent_message
                        .map(|text| text.trim().to_string())
                        .filter(|text| !text.is_empty()));
                }
                _ => {}
            },
            Err(err) => {
                return Err(format!("Scheduled loop execution failed: {err}"));
            }
        }
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
        let file_search = FileSearchManager::new(config.cwd.to_path_buf(), app_event_tx.clone());
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
            primary_loop_generated_turn_in_flight: false,
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

        let completion = app.finish_loop_timer(
            "timer-1".to_string(),
            "check status".to_string(),
            Ok(Some("latest answer only".to_string())),
        );
        let cells = completion.cells;

        assert_eq!(cells.len(), 1);
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
            rendered[0].contains("latest answer only"),
            "expected final assistant message, got: {}",
            rendered[0]
        );

        let stored = app
            .loop_timers
            .thread_history_cells
            .get(&primary_thread_id)
            .expect("primary thread history should be recorded");
        assert_eq!(stored.len(), 1);
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
            Ok(Some("latest answer only".to_string())),
        );
        let cells = completion.cells;

        assert_eq!(cells.len(), 1);
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
            rendered[0].contains("latest answer only"),
            "expected final assistant message, got: {}",
            rendered[0]
        );
        assert!(
            !rendered[0].contains("/loop"),
            "did not expect loop prompt in loop summary mode: {}",
            rendered[0]
        );

        let stored = app
            .loop_timers
            .thread_history_cells
            .get(&primary_thread_id)
            .expect("primary thread history should be recorded");
        assert_eq!(stored.len(), 1);
    }
}
