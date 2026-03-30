mod session_admin;

use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;

use crate::config::ClawbotTurnMode;
use crate::config::FeishuConfig;
use crate::model::CachedUnreadMessage;
use crate::model::ClawbotSnapshot;
use crate::model::InboundMessageReceipt;
use crate::model::ProviderRuntimeState;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;
use crate::model::SessionBinding;
use crate::model::SessionForwardingMode;
use crate::model::SessionStatus;
use crate::provider::FeishuProviderRuntime;
use crate::provider::ProviderEvent;
use crate::store::ClawbotStore;

#[derive(Debug)]
pub struct ClawbotRuntime {
    store: ClawbotStore,
    snapshot: ClawbotSnapshot,
}

impl ClawbotRuntime {
    pub fn load(workspace_root: PathBuf) -> Result<Self> {
        let store = ClawbotStore::new(workspace_root);
        let snapshot = store.load_snapshot()?;
        Ok(Self { store, snapshot })
    }

    pub fn reload(&mut self) -> Result<&ClawbotSnapshot> {
        self.snapshot = self.store.load_snapshot()?;
        Ok(&self.snapshot)
    }

    pub fn snapshot(&self) -> &ClawbotSnapshot {
        &self.snapshot
    }

    pub fn store(&self) -> &ClawbotStore {
        &self.store
    }

    pub fn feishu_provider(&self) -> Option<FeishuProviderRuntime> {
        self.snapshot
            .config
            .feishu
            .clone()
            .map(FeishuProviderRuntime::new)
    }

    pub fn persist_runtime_state(
        &mut self,
        state: ProviderRuntimeState,
    ) -> Result<&ClawbotSnapshot> {
        self.store.upsert_runtime_state(state)?;
        self.reload()
    }

    pub fn persist_session(&mut self, session: ProviderSession) -> Result<&ClawbotSnapshot> {
        self.store.upsert_session(session)?;
        self.reload()
    }

    pub fn persist_binding(&mut self, binding: SessionBinding) -> Result<&ClawbotSnapshot> {
        self.store.upsert_binding(binding)?;
        self.reload()
    }

    pub fn cache_unread_message(
        &mut self,
        message: CachedUnreadMessage,
    ) -> Result<&ClawbotSnapshot> {
        self.store.append_unread_message(&message)?;
        self.reload()
    }

    pub fn take_unread_messages(
        &mut self,
        session: &ProviderSessionRef,
    ) -> Result<Vec<CachedUnreadMessage>> {
        let unread_messages = self.store.take_unread_messages(session)?;
        self.reload()?;
        Ok(unread_messages)
    }

    pub fn update_feishu_config(
        &mut self,
        config: Option<FeishuConfig>,
    ) -> Result<&ClawbotSnapshot> {
        self.snapshot.config.feishu = config;
        self.store.save_config(&self.snapshot.config)?;
        self.reload()
    }

    pub fn update_turn_mode(&mut self, mode: ClawbotTurnMode) -> Result<&ClawbotSnapshot> {
        self.snapshot.config.turn_mode = mode;
        self.store.save_config(&self.snapshot.config)?;
        self.reload()
    }

