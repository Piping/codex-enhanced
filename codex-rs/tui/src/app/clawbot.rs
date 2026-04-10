use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadStatus;
use codex_app_server_protocol::TurnStatus;
use codex_clawbot::CachedUnreadMessage;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::ClawbotStore;
use codex_clawbot::ClawbotTurnMode;
use codex_clawbot::FeishuConfig;
use codex_clawbot::FeishuProviderRuntime;
use codex_clawbot::PendingClawbotTurn;
use codex_clawbot::ProviderEvent;
use codex_clawbot::ProviderMessageRef;
use codex_clawbot::ProviderOutboundReaction;
use codex_clawbot::ProviderOutboundTextMessage;
use codex_clawbot::ProviderSessionRef;
use codex_clawbot::append_diagnostic_event;
use codex_clawbot::feishu_failure_reply_text;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::GranularApprovalConfig;
use codex_protocol::protocol::Op;
use codex_protocol::request_permissions::PermissionGrantScope;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_protocol::user_input::UserInput;
use tokio::sync::mpsc;

use super::App;
use crate::app_command::AppCommand;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::app_server_session::ThreadSessionState;

const FEISHU_AUTO_ACK_EMOJI_TYPE: &str = "TONGUE";

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
            if let Ok(mut runtime) = ClawbotRuntime::load(workspace_root.clone())
                && runtime.snapshot().config.feishu.is_some()
                && let Err(err) = runtime.scan_feishu_sessions().await
            {
                tracing::warn!(error = %err, "failed to refresh clawbot Feishu sessions");
            }
        }
        self.restore_clawbot_pending_turns()?;
        self.reconcile_clawbot_pending_turns(app_server).await?;

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

    fn clawbot_store(&self) -> Result<ClawbotStore> {
        let workspace_root = self
            .clawbot_workspace_root
            .clone()
            .or_else(|| Some(self.config.cwd.to_path_buf()))
            .context("missing clawbot workspace root")?;
        Ok(ClawbotStore::new(workspace_root))
    }

    fn restore_clawbot_pending_turns(&mut self) -> Result<()> {
        let pending_turns = self.clawbot_store()?.load_pending_turns()?;
        self.clawbot_pending_turns.clear();
        for pending_turn in pending_turns {
            let thread_id = ThreadId::from_string(&pending_turn.thread_id).with_context(|| {
                format!("invalid clawbot thread id `{}`", pending_turn.thread_id)
            })?;
            self.clawbot_pending_turns
                .entry(thread_id)
                .or_default()
                .push_back(pending_turn);
        }
        Ok(())
    }

    async fn reconcile_clawbot_pending_turns(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        let pending_turns = self
            .clawbot_pending_turns
            .values()
            .flat_map(|queue| queue.iter().cloned())
            .collect::<Vec<_>>();
        for pending_turn in pending_turns {
            let thread_id = ThreadId::from_string(&pending_turn.thread_id).with_context(|| {
                format!("invalid clawbot thread id `{}`", pending_turn.thread_id)
            })?;
            let thread = match app_server
                .thread_read(thread_id, /*include_turns*/ true)
                .await
            {
                Ok(thread) => thread,
                Err(err) => {
                    tracing::warn!(
                        thread_id = pending_turn.thread_id,
                        turn_id = pending_turn.turn_id,
                        error = %err,
                        "failed to reconcile clawbot pending turn; clearing stale entry"
                    );
                    let _ = self.take_pending_clawbot_turn(thread_id, &pending_turn.turn_id)?;
                    continue;
                }
            };
            let Some(turn) = thread
                .turns
                .iter()
                .find(|turn| turn.id == pending_turn.turn_id)
                .cloned()
            else {
                if !matches!(thread.status, ThreadStatus::Active { .. }) {
                    let _ = self.take_pending_clawbot_turn(thread_id, &pending_turn.turn_id)?;
                }
                continue;
            };
            match turn.status {
                TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Interrupted => {
                    self.handle_clawbot_turn_completed(app_server, thread_id, turn)
                        .await?;
                }
                TurnStatus::InProgress => {
                    self.attach_clawbot_bound_thread_if_needed(app_server, thread_id)
                        .await?;
                    if let Some(channel) = self.thread_event_channels.get(&thread_id) {
                        let mut store = channel.store.lock().await;
                        store.active_turn_id = Some(turn.id);
                    }
                }
            }
        }
        Ok(())
    }

    fn start_clawbot_provider_runtime(&mut self, workspace_root: &Path, config: FeishuConfig) {
        self.abort_clawbot_provider_runtime();
        let app_event_tx = self.app_event_tx.clone();
        let workspace = workspace_root.display().to_string();
        let workspace_root = workspace_root.to_path_buf();
        self.clawbot_provider_task = Some(tokio::spawn(async move {
            let (provider_event_tx, mut provider_event_rx) = mpsc::unbounded_channel();
            let forward_task = tokio::spawn(async move {
                while let Some(event) = provider_event_rx.recv().await {
                    app_event_tx.send(AppEvent::ClawbotProviderEvent { event });
                }
            });
            if let Err(err) = FeishuProviderRuntime::new(workspace_root.to_path_buf(), config)
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
            ProviderEvent::InboundMessage(message) => {
                let _ = append_diagnostic_event(
                    workspace_root.as_path(),
                    "bridge.inbound_message_received",
                    serde_json::json!({
                        "session_id": message.session.session_id,
                        "message_id": message.message_id,
                        "text": message.text,
                    }),
                );
                Some(message.session.clone())
            }
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
        let next_session =
            if let Some(pending) = self.take_pending_clawbot_turn(thread_id, &turn.id)? {
                let reply_text = if let Some(text) = clawbot_outbound_text_for_turn(&turn) {
                    Some(text)
                } else if matches!(
                    turn.status,
                    codex_app_server_protocol::TurnStatus::Completed
                        | codex_app_server_protocol::TurnStatus::Failed
                        | codex_app_server_protocol::TurnStatus::Interrupted
                ) {
                    match app_server
                        .thread_read(thread_id, /*include_turns*/ true)
                        .await
                    {
                        Ok(thread) => thread
                            .turns
                            .iter()
                            .find(|thread_turn| thread_turn.id == turn.id)
                            .and_then(clawbot_outbound_text_for_turn),
                        Err(err) => {
                            tracing::warn!(
                                thread_id = %thread_id,
                                turn_id = turn.id,
                                error = %err,
                                "failed to load full turn for clawbot outbound reply"
                            );
                            None
                        }
                    }
                } else {
                    None
                };
                let _ = append_diagnostic_event(
                    workspace_root.as_path(),
                    "bridge.turn_completed",
                    serde_json::json!({
                        "session_id": pending.session.session_id.clone(),
                        "thread_id": thread_id,
                        "turn_id": turn.id,
                        "status": format!("{:?}", turn.status),
                        "reply_text": reply_text.clone(),
                    }),
                );
                let reply_result = if let Some(text) = reply_text {
                    self.send_clawbot_thread_reply(workspace_root.as_path(), &pending.session, text)
                        .await
                } else {
                    Ok(())
                };
                let reply_succeeded = reply_result.is_ok();
                if let Err(err) = self
                    .remove_clawbot_auto_ack_reaction(
                        workspace_root.as_path(),
                        ProviderMessageRef::new(
                            pending.session.provider,
                            pending.session.session_id.clone(),
                            pending.message_id.clone(),
                        ),
                    )
                    .await
                {
                    tracing::warn!(
                        thread_id = %thread_id,
                        session = pending.session.session_id,
                        message_id = pending.message_id,
                        error = %err,
                        "failed to clear clawbot auto-ack reaction"
                    );
                    let _ = append_diagnostic_event(
                        workspace_root.as_path(),
                        "bridge.auto_ack_clear_failed",
                        serde_json::json!({
                            "session_id": pending.session.session_id,
                            "thread_id": thread_id,
                            "message_id": pending.message_id,
                            "emoji_type": FEISHU_AUTO_ACK_EMOJI_TYPE,
                            "error": err.to_string(),
                        }),
                    );
                }
                reply_result?;
                if reply_succeeded {
                    let mut runtime = ClawbotRuntime::load(workspace_root.clone())?;
                    let removed = runtime.take_next_unread_message(&pending.session)?;
                    if removed.as_ref().map(|entry| entry.message_id.as_str())
                        != Some(pending.message_id.as_str())
                    {
                        tracing::warn!(
                            session = pending.session.session_id,
                            expected = pending.message_id,
                            actual = removed.as_ref().map(|entry| entry.message_id.as_str()),
                            "clawbot unread FIFO state drifted while completing turn"
                        );
                    }
                }
                Some(pending.session)
            } else {
                let runtime = ClawbotRuntime::load(workspace_root.clone())?;
                let bound_session = runtime.bound_session_for_thread(&thread_id.to_string())?;
                if let Some(session) = bound_session.as_ref()
                    && let Some(text) = clawbot_outbound_text_for_turn(&turn)
                {
                    let _ = append_diagnostic_event(
                        workspace_root.as_path(),
                        "bridge.bound_thread_turn_completed",
                        serde_json::json!({
                            "session_id": session.session_id,
                            "thread_id": thread_id,
                            "turn_id": turn.id,
                            "status": format!("{:?}", turn.status),
                            "reply_text": text.clone(),
                        }),
                    );
                    self.send_clawbot_thread_reply(workspace_root.as_path(), session, text)
                        .await?;
                }
                bound_session
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
        let runtime = ClawbotRuntime::load(workspace_root.clone())?;
        let Some(binding) = runtime.load_binding_for_session(session)? else {
            let _ = append_diagnostic_event(
                workspace_root.as_path(),
                "bridge.dispatch_skipped",
                serde_json::json!({
                    "reason": "missing_binding",
                    "session_id": session.session_id,
                }),
            );
            return Ok(());
        };
        if !binding.inbound_forwarding_enabled {
            let _ = append_diagnostic_event(
                workspace_root.as_path(),
                "bridge.dispatch_skipped",
                serde_json::json!({
                    "reason": "inbound_forwarding_disabled",
                    "session_id": session.session_id,
                    "thread_id": binding.thread_id,
                }),
            );
            return Ok(());
        }
        let thread_id = ThreadId::from_string(&binding.thread_id)
            .with_context(|| format!("invalid clawbot thread id `{}`", binding.thread_id))?;
        if self.active_turn_id_for_thread(thread_id).await.is_some() {
            let _ = append_diagnostic_event(
                workspace_root.as_path(),
                "bridge.dispatch_skipped",
                serde_json::json!({
                    "reason": "thread_busy",
                    "session_id": session.session_id,
                    "thread_id": thread_id,
                }),
            );
            return Ok(());
        }
        if self
            .clawbot_pending_turns
            .get(&thread_id)
            .is_some_and(|queue| queue.iter().any(|pending| pending.session == *session))
        {
            let _ = append_diagnostic_event(
                workspace_root.as_path(),
                "bridge.dispatch_skipped",
                serde_json::json!({
                    "reason": "pending_turn",
                    "session_id": session.session_id,
                    "thread_id": thread_id,
                }),
            );
            return Ok(());
        }

        let Some(message) = next_unread_message_for_session(&runtime, session) else {
            let _ = append_diagnostic_event(
                workspace_root.as_path(),
                "bridge.dispatch_skipped",
                serde_json::json!({
                    "reason": "no_unread_message",
                    "session_id": session.session_id,
                    "thread_id": thread_id,
                }),
            );
            return Ok(());
        };
        let turn_mode = runtime.snapshot().config.turn_mode;
        self.attach_clawbot_bound_thread_if_needed(app_server, thread_id)
            .await?;
        if self.active_thread_id.is_some() && self.active_thread_id != Some(thread_id) {
            let session_title = runtime
                .snapshot()
                .sessions
                .iter()
                .find(|provider_session| provider_session.session_ref() == *session)
                .map_or_else(|| session.session_id.clone(), session_title);
            self.chat_widget.add_info_message(
                format!(
                    "New Feishu message from {session_title} was imported into agent thread {}.",
                    self.thread_label(thread_id)
                ),
                Some("Open /clawbot to inspect bindings and jump.".to_string()),
            );
        }
        if let Err(err) = self
            .send_clawbot_outbound_reaction(ProviderOutboundReaction {
                target: ProviderMessageRef::new(
                    message.provider,
                    message.session_id.clone(),
                    message.message_id.clone(),
                ),
                emoji_type: FEISHU_AUTO_ACK_EMOJI_TYPE.to_string(),
            })
            .await
        {
            tracing::warn!(
                thread_id = %thread_id,
                session = session.session_id,
                error = %err,
                "failed to auto-ack clawbot inbound message"
            );
            let _ = append_diagnostic_event(
                workspace_root.as_path(),
                "bridge.auto_ack_failed",
                serde_json::json!({
                    "session_id": session.session_id,
                    "thread_id": thread_id,
                    "message_id": message.message_id,
                    "emoji_type": FEISHU_AUTO_ACK_EMOJI_TYPE,
                    "error": err.to_string(),
                }),
            );
        }
        let turn_id = self
            .submit_clawbot_message_to_thread(
                app_server,
                thread_id,
                message.text.clone(),
                turn_mode,
            )
            .await?;
        let _ = append_diagnostic_event(
            workspace_root.as_path(),
            "bridge.thread_turn_started",
            serde_json::json!({
                "session_id": session.session_id,
                "thread_id": thread_id,
                "turn_id": turn_id,
                "message_id": message.message_id.clone(),
                "text": message.text.clone(),
            }),
        );
        self.register_pending_clawbot_turn(
            thread_id,
            session.clone(),
            turn_id,
            message.message_id.clone(),
            turn_mode,
        );
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
        turn_mode: ClawbotTurnMode,
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
        let op: AppCommand = Op::UserTurn {
            items: vec![UserInput::Text {
                text: trimmed.clone(),
                text_elements: Vec::new(),
            }],
            cwd: session.cwd.clone(),
            approval_policy: clawbot_approval_policy(session.approval_policy, turn_mode),
            approvals_reviewer: Some(session.approvals_reviewer),
            sandbox_policy: session.sandbox_policy.clone(),
            model: session.model.clone(),
            effort: session.reasoning_effort,
            summary: None,
            service_tier: Some(session.service_tier),
            final_output_json_schema: None,
            collaboration_mode: None,
            personality: self.config.personality,
        }
        .into();
        crate::session_log::log_outbound_op(&op);
        let response = app_server
            .turn_start(
                thread_id,
                vec![UserInput::Text {
                    text: trimmed,
                    text_elements: Vec::new(),
                }],
                session.cwd,
                clawbot_approval_policy(session.approval_policy, turn_mode),
                session.approvals_reviewer,
                session.sandbox_policy,
                session.model,
                session.reasoning_effort,
                /*summary*/ None,
                Some(session.service_tier),
                /*collaboration_mode*/ None,
                self.config.personality,
                /*output_schema*/ None,
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
            let _ = append_diagnostic_event(
                workspace_root,
                "bridge.reply_skipped",
                serde_json::json!({
                    "reason": "missing_binding",
                    "session_id": session.session_id,
                    "text": text,
                }),
            );
            return Ok(());
        };
        if !binding.outbound_forwarding_enabled {
            let _ = append_diagnostic_event(
                workspace_root,
                "bridge.reply_skipped",
                serde_json::json!({
                    "reason": "outbound_forwarding_disabled",
                    "session_id": session.session_id,
                    "thread_id": binding.thread_id,
                    "text": text,
                }),
            );
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
            let session_id = message.session.session_id.clone();
            let text = message.text.clone();
            let send_result = provider.send_text(message).await;
            let kind = if send_result.is_ok() {
                "bridge.reply_forwarded"
            } else {
                "bridge.reply_forward_failed"
            };
            let _ = append_diagnostic_event(
                workspace_root,
                kind,
                serde_json::json!({
                    "session_id": session_id,
                    "text": text,
                    "error": send_result.as_ref().err().map(ToString::to_string),
                }),
            );
            send_result
        }
    }

    async fn send_clawbot_outbound_reaction(
        &mut self,
        reaction: ProviderOutboundReaction,
    ) -> Result<()> {
        #[cfg(test)]
        {
            self.clawbot_outbound_reactions.push(reaction);
            Ok(())
        }

        #[cfg(not(test))]
        {
            let workspace_root = self
                .clawbot_workspace_root
                .clone()
                .context("missing clawbot workspace root for outbound reaction")?;
            let runtime = ClawbotRuntime::load(workspace_root)?;
            let provider = runtime
                .feishu_provider()
                .context("missing Feishu config for clawbot outbound bridge")?;
            provider.add_reaction(reaction).await
        }
    }

    async fn remove_clawbot_auto_ack_reaction(
        &mut self,
        workspace_root: &Path,
        target: ProviderMessageRef,
    ) -> Result<()> {
        let reaction = ProviderOutboundReaction {
            target,
            emoji_type: FEISHU_AUTO_ACK_EMOJI_TYPE.to_string(),
        };
        #[cfg(test)]
        {
            let _ = workspace_root;
            self.clawbot_removed_outbound_reactions.push(reaction);
            Ok(())
        }

        #[cfg(not(test))]
        {
            let runtime = ClawbotRuntime::load(workspace_root.to_path_buf())?;
            let provider = runtime
                .feishu_provider()
                .context("missing Feishu config for clawbot reaction cleanup")?;
            provider.remove_reaction(reaction).await
        }
    }

    fn register_pending_clawbot_turn(
        &mut self,
        thread_id: ThreadId,
        session: ProviderSessionRef,
        turn_id: String,
        message_id: String,
        turn_mode: ClawbotTurnMode,
    ) {
        let pending_turn = PendingClawbotTurn {
            thread_id: thread_id.to_string(),
            turn_id,
            session,
            message_id,
            turn_mode,
        };
        self.clawbot_pending_turns
            .entry(thread_id)
            .or_default()
            .push_back(pending_turn.clone());
        if let Err(err) = self
            .clawbot_store()
            .and_then(|store| store.upsert_pending_turn(pending_turn))
        {
            tracing::warn!(
                thread_id = %thread_id,
                error = %err,
                "failed to persist clawbot pending turn"
            );
        }
    }

    fn take_pending_clawbot_turn(
        &mut self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Result<Option<PendingClawbotTurn>> {
        let Some(queue) = self.clawbot_pending_turns.get_mut(&thread_id) else {
            return Ok(None);
        };
        let pending = queue
            .iter()
            .position(|pending| pending.turn_id == turn_id)
            .and_then(|index| queue.remove(index));
        if queue.is_empty() {
            self.clawbot_pending_turns.remove(&thread_id);
        }
        if pending.is_some() {
            let _ = self.remove_pending_clawbot_turn(thread_id, turn_id)?;
        }
        Ok(pending)
    }

    fn remove_pending_clawbot_turn(
        &mut self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Result<Option<PendingClawbotTurn>> {
        self.clawbot_store()?
            .remove_pending_turn(&thread_id.to_string(), turn_id)
    }

    fn clawbot_turn_mode_for_turn(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Option<ClawbotTurnMode> {
        self.clawbot_pending_turns
            .get(&thread_id)?
            .iter()
            .find(|pending| pending.turn_id == turn_id)
            .map(|pending| pending.turn_mode)
    }

    pub(super) fn clawbot_auto_response_op_for_server_request(
        &self,
        thread_id: ThreadId,
        request: &ServerRequest,
    ) -> Option<AppCommand> {
        match request {
            ServerRequest::ToolRequestUserInput { params, .. } => {
                let turn_mode = self.clawbot_turn_mode_for_turn(thread_id, &params.turn_id)?;
                if !turn_mode.uses_noninteractive_prompt_handling() {
                    return None;
                }
                Some(AppCommand::user_input_answer(
                    params.turn_id.clone(),
                    RequestUserInputResponse {
                        answers: HashMap::new(),
                    },
                ))
            }
            ServerRequest::PermissionsRequestApproval { params, .. } => {
                let turn_mode = self.clawbot_turn_mode_for_turn(thread_id, &params.turn_id)?;
                if !turn_mode.uses_noninteractive_prompt_handling() {
                    return None;
                }
                Some(AppCommand::request_permissions_response(
                    params.item_id.clone(),
                    RequestPermissionsResponse {
                        permissions: Default::default(),
                        scope: PermissionGrantScope::Turn,
                    },
                ))
            }
            _ => None,
        }
    }

    pub(super) async fn maybe_auto_resolve_clawbot_server_request(
        &mut self,
        app_server: &AppServerSession,
        thread_id: ThreadId,
        request: &ServerRequest,
    ) -> bool {
        let Some(op) = self.clawbot_auto_response_op_for_server_request(thread_id, request) else {
            return false;
        };

        match self
            .try_resolve_app_server_request(app_server, thread_id, &op)
            .await
        {
            Ok(true) => true,
            Ok(false) => {
                self.pending_app_server_requests
                    .note_server_request(request);
                false
            }
            Err(err) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    error = %err,
                    "failed to auto-resolve clawbot server request"
                );
                self.pending_app_server_requests
                    .note_server_request(request);
                false
            }
        }
    }
}

fn clawbot_approval_policy(
    existing_policy: AskForApproval,
    turn_mode: ClawbotTurnMode,
) -> AskForApproval {
    if !turn_mode.uses_noninteractive_prompt_handling() {
        return existing_policy;
    }

    AskForApproval::Granular(GranularApprovalConfig {
        sandbox_approval: false,
        rules: false,
        skill_approval: false,
        request_permissions: false,
        mcp_elicitations: false,
    })
}

fn clawbot_outbound_text_for_turn(turn: &codex_app_server_protocol::Turn) -> Option<String> {
    match turn.status {
        codex_app_server_protocol::TurnStatus::Completed => last_agent_message_for_turn(turn),
        codex_app_server_protocol::TurnStatus::Failed => turn
            .error
            .as_ref()
            .map(|error| feishu_failure_reply_text(&error.message)),
        codex_app_server_protocol::TurnStatus::Interrupted => {
            last_agent_message_for_turn(turn).or_else(|| Some("Request interrupted.".to_string()))
        }
        codex_app_server_protocol::TurnStatus::InProgress => None,
    }
}

fn last_agent_message_for_turn(turn: &codex_app_server_protocol::Turn) -> Option<String> {
    turn.items.iter().rev().find_map(|item| {
        let codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } = item else {
            return None;
        };
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
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

fn session_title(session: &codex_clawbot::ProviderSession) -> String {
    session
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| session.session_id.clone())
}

#[cfg(test)]
mod tests {
    use codex_app_server_protocol::ThreadItem;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnStatus;
    use pretty_assertions::assert_eq;

    use super::clawbot_outbound_text_for_turn;

    #[test]
    fn completed_turn_uses_last_agent_message_for_reply() {
        let turn = Turn {
            id: "turn-1".to_string(),
            status: TurnStatus::Completed,
            items: vec![
                ThreadItem::AgentMessage {
                    id: "agent-1".to_string(),
                    text: "draft reply".to_string(),
                    phase: None,
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "agent-2".to_string(),
                    text: "final reply".to_string(),
                    phase: None,
                    memory_citation: None,
                },
            ],
            error: None,
        };

        assert_eq!(
            clawbot_outbound_text_for_turn(&turn),
            Some("final reply".to_string())
        );
    }

    #[test]
    fn interrupted_turn_uses_last_agent_message_for_reply() {
        let turn = Turn {
            id: "turn-1".to_string(),
            status: TurnStatus::Interrupted,
            items: vec![
                ThreadItem::AgentMessage {
                    id: "agent-1".to_string(),
                    text: "first reply".to_string(),
                    phase: None,
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "agent-2".to_string(),
                    text: "  ".to_string(),
                    phase: None,
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "agent-3".to_string(),
                    text: "last reply".to_string(),
                    phase: None,
                    memory_citation: None,
                },
            ],
            error: None,
        };

        assert_eq!(
            clawbot_outbound_text_for_turn(&turn),
            Some("last reply".to_string())
        );
    }
}
