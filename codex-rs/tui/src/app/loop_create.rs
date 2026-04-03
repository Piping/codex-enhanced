use super::App;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use codex_loop::LoopContextMode;
use codex_loop::LoopResponseMode;
use codex_loop::LoopSecurityMode;
use codex_loop::LoopTriggerKind;

const LOOP_CREATE_TRIGGER_VIEW_ID: &str = "fork-loop-create-trigger-panel";
const LOOP_CREATE_RESPONSE_VIEW_ID: &str = "fork-loop-create-response-panel";
const LOOP_CREATE_SECURITY_VIEW_ID: &str = "fork-loop-create-security-panel";

#[derive(Clone)]
pub(crate) struct LoopCreateDraft {
    pub(crate) id: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) trigger_kind: Option<LoopTriggerKind>,
    pub(crate) context_mode: LoopContextMode,
    pub(crate) response_mode: LoopResponseMode,
    pub(crate) security_mode: LoopSecurityMode,
    pub(crate) writable_roots_input: Option<String>,
}

impl LoopCreateDraft {
    pub(crate) fn new(context_mode: LoopContextMode) -> Self {
        Self {
            id: None,
            prompt: None,
            trigger_kind: None,
            context_mode,
            response_mode: LoopResponseMode::default(),
            security_mode: LoopSecurityMode::default(),
            writable_roots_input: None,
        }
    }

    fn subtitle(&self) -> String {
        match self.context_mode {
            LoopContextMode::Embed => "Create loop agent · embed".to_string(),
            LoopContextMode::Ephemeral => "Create loop agent · ephemeral".to_string(),
            LoopContextMode::Persistent => "Create loop agent · persistent".to_string(),
        }
    }
}

impl App {
    pub(crate) fn start_loop_create_draft(&mut self, context_mode: LoopContextMode) {
        self.loop_timers.create_draft = Some(LoopCreateDraft::new(context_mode));
        match context_mode {
            LoopContextMode::Persistent => self.chat_widget.open_create_loop_id_prompt(),
            LoopContextMode::Embed | LoopContextMode::Ephemeral => {
                self.chat_widget.open_create_loop_prompt()
            }
        }
    }