    pub fn connect_session_to_thread(
        &mut self,
        session: &ProviderSessionRef,
        thread_id: String,
    ) -> Result<&ClawbotSnapshot> {
        let now = unix_timestamp_now()?;
        let mut bindings = self.store.load_bindings()?;
        let mut sessions = self.store.load_sessions()?;
        let existing_binding = bindings
            .iter()
            .find(|binding| binding.session_ref() == *session)
            .cloned();

        for binding in &bindings {
            if binding.thread_id == thread_id
                && binding.session_ref() != *session
                && let Some(existing_session) = sessions.iter_mut().find(|existing_session| {
                    existing_session.session_ref() == binding.session_ref()
                })
            {
                existing_session.bound_thread_id = None;
                existing_session.status = SessionStatus::Discovered;
            }
        }

        bindings
            .retain(|binding| binding.thread_id != thread_id || binding.session_ref() == *session);

        if let Some(binding) = bindings
            .iter_mut()
            .find(|binding| binding.session_ref() == *session)
        {
            binding.thread_id = thread_id.clone();
            binding.updated_at = now;
        } else {
            bindings.push(SessionBinding {
                provider: session.provider,
                session_id: session.session_id.clone(),
                thread_id: thread_id.clone(),
                inbound_forwarding_enabled: true,
                outbound_forwarding_enabled: true,
                created_at: existing_binding
                    .as_ref()
                    .map_or(now, |binding| binding.created_at),
                updated_at: now,
            });
        }

        if let Some(provider_session) = sessions
            .iter_mut()
            .find(|provider_session| provider_session.session_ref() == *session)
        {
            provider_session.bound_thread_id = Some(thread_id.clone());
            provider_session.status = SessionStatus::Bound;
        } else {
            sessions.push(ProviderSession {
                provider: session.provider,
                session_id: session.session_id.clone(),
                display_name: None,
                unread_count: self.unread_count_for_session(session)?,
                last_message_at: None,
                status: SessionStatus::Bound,
                bound_thread_id: Some(thread_id.clone()),
            });
        }

        self.store.save_bindings(&bindings)?;
        self.store.save_sessions(&sessions)?;
        self.reload()
    }

    pub fn disconnect_session(&mut self, session: &ProviderSessionRef) -> Result<&ClawbotSnapshot> {
        let mut provider_session = self
            .load_session(session)?
            .ok_or_else(|| anyhow!("session `{}` not found", session.session_id))?;
        provider_session.bound_thread_id = None;
        provider_session.status = SessionStatus::Discovered;

        self.store.remove_binding(session)?;
        self.store.upsert_session(provider_session)?;
        self.reload()
    }

    pub fn bound_session_for_thread(&self, thread_id: &str) -> Result<Option<ProviderSessionRef>> {
        Ok(self
            .load_binding_for_thread(thread_id)?
            .as_ref()
            .map(SessionBinding::session_ref))
    }

    pub fn load_binding_for_thread(&self, thread_id: &str) -> Result<Option<SessionBinding>> {
        Ok(self
            .store
            .load_bindings()?
            .into_iter()
            .find(|binding| binding.thread_id == thread_id))
    }

    pub fn load_binding_for_session(
        &self,
        session: &ProviderSessionRef,
    ) -> Result<Option<SessionBinding>> {
        Ok(self
            .store
            .load_bindings()?
            .into_iter()
            .find(|binding| binding.session_ref() == *session))
    }

    pub fn set_session_forwarding(
        &mut self,
        session: &ProviderSessionRef,
        mode: SessionForwardingMode,
    ) -> Result<&ClawbotSnapshot> {
        let mut bindings = self.store.load_bindings()?;
        let binding = bindings
            .iter_mut()
            .find(|binding| binding.session_ref() == *session)
            .ok_or_else(|| anyhow!("session `{}` is not currently bound", session.session_id))?;
        binding.set_forwarding_mode(mode);
        binding.updated_at = unix_timestamp_now()?;
        self.store.save_bindings(&bindings)?;
        self.reload()
    }

    pub fn flush_cached_messages(
        &mut self,
        session: &ProviderSessionRef,
    ) -> Result<Vec<CachedUnreadMessage>> {
        let cached_messages = self.store.take_unread_messages(session)?;
        if let Some(mut provider_session) = self.load_session(session)? {
            provider_session.unread_count = provider_session
                .unread_count
                .saturating_sub(cached_messages.len());
            self.store.upsert_session(provider_session)?;
        }
        self.reload()?;
        Ok(cached_messages)
    }

