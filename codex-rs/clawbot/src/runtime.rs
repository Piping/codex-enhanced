use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;

use crate::model::CachedUnreadMessage;
use crate::model::ClawbotSnapshot;
use crate::model::InboundMessageReceipt;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;
use crate::model::SessionBinding;
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

    pub fn persist_session(&mut self, session: ProviderSession) -> Result<&ClawbotSnapshot> {
        self.store.upsert_session(session)?;
        self.reload()
    }

    pub fn persist_binding(&mut self, binding: SessionBinding) -> Result<&ClawbotSnapshot> {
        self.store.upsert_binding(binding)?;
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
                && let Some(existing_session) = sessions
                    .iter_mut()
                    .find(|existing| existing.session_ref() == binding.session_ref())
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
                bound_thread_id: Some(thread_id),
            });
        }

        self.store.save_bindings(&bindings)?;
        self.store.save_sessions(&sessions)?;
        self.reload()
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

    pub fn bound_session_for_thread(&self, thread_id: &str) -> Result<Option<ProviderSessionRef>> {
        Ok(self
            .load_binding_for_thread(thread_id)?
            .as_ref()
            .map(SessionBinding::session_ref))
    }

    pub fn take_next_unread_message(
        &mut self,
        session: &ProviderSessionRef,
    ) -> Result<Option<CachedUnreadMessage>> {
        let message = self.store.take_next_unread_message(session)?;
        if message.is_some()
            && let Some(mut provider_session) = self.load_session(session)?
        {
            provider_session.unread_count = provider_session.unread_count.saturating_sub(1);
            self.store.upsert_session(provider_session)?;
        }
        self.reload()?;
        Ok(message)
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
                    message_id: message.message_id,
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
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::ClawbotRuntime;
    use crate::events::ProviderInboundMessage;
    use crate::model::ConnectionStatus;
    use crate::model::ProviderKind;
    use crate::model::ProviderRuntimeState;
    use crate::model::ProviderSession;
    use crate::model::ProviderSessionRef;
    use crate::model::SessionStatus;
    use crate::provider::ProviderEvent;

    #[test]
    fn take_next_unread_message_is_fifo_per_session() {
        let tempdir = tempdir().expect("tempdir");
        let mut runtime = ClawbotRuntime::load(tempdir.path().to_path_buf()).expect("runtime");
        let session = ProviderSessionRef::new(ProviderKind::Feishu, "chat_1");

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
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session: session.clone(),
                message_id: "msg_2".to_string(),
                text: "second".to_string(),
                received_at: 2,
            }))
            .expect("second");
        runtime
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session: session.clone(),
                message_id: "msg_1".to_string(),
                text: "first".to_string(),
                received_at: 1,
            }))
            .expect("first");

        assert_eq!(
            runtime
                .take_next_unread_message(&session)
                .expect("take first")
                .expect("message")
                .message_id,
            "msg_1"
        );
        assert_eq!(
            runtime
                .take_next_unread_message(&session)
                .expect("take second")
                .expect("message")
                .message_id,
            "msg_2"
        );
        assert_eq!(
            runtime
                .take_next_unread_message(&session)
                .expect("take none"),
            None
        );
    }

    #[test]
    fn apply_provider_event_deduplicates_inbound_messages() {
        let tempdir = tempdir().expect("tempdir");
        let mut runtime = ClawbotRuntime::load(tempdir.path().to_path_buf()).expect("runtime");
        let session = ProviderSessionRef::new(ProviderKind::Feishu, "chat_2");

        runtime
            .apply_provider_event(ProviderEvent::RuntimeStateUpdated(ProviderRuntimeState {
                provider: ProviderKind::Feishu,
                connection: ConnectionStatus::Connected,
                last_error: None,
                updated_at: Some(1),
            }))
            .expect("runtime state");
        runtime
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session: session.clone(),
                message_id: "msg_1".to_string(),
                text: "hello".to_string(),
                received_at: 10,
            }))
            .expect("first inbound");
        runtime
            .apply_provider_event(ProviderEvent::InboundMessage(ProviderInboundMessage {
                session,
                message_id: "msg_1".to_string(),
                text: "hello".to_string(),
                received_at: 10,
            }))
            .expect("duplicate inbound");

        assert_eq!(runtime.snapshot().runtime.len(), 1);
        assert_eq!(runtime.snapshot().unread_message_count, 1);
        assert_eq!(runtime.snapshot().sessions.len(), 1);
        assert_eq!(runtime.snapshot().sessions[0].unread_count, 1);
    }
}
