use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_clawbot::CachedUnreadMessage;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::FeishuConfig;
use codex_clawbot::FeishuProviderRuntime;
use codex_clawbot::ProviderEvent;
use codex_clawbot::ProviderOutboundTextMessage;
use codex_clawbot::ProviderSessionRef;
use codex_clawbot::feishu_failure_reply_text;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use tokio::sync::mpsc;

use super::App;
use crate::app_command::AppCommand;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::app_server_session::ThreadSessionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingClawbotTurn {
    pub(crate) turn_id: String,
    pub(crate) session: ProviderSessionRef,
}

impl App {
    pub(super) async fn sync_clawbot_workspace(&mut self, app_server: &mut AppServerSession) {
        if let Err(err) = self.sync_clawbot_workspace_inner(app_server).await {
            tracing::warn!(error = %err, "failed to sync clawbot workspace");
            self.chat_widget
                .add_error_message(format!("Clawbot workspace sync failed: {err}"));
        }
    }

    async fn sync_clawbot_workspace_inner(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        let workspace_root = self.config.cwd.to_path_buf();
        let workspace_changed = self.clawbot_workspace_root.as_ref() != Some(&workspace_root);
        if workspace_changed {
            self.abort_clawbot_provider_runtime();
            self.clawbot_workspace_root = Some(workspace_root.clone());
            self.clawbot_pending_turns.clear();
            self.refresh_clawbot_provider_runtime()?;
        }

        let runtime = ClawbotRuntime::load(workspace_root)?;
        let sessions_to_drain = runtime
            .snapshot()
            .sessions
            .iter()
            .filter(|session| session.bound_thread_id.is_some())
            .filter(|session| session.unread_count > 0)
            .map(codex_clawbot::ProviderSession::session_ref)
            .collect::<Vec<_>>();
        for session in sessions_to_drain {
            self.dispatch_next_clawbot_message(app_server, &session)
                .await?;
        }
        Ok(())
    }

    fn start_clawbot_provider_runtime(&mut self, workspace_root: &Path, config: FeishuConfig) {
        self.abort_clawbot_provider_runtime();
        let app_event_tx = self.app_event_tx.clone();
        let workspace = workspace_root.display().to_string();
        self.clawbot_provider_task = Some(tokio::spawn(async move {
            let (provider_event_tx, mut provider_event_rx) = mpsc::unbounded_channel();
            let forward_task = tokio::spawn(async move {
                while let Some(event) = provider_event_rx.recv().await {
                    app_event_tx.send(AppEvent::ClawbotProviderEvent { event });
                }
            });
            if let Err(err) = FeishuProviderRuntime::new(config)
                .run(provider_event_tx)
                .await
            {
                tracing::warn!(workspace, error = %err, "clawbot provider runtime exited");
            }
            forward_task.abort();
            let _ = forward_task.await;
        }));
    }

    pub(super) fn abort_clawbot_provider_runtime(&mut self) {
        if let Some(handle) = self.clawbot_provider_task.take() {
            handle.abort();
        }
    }

    pub(super) fn refresh_clawbot_provider_runtime(&mut self) -> Result<()> {
        let workspace_root = self.config.cwd.to_path_buf();
        self.clawbot_workspace_root = Some(workspace_root.clone());
        let runtime = ClawbotRuntime::load(workspace_root.clone())?;
        if let Some(feishu) = runtime.snapshot().config.feishu.clone()
            && feishu.has_api_credentials()
        {
            self.start_clawbot_provider_runtime(workspace_root.as_path(), feishu);
        } else {
            self.abort_clawbot_provider_runtime();
        }
        Ok(())
    }