    pub fn apply_provider_event(&mut self, event: ProviderEvent) -> Result<&ClawbotSnapshot> {
        match event {
            ProviderEvent::RuntimeStateUpdated(state) => {
                self.store.upsert_runtime_state(state)?;
            }
            ProviderEvent::SessionUpserted(mut session) => {
                session.bound_thread_id = self.lookup_bound_thread_id(&session.session_ref())?;
                session.unread_count = self.unread_count_for_session(&session.session_ref())?;
                if session.bound_thread_id.is_some() {
                    session.status = SessionStatus::Bound;
                }
                self.store.upsert_session(session)?;
            }
            ProviderEvent::SessionRemoved(session) => {
                self.store.remove_session(&session)?;
            }
            ProviderEvent::InboundMessage(message) => {
                if self
                    .store
                    .has_inbound_receipt(&message.session, &message.message_id)?
                {
                    return self.reload();
                }

                self.store.append_unread_message(&CachedUnreadMessage {
                    provider: message.session.provider,
                    session_id: message.session.session_id.clone(),
                    message_id: message.message_id.clone(),
                    text: message.text,
                    received_at: message.received_at,
                })?;
                self.store.record_inbound_receipt(InboundMessageReceipt {
                    provider: message.session.provider,
                    session_id: message.session.session_id.clone(),
                    message_id: message.message_id.clone(),
                    received_at: message.received_at,
                })?;

                let mut session = self
                    .load_session(&message.session)?
                    .unwrap_or(ProviderSession {
                        provider: message.session.provider,
                        session_id: message.session.session_id.clone(),
                        display_name: None,
                        unread_count: 0,
                        last_message_at: None,
                        status: SessionStatus::Discovered,
                        bound_thread_id: None,
                    });
                session.bound_thread_id = self.lookup_bound_thread_id(&message.session)?;
                session.unread_count = self.unread_count_for_session(&message.session)?;
                session.last_message_at = Some(message.received_at);
                if session.bound_thread_id.is_some() {
                    session.status = SessionStatus::Bound;
                }
                self.store.upsert_session(session)?;
            }
        }

        self.reload()
    }

    fn load_session(&self, session: &ProviderSessionRef) -> Result<Option<ProviderSession>> {
        Ok(self
            .store
            .load_sessions()?
            .into_iter()
            .find(|existing| existing.session_ref() == *session))
    }

    fn lookup_bound_thread_id(&self, session: &ProviderSessionRef) -> Result<Option<String>> {
        Ok(self
            .store
            .load_bindings()?
            .into_iter()
            .find(|binding| binding.session_ref() == *session)
            .map(|binding| binding.thread_id))
    }

    fn unread_count_for_session(&self, session: &ProviderSessionRef) -> Result<usize> {
        Ok(self
            .store
            .load_unread_messages()?
            .into_iter()
            .filter(|message| message.session_ref() == *session)
            .count())
    }
}

