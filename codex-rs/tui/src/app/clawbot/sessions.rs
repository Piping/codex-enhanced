use std::collections::HashSet;

use anyhow::Result;
use codex_clawbot::ClawbotStore;
use codex_clawbot::ConnectionStatus;
use codex_clawbot::ProviderKind;
use codex_clawbot::ProviderRuntimeState;
use codex_clawbot::ProviderSession;

use super::App;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;

pub(super) const CLAWBOT_SESSIONS_PANEL_VIEW_ID: &str = "fork-clawbot-sessions-panel";

pub(super) fn feishu_sessions_menu_description(
    provider_state: Option<&ProviderRuntimeState>,
    sessions: &[ProviderSession],
) -> String {
    let total_sessions = sessions.len();
    let bound_sessions = sessions
        .iter()
        .filter(|session| session.bound_thread_id.is_some())
        .count();
    match provider_state {
        Some(state) => format!(
            "{} · {} total · {} bound",
            state.connection.label(),
            total_sessions,
            bound_sessions
        ),
        None => format!("unconfigured · {total_sessions} total · {bound_sessions} bound"),
    }
}

impl App {
    pub(crate) fn open_clawbot_sessions_panel(&mut self) {
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(CLAWBOT_SESSIONS_PANEL_VIEW_ID);
        if !self.chat_widget.replace_selection_view_if_active(
            CLAWBOT_SESSIONS_PANEL_VIEW_ID,
            self.clawbot_sessions_panel_params(initial_selected_idx),
        ) {
            self.chat_widget
                .show_selection_view(self.clawbot_sessions_panel_params(initial_selected_idx));
        }
    }