    pub(super) async fn handle_clawbot_provider_event(
        &mut self,
        app_server: &mut AppServerSession,
        event: ProviderEvent,
    ) -> Result<()> {
        let Some(workspace_root) = self.clawbot_workspace_root.clone() else {
            return Ok(());
        };
        let session_to_drain = match &event {
            ProviderEvent::InboundMessage(message) => Some(message.session.clone()),
            _ => None,
        };
        let mut runtime = ClawbotRuntime::load(workspace_root)?;
        runtime.apply_provider_event(event)?;
        if let Some(session) = session_to_drain {
            self.dispatch_next_clawbot_message(app_server, &session)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn handle_clawbot_turn_completed(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
        turn: codex_app_server_protocol::Turn,
    ) -> Result<()> {
        let Some(workspace_root) = self.clawbot_workspace_root.clone() else {
            return Ok(());
        };
        let next_session = if let Some(pending) =
            self.take_pending_clawbot_turn(thread_id, &turn.id)
        {
            if let Some(text) = clawbot_outbound_text_for_turn(&turn) {
                self.send_clawbot_thread_reply(workspace_root.as_path(), &pending.session, text)
                    .await?;
            }
            Some(pending.session)
        } else {
            let runtime = ClawbotRuntime::load(workspace_root)?;
            runtime.bound_session_for_thread(&thread_id.to_string())?
        };

        if let Some(session) = next_session {
            self.dispatch_next_clawbot_message(app_server, &session)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn dispatch_next_clawbot_message(
        &mut self,
        app_server: &mut AppServerSession,
        session: &ProviderSessionRef,
    ) -> Result<()> {
        let Some(workspace_root) = self.clawbot_workspace_root.clone() else {
            return Ok(());
        };
        let mut runtime = ClawbotRuntime::load(workspace_root)?;
        let Some(binding) = runtime.load_binding_for_session(session)? else {
            return Ok(());
        };
        if !binding.inbound_forwarding_enabled {
            return Ok(());
        }
        let thread_id = ThreadId::from_string(&binding.thread_id)
            .with_context(|| format!("invalid clawbot thread id `{}`", binding.thread_id))?;
        if self.active_turn_id_for_thread(thread_id).await.is_some() {
            return Ok(());
        }

        let Some(message) = next_unread_message_for_session(&runtime, session) else {
            return Ok(());
        };
        self.attach_clawbot_bound_thread_if_needed(app_server, thread_id)
            .await?;
        let turn_id = self
            .submit_clawbot_message_to_thread(app_server, thread_id, message.text.clone())
            .await?;
        self.register_pending_clawbot_turn(thread_id, session.clone(), turn_id);
        let removed = runtime.take_next_unread_message(session)?;
        if removed.as_ref().map(|entry| entry.message_id.as_str())
            != Some(message.message_id.as_str())
        {
            tracing::warn!(
                session = session.session_id,
                expected = message.message_id,
                actual = removed.as_ref().map(|entry| entry.message_id.as_str()),
                "clawbot unread FIFO state drifted while dispatching"
            );
        }
        Ok(())
    }

    async fn attach_clawbot_bound_thread_if_needed(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
    ) -> Result<()> {
        if self.thread_event_channels.contains_key(&thread_id) {
            return Ok(());
        }
        if let Err(attach_err) = self
            .attach_live_thread_for_selection(app_server, thread_id)
            .await
        {
            tracing::warn!(
                thread_id = %thread_id,
                error = %attach_err,
                "falling back to thread/read for clawbot thread attachment"
            );
            let thread = app_server
                .thread_read(thread_id, /*include_turns*/ false)
                .await
                .map_err(|err| {
                    anyhow::anyhow!("failed to read clawbot thread {thread_id}: {err}")
                })?;
            let session = self.session_state_for_thread_read(thread_id, &thread).await;
            let channel = self.ensure_thread_channel(thread_id);
            let mut store = channel.store.lock().await;
            store.set_session(session, Vec::new());
        }
        Ok(())
    }

    async fn submit_clawbot_message_to_thread(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
        message: String,
    ) -> Result<String> {
        let trimmed = message.trim().to_string();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("cannot submit empty clawbot message"));
        }

        let session = self
            .clawbot_thread_session(thread_id)
            .await?
            .with_context(|| {
                format!("missing live thread session for clawbot thread {thread_id}")
            })?;
        let op = AppCommand::from_core(Op::UserTurn {
            items: vec![UserInput::Text {
                text: trimmed.clone(),
                text_elements: Vec::new(),
            }],
            cwd: session.cwd.clone(),
            approval_policy: session.approval_policy,
            approvals_reviewer: Some(session.approvals_reviewer),
            sandbox_policy: session.sandbox_policy.clone(),
            model: session.model.clone(),
            effort: session.reasoning_effort,
            summary: None,
            service_tier: Some(session.service_tier),
            final_output_json_schema: None,
            collaboration_mode: None,
            personality: self.config.personality,
        });
        crate::session_log::log_outbound_op(&op);
        let response = app_server
            .turn_start(
                thread_id,
                vec![UserInput::Text {
                    text: trimmed,
                    text_elements: Vec::new(),
                }],
                session.cwd,
                session.approval_policy,
                session.approvals_reviewer,
                session.sandbox_policy,
                session.model,
                session.reasoning_effort,
                None,
                Some(session.service_tier),
                None,
                self.config.personality,
                None,
            )
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to start clawbot turn for thread {thread_id}: {err}")
            })?;
        self.note_thread_outbound_op(thread_id, &op).await;
        if let Some(channel) = self.thread_event_channels.get(&thread_id) {
            let mut store = channel.store.lock().await;
            store.active_turn_id = Some(response.turn.id.clone());
        }
        Ok(response.turn.id)
    }