fn unix_timestamp_now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::ClawbotRuntime;
    use crate::config::ClawbotConfig;
    use crate::config::FeishuConfig;
    use crate::events::ProviderInboundMessage;
    use crate::model::CachedUnreadMessage;
    use crate::model::ConnectionStatus;
    use crate::model::ProviderKind;
    use crate::model::ProviderRuntimeState;
    use crate::model::ProviderSession;
    use crate::model::ProviderSessionRef;
    use crate::model::SessionForwardingDirection;
    use crate::model::SessionForwardingMode;
    use crate::model::SessionStatus;
    use crate::provider::ProviderEvent;

    #[test]
    fn connect_flush_and_disconnect_update_binding_and_unread_state() {
        let tempdir = tempdir().expect("tempdir");
        let workspace_root = tempdir.path().to_path_buf();
        let mut runtime = ClawbotRuntime::load(workspace_root).expect("runtime");

        runtime
            .update_feishu_config(Some(FeishuConfig {
                app_id: "cli_a".to_string(),
                app_secret: "secret".to_string(),
                verification_token: Some("verify".to_string()),
                encrypt_key: None,
                bot_open_id: None,
                bot_user_id: None,
            }))
            .expect("config");
        runtime
            .persist_session(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                display_name: Some("Alice".to_string()),
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })
            .expect("session");
        runtime
            .cache_unread_message(CachedUnreadMessage {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                message_id: "msg_1".to_string(),
                text: "hello".to_string(),
                received_at: 1,
            })
            .expect("cache");
        runtime
            .persist_session(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                display_name: Some("Alice".to_string()),
                unread_count: 1,
                last_message_at: Some(1),
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })
            .expect("session unread");

        let session = ProviderSessionRef::new(ProviderKind::Feishu, "chat_1");
        runtime
            .connect_session_to_thread(&session, "thread_123".to_string())
            .expect("connect");
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.bindings.len(), 1);
        assert_eq!(
            snapshot.sessions[0].bound_thread_id.as_deref(),
            Some("thread_123")
        );
        assert_eq!(snapshot.sessions[0].status, SessionStatus::Bound);
        let binding = runtime
            .load_binding_for_thread("thread_123")
            .expect("binding lookup")
            .expect("binding");
        assert_eq!(binding.inbound_forwarding_enabled, true);
        assert_eq!(binding.outbound_forwarding_enabled, true);

        let flushed = runtime.flush_cached_messages(&session).expect("flush");
        assert_eq!(flushed.len(), 1);
        assert_eq!(runtime.snapshot().unread_message_count, 0);
        assert_eq!(runtime.snapshot().sessions[0].unread_count, 0);

        runtime.disconnect_session(&session).expect("disconnect");
        assert_eq!(runtime.snapshot().bindings.len(), 0);
        assert_eq!(runtime.snapshot().sessions[0].bound_thread_id, None);
        assert_eq!(
            runtime.snapshot().sessions[0].status,
            SessionStatus::Discovered
        );
    }

    #[test]
    fn apply_provider_event_preserves_binding_and_updates_unread_count() {
        let tempdir = tempdir().expect("tempdir");
        let workspace_root = tempdir.path().to_path_buf();
        let mut runtime = ClawbotRuntime::load(workspace_root.clone()).expect("runtime");

        fs::create_dir_all(workspace_root.join(".codex/clawbot")).expect("clawbot dir");
        runtime
            .store()
            .save_config(&ClawbotConfig {
                feishu: Some(FeishuConfig {
                    app_id: "cli_a".to_string(),
                    app_secret: "secret".to_string(),
                    verification_token: Some("verify".to_string()),
                    encrypt_key: None,
                    bot_open_id: None,
                    bot_user_id: None,
                }),
                ..Default::default()
            })
            .expect("config");
        runtime
            .persist_session(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_2".to_string(),
                display_name: Some("Bob".to_string()),
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })
            .expect("session");
        runtime
            .connect_session_to_thread(
                &ProviderSessionRef::new(ProviderKind::Feishu, "chat_2"),
                "thread_456".to_string(),
            )
            .expect("connect");

        runtime
            .apply_provider_event(ProviderEvent::RuntimeStateUpdated(ProviderRuntimeState {
                provider: ProviderKind::Feishu,
                connection: ConnectionStatus::Connected,
                last_error: None,
                updated_at: Some(10),
            }))
            .expect("runtime state");
        runtime
            .apply_provider_event(ProviderEvent::SessionUpserted(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_2".to_string(),
                display_name: Some("Bob Updated".to_string()),
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            }))
            .expect("session upsert");
        runtime
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session: ProviderSessionRef::new(ProviderKind::Feishu, "chat_2"),
                message_id: "msg_2".to_string(),
                text: "hello".to_string(),
                received_at: 20,
            }))
            .expect("inbound");

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.runtime[0].connection, ConnectionStatus::Connected);
        assert_eq!(snapshot.unread_message_count, 1);
        assert_eq!(
            snapshot.sessions[0].display_name.as_deref(),
            Some("Bob Updated")
        );
        assert_eq!(
            snapshot.sessions[0].bound_thread_id.as_deref(),
            Some("thread_456")
        );
        assert_eq!(snapshot.sessions[0].unread_count, 1);
        assert_eq!(snapshot.sessions[0].status, SessionStatus::Bound);
    }

    #[test]
    fn bound_session_for_thread_returns_persisted_binding() {
        let tempdir = tempdir().expect("tempdir");
        let mut runtime = ClawbotRuntime::load(tempdir.path().to_path_buf()).expect("runtime");
        runtime
            .persist_session(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_3".to_string(),
                display_name: Some("Carol".to_string()),
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })
            .expect("session");
        let session = ProviderSessionRef::new(ProviderKind::Feishu, "chat_3");
        runtime
            .connect_session_to_thread(&session, "thread_789".to_string())
            .expect("connect");

        assert_eq!(
            runtime
                .bound_session_for_thread("thread_789")
                .expect("binding lookup"),
            Some(session)
        );
        assert_eq!(
            runtime
                .bound_session_for_thread("thread_missing")
                .expect("missing binding lookup"),
            None
        );
    }

    #[test]
    fn set_session_forwarding_updates_persisted_binding() {
        let tempdir = tempdir().expect("tempdir");
        let mut runtime = ClawbotRuntime::load(tempdir.path().to_path_buf()).expect("runtime");
        runtime
            .persist_session(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_5".to_string(),
                display_name: Some("Eve".to_string()),
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })
            .expect("session");
        let session = ProviderSessionRef::new(ProviderKind::Feishu, "chat_5");
        runtime
            .connect_session_to_thread(&session, "thread_abc".to_string())
            .expect("connect");

        runtime
            .set_session_forwarding(&session, SessionForwardingMode::InboundDisabled)
            .expect("disable inbound");
        runtime
            .set_session_forwarding(&session, SessionForwardingMode::OutboundDisabled)
            .expect("disable outbound");

        let binding = runtime
            .load_binding_for_session(&session)
            .expect("binding lookup")
            .expect("binding");
        assert_eq!(
            binding.forwarding_enabled(SessionForwardingDirection::Inbound),
            false
        );
        assert_eq!(
            binding.forwarding_enabled(SessionForwardingDirection::Outbound),
            false
        );
    }

    #[test]
    fn connect_session_to_thread_creates_placeholder_and_replaces_existing_thread_binding() {
        let tempdir = tempdir().expect("tempdir");
        let mut runtime = ClawbotRuntime::load(tempdir.path().to_path_buf()).expect("runtime");

        runtime
            .persist_session(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_existing".to_string(),
                display_name: Some("Existing".to_string()),
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })
            .expect("existing session");
        runtime
            .connect_session_to_thread(
                &ProviderSessionRef::new(ProviderKind::Feishu, "chat_existing"),
                "thread_manual".to_string(),
            )
            .expect("connect existing");
        runtime
            .connect_session_to_thread(
                &ProviderSessionRef::new(ProviderKind::Feishu, "chat_manual"),
                "thread_manual".to_string(),
            )
            .expect("connect manual");

        assert_eq!(
            runtime
                .bound_session_for_thread("thread_manual")
                .expect("binding lookup"),
            Some(ProviderSessionRef::new(ProviderKind::Feishu, "chat_manual"))
        );
        assert_eq!(runtime.snapshot().bindings.len(), 1);
        assert_eq!(
            runtime
                .snapshot()
                .sessions
                .iter()
                .find(|session| session.session_id == "chat_existing")
                .expect("existing session persisted")
                .bound_thread_id,
            None
        );
        assert_eq!(
            runtime
                .snapshot()
                .sessions
                .iter()
                .find(|session| session.session_id == "chat_manual")
                .expect("manual session persisted"),
            &ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_manual".to_string(),
                display_name: None,
                unread_count: 0,
                last_message_at: None,
                status: SessionStatus::Bound,
                bound_thread_id: Some("thread_manual".to_string()),
            }
        );
    }

    #[test]
    fn duplicate_inbound_message_is_ignored_after_receipt_is_recorded() {
        let tempdir = tempdir().expect("tempdir");
        let mut runtime = ClawbotRuntime::load(tempdir.path().to_path_buf()).expect("runtime");

        runtime
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session: ProviderSessionRef::new(ProviderKind::Feishu, "chat_dup"),
                message_id: "msg_dup".to_string(),
                text: "hello".to_string(),
                received_at: 20,
            }))
            .expect("first inbound");
        runtime
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session: ProviderSessionRef::new(ProviderKind::Feishu, "chat_dup"),
                message_id: "msg_dup".to_string(),
                text: "hello".to_string(),
                received_at: 20,
            }))
            .expect("duplicate inbound");

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.unread_message_count, 1);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].unread_count, 1);
    }
}