    pub(crate) async fn clawbot_scan_sessions(&mut self, provider: ProviderKind) -> Result<()> {
        let mut runtime = self.clawbot_runtime()?;
        runtime.scan_provider_sessions(provider).await?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(crate) fn clawbot_clear_sessions(&mut self, provider: ProviderKind) -> Result<()> {
        let mut runtime = self.clawbot_runtime()?;
        runtime.clear_unbound_sessions(provider)?;
        self.open_clawbot_sessions_panel();
        Ok(())
    }

    pub(super) fn clawbot_sessions_panel_params(
        &self,
        initial_selected_idx: Option<usize>,
    ) -> SelectionViewParams {
        let store = ClawbotStore::new(self.config.cwd.clone());
        let snapshot = store.load_snapshot().unwrap_or_default();
        let provider_state = snapshot.provider_state(ProviderKind::Feishu);
        let bound_session_refs = snapshot
            .sessions
            .iter()
            .filter(|session| session.provider == ProviderKind::Feishu)
            .filter(|session| session.bound_thread_id.is_some())
            .map(ProviderSession::session_ref)
            .collect::<HashSet<_>>();
        let clearable_session_count = snapshot
            .sessions
            .iter()
            .filter(|session| session.provider == ProviderKind::Feishu)
            .filter(|session| session.bound_thread_id.is_none())
            .count();
        let clearable_unread_count = store
            .load_unread_messages()
            .unwrap_or_default()
            .into_iter()
            .filter(|message| message.provider == ProviderKind::Feishu)
            .filter(|message| !bound_session_refs.contains(&message.session_ref()))
            .count();
        let status_description =
            feishu_sessions_menu_description(provider_state, &snapshot.sessions);
        let status_selected_description = provider_state
            .and_then(|state| state.last_error.as_ref())
            .map(|error| format!("Last session/runtime error: {error}"))
            .unwrap_or_else(|| {
                "Inspect Feishu session status and manage scan / clear operations.".to_string()
            });
        let retry_description = match provider_state.map(|state| state.connection) {
            Some(ConnectionStatus::Connected) => {
                "Reconnect the Feishu gateway and refresh websocket delivery."
            }
            Some(ConnectionStatus::Connecting) => {
                "Reconnect the Feishu gateway if the current startup looks stuck."
            }
            Some(ConnectionStatus::Disconnected | ConnectionStatus::Error) => {
                "Reconnect the Feishu gateway using the persisted workspace credentials."
            }
            Some(ConnectionStatus::Unconfigured) | None => {
                "Persist Feishu app credentials first, then retry the gateway connection."
            }
        };

        let mut items = vec![
            SelectionItem {
                name: "Status".to_string(),
                description: Some(status_description),
                selected_description: Some(status_selected_description),
                is_disabled: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Retry Connection".to_string(),
                description: Some(retry_description.to_string()),
                selected_description: Some(
                    "Restart the Feishu runtime task and persist the refreshed connection state."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::ClawbotRetryConnection {
                        provider: ProviderKind::Feishu,
                    })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Manual Bind Session ID".to_string(),
                description: Some(match self.active_thread_id {
                    Some(thread_id) => {
                        format!("Bind a Feishu chat_id directly to thread {thread_id}.")
                    }
                    None => "Open a thread first, then manually bind a Feishu chat_id."
                        .to_string(),
                }),
                selected_description: Some(
                    "Use this when a Feishu p2p session is not visible in the discovered session list."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenClawbotManualBindPrompt))],
                dismiss_on_select: true,
                is_disabled: self.active_thread_id.is_none(),
                ..Default::default()
            },
            SelectionItem {
                name: "Scan Sessions".to_string(),
                description: Some(
                    "Refresh the discovered Feishu session list from the provider API."
                        .to_string(),
                ),
                selected_description: Some(
                    "Use the current Feishu credentials to rescan sessions and refresh status."
                        .to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::ClawbotScanSessions {
                        provider: ProviderKind::Feishu,
                    })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Clear Sessions".to_string(),
                description: Some(format!(
                    "Remove {clearable_session_count} unbound sessions and {clearable_unread_count} cached unread messages."
                )),
                selected_description: Some(
                    "Bound sessions and persisted bindings are preserved.".to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::ClawbotClearSessions {
                        provider: ProviderKind::Feishu,
                    })
                })],
                dismiss_on_select: false,
                is_disabled: clearable_session_count == 0 && clearable_unread_count == 0,
                ..Default::default()
            },
        ];

        let bindings = snapshot.bindings.clone();
        let feishu_sessions = snapshot
            .sessions
            .into_iter()
            .filter(|session| session.provider == ProviderKind::Feishu)
            .collect::<Vec<_>>();
        if feishu_sessions.is_empty() {
            items.push(SelectionItem {
                name: "No Feishu sessions discovered".to_string(),
                description: Some(
                    "Once the gateway is configured and connected, private chats will appear here."
                        .to_string(),
                ),
                selected_description: Some(
                    "Future actions here will connect a discovered session to the current thread."
                        .to_string(),
                ),
                is_disabled: true,
                ..Default::default()
            });
        } else {
            items.extend(feishu_sessions.into_iter().map(|session| {
                let session_ref = session.session_ref();
                let binding = bindings
                    .iter()
                    .find(|binding| binding.session_ref() == session_ref)
                    .cloned();
                let binding_description = match &session.bound_thread_id {
                    Some(thread_id) => format!("thread {thread_id}"),
                    None => "unbound".to_string(),
                };
                let forwarding_description = binding.as_ref().map_or_else(
                    || "inbound on · outbound on".to_string(),
                    |binding| {
                        format!(
                            "inbound {} · outbound {}",
                            if binding.inbound_forwarding_enabled {
                                "on"
                            } else {
                                "off"
                            },
                            if binding.outbound_forwarding_enabled {
                                "on"
                            } else {
                                "off"
                            }
                        )
                    },
                );
                let selected_description = if session.bound_thread_id.is_some() {
                    "Manage binding and unread cache for this session.".to_string()
                } else {
                    "Connect this discovered session to the current thread.".to_string()
                };
                SelectionItem {
                    name: session
                        .display_name
                        .clone()
                        .unwrap_or_else(|| session.session_id.clone()),
                    description: Some(format!(
                        "{} · {} unread · {} · {}",
                        session.status.label(),
                        session.unread_count,
                        binding_description,
                        forwarding_description
                    )),
                    selected_description: Some(selected_description),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenClawbotSessionActions {
                            session: session_ref.clone(),
                        })
                    })],
                    dismiss_on_select: false,
                    ..Default::default()
                }
            }));
        }

        SelectionViewParams {
            view_id: Some(CLAWBOT_SESSIONS_PANEL_VIEW_ID),
            title: Some("Clawbot".to_string()),
            subtitle: Some("Sessions".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            footer_path: Some(store.root_dir().display().to_string()),
            initial_selected_idx,
            items,
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenClawbotPanel))),
            ..Default::default()
        }
    }
}