    async fn clawbot_thread_session(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadSessionState>> {
        let Some(channel) = self.thread_event_channels.get(&thread_id) else {
            return Ok(None);
        };
        let store = channel.store.lock().await;
        Ok(store.session.clone())
    }

    async fn send_clawbot_thread_reply(
        &mut self,
        workspace_root: &Path,
        session: &ProviderSessionRef,
        text: String,
    ) -> Result<()> {
        let runtime = ClawbotRuntime::load(workspace_root.to_path_buf())?;
        let Some(binding) = runtime.load_binding_for_session(session)? else {
            return Ok(());
        };
        if !binding.outbound_forwarding_enabled {
            return Ok(());
        }
        self.send_clawbot_outbound_text(
            workspace_root,
            ProviderOutboundTextMessage {
                session: session.clone(),
                text,
            },
        )
        .await
    }

    async fn send_clawbot_outbound_text(
        &mut self,
        workspace_root: &Path,
        message: ProviderOutboundTextMessage,
    ) -> Result<()> {
        #[cfg(test)]
        {
            let _ = workspace_root;
            self.clawbot_outbound_messages.push(message);
            Ok(())
        }

        #[cfg(not(test))]
        {
            let runtime = ClawbotRuntime::load(workspace_root.to_path_buf())?;
            let provider = runtime
                .feishu_provider()
                .context("missing Feishu config for clawbot outbound bridge")?;
            provider.send_text(message).await
        }
    }

    fn register_pending_clawbot_turn(
        &mut self,
        thread_id: ThreadId,
        session: ProviderSessionRef,
        turn_id: String,
    ) {
        self.clawbot_pending_turns
            .entry(thread_id)
            .or_default()
            .push_back(PendingClawbotTurn { turn_id, session });
    }

    fn take_pending_clawbot_turn(
        &mut self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Option<PendingClawbotTurn> {
        let queue = self.clawbot_pending_turns.get_mut(&thread_id)?;
        let pending = queue
            .iter()
            .position(|pending| pending.turn_id == turn_id)
            .and_then(|index| queue.remove(index));
        if queue.is_empty() {
            self.clawbot_pending_turns.remove(&thread_id);
        }
        pending
    }
}

fn clawbot_outbound_text_for_turn(turn: &codex_app_server_protocol::Turn) -> Option<String> {
    match turn.status {
        codex_app_server_protocol::TurnStatus::Completed => {
            super::last_agent_message_for_turn(turn)
        }
        codex_app_server_protocol::TurnStatus::Failed => turn
            .error
            .as_ref()
            .map(|error| feishu_failure_reply_text(&error.message)),
        codex_app_server_protocol::TurnStatus::Interrupted => None,
        codex_app_server_protocol::TurnStatus::InProgress => None,
    }
}

fn next_unread_message_for_session(
    runtime: &ClawbotRuntime,
    session: &ProviderSessionRef,
) -> Option<CachedUnreadMessage> {
    runtime
        .store()
        .load_unread_messages()
        .ok()?
        .into_iter()
        .filter(|message| message.session_ref() == *session)
        .min_by(|left, right| {
            left.received_at
                .cmp(&right.received_at)
                .then(left.message_id.cmp(&right.message_id))
        })
}