    pub(crate) fn save_create_loop_id(&mut self, id: String) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let id = id.trim();
        if id.is_empty() {
            self.chat_widget
                .add_error_message("Loop id cannot be empty.".to_string());
            return;
        }
        if let Err(err) = codex_loop::validate_loop_id(id) {
            self.chat_widget
                .add_error_message(format!("Failed to create `/loop`: {err}"));
            return;
        }
        draft.id = Some(id.to_string());
        self.chat_widget.open_create_loop_prompt();
    }

    pub(crate) fn save_create_loop_prompt(&mut self, prompt: String) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            self.chat_widget
                .add_error_message("Loop prompt cannot be empty.".to_string());
            return;
        }
        draft.prompt = Some(prompt);
        self.open_create_loop_draft_trigger_menu();
    }

    pub(crate) fn open_create_loop_draft_trigger_menu(&mut self) {
        let Some(draft) = self.loop_timers.create_draft.as_ref() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_CREATE_TRIGGER_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(draft.subtitle()),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "Timer".to_string(),
                    description: Some(
                        "Run this loop whenever an interval or cron schedule becomes due."
                            .to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::OpenCreateLoopTimerSchedulePrompt)
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Idle".to_string(),
                    description: Some(
                        "Run after the main thread stays idle for a configured duration."
                            .to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::OpenCreateLoopIdleAfterPrompt)
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Before Turn".to_string(),
                    description: Some(
                        "Run before a main-thread user turn is submitted.".to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::SaveCreateLoopBeforeTurnTrigger)
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "After Turn".to_string(),
                    description: Some(
                        "Run after the main-thread assistant final response completes.".to_string(),
                    ),
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::SaveCreateLoopAfterTurnTrigger)
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
    }

    pub(crate) fn save_create_loop_timer_schedule(&mut self, schedule: String) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let schedule = match codex_loop::parse_loop_schedule(schedule.trim()) {
            Ok(schedule) => schedule,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to create `/loop`: {err}"));
                return;
            }
        };
        draft.trigger_kind = Some(LoopTriggerKind::Timer { schedule });
        self.open_create_loop_response_mode_menu();
    }

    pub(crate) fn save_create_loop_idle_trigger(&mut self, after: String) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let after = match codex_loop::parse_loop_idle_after(after.trim()) {
            Ok(after) => after,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to create `/loop`: {err}"));
                return;
            }
        };
        draft.trigger_kind = Some(LoopTriggerKind::Idle { after });
        self.open_create_loop_response_mode_menu();
    }

    pub(crate) fn save_create_loop_before_turn_trigger(&mut self) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        draft.trigger_kind = Some(LoopTriggerKind::BeforeTurn);
        self.open_create_loop_response_mode_menu();
    }

    pub(crate) fn save_create_loop_after_turn_trigger(&mut self) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        draft.trigger_kind = Some(LoopTriggerKind::AfterTurn);
        self.open_create_loop_response_mode_menu();
    }

    pub(crate) fn open_create_loop_response_mode_menu(&mut self) {
        let Some(draft) = self.loop_timers.create_draft.as_ref() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let current_mode = draft.response_mode;
        let items = LoopResponseMode::USER_SELECTABLE
            .into_iter()
            .map(|response_mode| SelectionItem {
                name: response_mode.title().to_string(),
                description: Some(response_mode.description().to_string()),
                is_current: current_mode == response_mode,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::SaveCreateLoopResponseMode { response_mode })
                })],
                dismiss_on_select: true,
                ..Default::default()
            })
            .collect();
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_CREATE_RESPONSE_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(draft.subtitle()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new(|tx| {
                tx.send(AppEvent::OpenCreateLoopDraftTriggerMenu)
            })),
            ..Default::default()
        });
    }

    pub(crate) fn save_create_loop_response_mode(&mut self, response_mode: LoopResponseMode) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        draft.response_mode = response_mode;
        self.open_create_loop_security_mode_menu();
    }

    pub(crate) fn open_create_loop_security_mode_menu(&mut self) {
        let Some(draft) = self.loop_timers.create_draft.as_ref() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        let current_mode = draft.security_mode;
        let items = [
            LoopSecurityMode::Inherited,
            LoopSecurityMode::SpecifiedDirectory,
        ]
        .into_iter()
        .map(|security_mode| SelectionItem {
            name: security_mode.title().to_string(),
            description: Some(match security_mode {
                LoopSecurityMode::Inherited => {
                    "Use the main thread's current execution policy.".to_string()
                }
                LoopSecurityMode::SpecifiedDirectory => {
                    "Allow writes only inside explicitly configured directories.".to_string()
                }
            }),
            is_current: current_mode == security_mode,
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::SaveCreateLoopSecurityMode { security_mode })
            })],
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
        self.chat_widget.show_selection_view(SelectionViewParams {
            view_id: Some(LOOP_CREATE_SECURITY_VIEW_ID),
            title: Some("Loop Manager".to_string()),
            subtitle: Some(draft.subtitle()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new(|tx| {
                tx.send(AppEvent::OpenCreateLoopDraftResponseMode)
            })),
            ..Default::default()
        });
    }

    pub(crate) fn save_create_loop_security_mode(&mut self, security_mode: LoopSecurityMode) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        draft.security_mode = security_mode;
        if security_mode == LoopSecurityMode::SpecifiedDirectory {
            self.chat_widget.open_create_loop_writable_roots_prompt();
        } else {
            self.finalize_loop_create_draft();
        }
    }

    pub(crate) fn save_create_loop_writable_roots(&mut self, writable_roots: String) {
        let Some(draft) = self.loop_timers.create_draft.as_mut() else {
            self.chat_widget
                .add_error_message("Loop creation is no longer active.".to_string());
            return;
        };
        draft.writable_roots_input = Some(writable_roots);
        self.finalize_loop_create_draft();
    }
}
