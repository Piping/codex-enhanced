use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::ClawbotConfig;
use crate::model::CLAWBOT_BINDINGS_RELATIVE_PATH;
use crate::model::CLAWBOT_CONFIG_RELATIVE_PATH;
use crate::model::CLAWBOT_INBOUND_RECEIPTS_RELATIVE_PATH;
use crate::model::CLAWBOT_RELATIVE_DIR;
use crate::model::CLAWBOT_RUNTIME_RELATIVE_PATH;
use crate::model::CLAWBOT_SESSIONS_RELATIVE_PATH;
use crate::model::CLAWBOT_UNREAD_MESSAGES_RELATIVE_PATH;
use crate::model::CachedUnreadMessage;
use crate::model::ClawbotSnapshot;
use crate::model::ConnectionStatus;
use crate::model::InboundMessageReceipt;
use crate::model::ProviderKind;
use crate::model::ProviderRuntimeState;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;
use crate::model::SessionBinding;

const MAX_INBOUND_RECEIPTS: usize = 4_096;

#[derive(Debug, Clone)]
pub struct ClawbotStore {
    workspace_root: PathBuf,
}

impl ClawbotStore {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn root_dir(&self) -> PathBuf {
        self.workspace_root.join(CLAWBOT_RELATIVE_DIR)
    }

    pub fn config_path(&self) -> PathBuf {
        self.workspace_root.join(CLAWBOT_CONFIG_RELATIVE_PATH)
    }

    pub fn sessions_path(&self) -> PathBuf {
        self.workspace_root.join(CLAWBOT_SESSIONS_RELATIVE_PATH)
    }

    pub fn bindings_path(&self) -> PathBuf {
        self.workspace_root.join(CLAWBOT_BINDINGS_RELATIVE_PATH)
    }

    pub fn unread_messages_path(&self) -> PathBuf {
        self.workspace_root
            .join(CLAWBOT_UNREAD_MESSAGES_RELATIVE_PATH)
    }

    pub fn runtime_path(&self) -> PathBuf {
        self.workspace_root.join(CLAWBOT_RUNTIME_RELATIVE_PATH)
    }

    pub fn inbound_receipts_path(&self) -> PathBuf {
        self.workspace_root
            .join(CLAWBOT_INBOUND_RECEIPTS_RELATIVE_PATH)
    }

    pub fn ensure_root_dir(&self) -> Result<()> {
        fs::create_dir_all(self.root_dir())
            .with_context(|| format!("failed to create {}", self.root_dir().display()))
    }

    pub fn load_snapshot(&self) -> Result<ClawbotSnapshot> {
        let config = self.load_config()?;
        let runtime = self.load_runtime_states_for_config(&config)?;
        let sessions = self.load_sessions()?;
        let bindings = self.load_bindings()?;
        let unread_message_count = self.load_unread_messages()?.len();

        Ok(ClawbotSnapshot {
            config,
            runtime,
            sessions,
            bindings,
            unread_message_count,
        })
    }

    pub fn load_config(&self) -> Result<ClawbotConfig> {
        let config_path = self.config_path();
        if !config_path.exists() {
            return Ok(ClawbotConfig::default());
        }

        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", config_path.display()))
    }

    pub fn save_config(&self, config: &ClawbotConfig) -> Result<()> {
        let rendered = toml::to_string_pretty(config).context("failed to encode clawbot config")?;
        self.write_string_file(&self.config_path(), &rendered)
    }

    pub fn load_runtime_states(&self) -> Result<Vec<ProviderRuntimeState>> {
        let config = self.load_config()?;
        self.load_runtime_states_for_config(&config)
    }

    pub fn load_inbound_receipts(&self) -> Result<Vec<InboundMessageReceipt>> {
        read_optional_json_file(&self.inbound_receipts_path())
            .with_context(|| format!("failed to load {}", self.inbound_receipts_path().display()))
    }

    pub fn save_runtime_states(&self, runtime_states: &[ProviderRuntimeState]) -> Result<()> {
        let mut sorted = runtime_states.to_vec();
        sorted.sort_by_key(|state| state.provider.title());
        self.write_json_file(&self.runtime_path(), &sorted)
    }

    pub fn upsert_runtime_state(
        &self,
        runtime_state: ProviderRuntimeState,
    ) -> Result<Vec<ProviderRuntimeState>> {
        let mut runtime_states = self.load_runtime_states()?;
        if let Some(existing) = runtime_states
            .iter_mut()
            .find(|state| state.provider == runtime_state.provider)
        {
            *existing = runtime_state;
        } else {
            runtime_states.push(runtime_state);
        }
        self.save_runtime_states(&runtime_states)?;
        Ok(runtime_states)
    }

    pub fn load_sessions(&self) -> Result<Vec<ProviderSession>> {
        read_optional_json_file(&self.sessions_path())
            .with_context(|| format!("failed to load {}", self.sessions_path().display()))
    }

    pub fn save_sessions(&self, sessions: &[ProviderSession]) -> Result<()> {
        let mut sorted = sessions.to_vec();
        sorted.sort_by(|left, right| {
            left.provider
                .title()
                .cmp(right.provider.title())
                .then(left.session_id.cmp(&right.session_id))
        });
        self.write_json_file(&self.sessions_path(), &sorted)
    }

    pub fn upsert_session(&self, session: ProviderSession) -> Result<Vec<ProviderSession>> {
        let mut sessions = self.load_sessions()?;
        if let Some(existing) = sessions
            .iter_mut()
            .find(|existing| existing.session_ref() == session.session_ref())
        {
            *existing = session;
        } else {
            sessions.push(session);
        }
        self.save_sessions(&sessions)?;
        Ok(sessions)
    }

    pub fn remove_session(&self, session: &ProviderSessionRef) -> Result<Vec<ProviderSession>> {
        let mut sessions = self.load_sessions()?;
        sessions.retain(|existing| existing.session_ref() != *session);
        self.save_sessions(&sessions)?;
        Ok(sessions)
    }

    pub fn load_bindings(&self) -> Result<Vec<SessionBinding>> {
        read_optional_json_file(&self.bindings_path())
            .with_context(|| format!("failed to load {}", self.bindings_path().display()))
    }

    pub fn save_bindings(&self, bindings: &[SessionBinding]) -> Result<()> {
        let mut sorted = bindings.to_vec();
        sorted.sort_by(|left, right| {
            left.provider
                .title()
                .cmp(right.provider.title())
                .then(left.session_id.cmp(&right.session_id))
                .then(left.thread_id.cmp(&right.thread_id))
        });
        self.write_json_file(&self.bindings_path(), &sorted)
    }

    pub fn upsert_binding(&self, binding: SessionBinding) -> Result<Vec<SessionBinding>> {
        let mut bindings = self.load_bindings()?;
        if let Some(existing) = bindings
            .iter_mut()
            .find(|existing| existing.session_ref() == binding.session_ref())
        {
            *existing = binding;
        } else {
            bindings.push(binding);
        }
        self.save_bindings(&bindings)?;
        Ok(bindings)
    }

    pub fn remove_binding(&self, session: &ProviderSessionRef) -> Result<Vec<SessionBinding>> {
        let mut bindings = self.load_bindings()?;
        bindings.retain(|binding| binding.session_ref() != *session);
        self.save_bindings(&bindings)?;
        Ok(bindings)
    }

    pub fn load_unread_messages(&self) -> Result<Vec<CachedUnreadMessage>> {
        let unread_messages_path = self.unread_messages_path();
        if !unread_messages_path.exists() {
            return Ok(Vec::new());
        }

        let raw = fs::read_to_string(&unread_messages_path)
            .with_context(|| format!("failed to read {}", unread_messages_path.display()))?;
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<CachedUnreadMessage>(line)
                    .with_context(|| format!("failed to parse {}", unread_messages_path.display()))
            })
            .collect()
    }

    pub fn save_unread_messages(&self, unread_messages: &[CachedUnreadMessage]) -> Result<()> {
        let mut sorted = unread_messages.to_vec();
        sorted.sort_by(|left, right| {
            left.provider
                .title()
                .cmp(right.provider.title())
                .then(left.session_id.cmp(&right.session_id))
                .then(left.received_at.cmp(&right.received_at))
                .then(left.message_id.cmp(&right.message_id))
        });
        sorted.dedup_by(|left, right| {
            left.provider == right.provider
                && left.session_id == right.session_id
                && left.message_id == right.message_id
        });

        let rendered = if sorted.is_empty() {
            String::new()
        } else {
            let lines = sorted
                .iter()
                .map(|message| {
                    serde_json::to_string(message).context("failed to encode unread message")
                })
                .collect::<Result<Vec<_>>>()?;
            format!("{}\n", lines.join("\n"))
        };
        self.write_string_file(&self.unread_messages_path(), &rendered)
    }

    pub fn append_unread_message(&self, unread_message: &CachedUnreadMessage) -> Result<()> {
        let mut unread_messages = self.load_unread_messages()?;
        unread_messages.push(unread_message.clone());
        self.save_unread_messages(&unread_messages)
    }

    pub fn has_inbound_receipt(
        &self,
        session: &ProviderSessionRef,
        message_id: &str,
    ) -> Result<bool> {
        Ok(self
            .load_inbound_receipts()?
            .into_iter()
            .any(|receipt| receipt.session_ref() == *session && receipt.message_id == message_id))
    }

    pub fn record_inbound_receipt(&self, receipt: InboundMessageReceipt) -> Result<()> {
        let mut receipts = self.load_inbound_receipts()?;
        receipts.retain(|existing| {
            existing.session_ref() != receipt.session_ref()
                || existing.message_id != receipt.message_id
        });
        receipts.push(receipt);
        receipts.sort_by(|left, right| {
            left.received_at
                .cmp(&right.received_at)
                .then(left.provider.title().cmp(right.provider.title()))
                .then(left.session_id.cmp(&right.session_id))
                .then(left.message_id.cmp(&right.message_id))
        });
        if receipts.len() > MAX_INBOUND_RECEIPTS {
            receipts.drain(..receipts.len().saturating_sub(MAX_INBOUND_RECEIPTS));
        }
        self.write_json_file(&self.inbound_receipts_path(), &receipts)
    }

    pub fn take_unread_messages(
        &self,
        session: &ProviderSessionRef,
    ) -> Result<Vec<CachedUnreadMessage>> {
        let unread_messages = self.load_unread_messages()?;
        let mut taken = Vec::new();
        let mut retained = Vec::new();

        for message in unread_messages {
            if message.session_ref() == *session {
                taken.push(message);
            } else {
                retained.push(message);
            }
        }

        self.save_unread_messages(&retained)?;
        Ok(taken)
    }

    fn load_runtime_states_for_config(
        &self,
        config: &ClawbotConfig,
    ) -> Result<Vec<ProviderRuntimeState>> {
        let mut runtime_states: Vec<ProviderRuntimeState> =
            read_optional_json_file(&self.runtime_path())
                .with_context(|| format!("failed to load {}", self.runtime_path().display()))?;
        if runtime_states.is_empty() {
            runtime_states.push(default_provider_state(config, ProviderKind::Feishu));
        }
        Ok(runtime_states)
    }

    fn write_json_file<T>(&self, path: &Path, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let rendered =
            serde_json::to_string_pretty(value).context("failed to encode clawbot JSON file")?;
        self.write_string_file(path, &rendered)
    }

    fn write_string_file(&self, path: &Path, contents: &str) -> Result<()> {
        self.ensure_root_dir()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let temporary_path = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary_path, contents)
            .with_context(|| format!("failed to write {}", temporary_path.display()))?;
        fs::rename(&temporary_path, path).with_context(|| {
            format!(
                "failed to move {} to {}",
                temporary_path.display(),
                path.display()
            )
        })
    }
}

fn read_optional_json_file<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

fn default_provider_state(config: &ClawbotConfig, provider: ProviderKind) -> ProviderRuntimeState {
    if config.has_provider_config(provider) {
        ProviderRuntimeState {
            provider,
            connection: ConnectionStatus::Disconnected,
            last_error: None,
            updated_at: None,
        }
    } else {
        ProviderRuntimeState::unconfigured(provider)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::ClawbotStore;
    use crate::config::ClawbotConfig;
    use crate::config::FeishuConfig;
    use crate::model::CachedUnreadMessage;
    use crate::model::InboundMessageReceipt;
    use crate::model::ProviderKind;
    use crate::model::ProviderRuntimeState;
    use crate::model::ProviderSession;
    use crate::model::ProviderSessionRef;
    use crate::model::SessionBinding;
    use crate::model::SessionStatus;

    #[test]
    fn save_and_load_snapshot_round_trips_workspace_state() {
        let tempdir = tempdir().expect("tempdir");
        let store = ClawbotStore::new(tempdir.path());

        store
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
        store
            .save_runtime_states(&[ProviderRuntimeState::unconfigured(ProviderKind::Feishu)])
            .expect("runtime");
        store
            .save_sessions(&[ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                display_name: Some("Alice".to_string()),
                unread_count: 2,
                last_message_at: Some(10),
                status: SessionStatus::Bound,
                bound_thread_id: Some("thread_1".to_string()),
            }])
            .expect("sessions");
        store
            .save_bindings(&[SessionBinding {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                thread_id: "thread_1".to_string(),
                inbound_forwarding_enabled: true,
                outbound_forwarding_enabled: true,
                created_at: 1,
                updated_at: 2,
            }])
            .expect("bindings");
        store
            .save_unread_messages(&[
                CachedUnreadMessage {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_1".to_string(),
                    message_id: "msg_1".to_string(),
                    text: "hello".to_string(),
                    received_at: 11,
                },
                CachedUnreadMessage {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_2".to_string(),
                    message_id: "msg_2".to_string(),
                    text: "world".to_string(),
                    received_at: 12,
                },
            ])
            .expect("unread");

        let snapshot = store.load_snapshot().expect("snapshot");
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.bindings.len(), 1);
        assert_eq!(snapshot.unread_message_count, 2);
    }

    #[test]
    fn take_unread_messages_removes_only_target_session() {
        let tempdir = tempdir().expect("tempdir");
        let store = ClawbotStore::new(tempdir.path());

        store
            .save_unread_messages(&[
                CachedUnreadMessage {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_1".to_string(),
                    message_id: "msg_1".to_string(),
                    text: "hello".to_string(),
                    received_at: 11,
                },
                CachedUnreadMessage {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_2".to_string(),
                    message_id: "msg_2".to_string(),
                    text: "world".to_string(),
                    received_at: 12,
                },
            ])
            .expect("unread");

        let taken = store
            .take_unread_messages(&ProviderSessionRef::new(ProviderKind::Feishu, "chat_1"))
            .expect("take unread");
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].message_id, "msg_1");
        assert_eq!(store.load_unread_messages().expect("remaining").len(), 1);
        assert_eq!(
            store.load_unread_messages().expect("remaining")[0].session_id,
            "chat_2"
        );
    }

    #[test]
    fn append_unread_message_dedups_same_message_id() {
        let tempdir = tempdir().expect("tempdir");
        let store = ClawbotStore::new(tempdir.path());
        let unread = CachedUnreadMessage {
            provider: ProviderKind::Feishu,
            session_id: "chat_1".to_string(),
            message_id: "msg_1".to_string(),
            text: "hello".to_string(),
            received_at: 11,
        };

        store.append_unread_message(&unread).expect("append first");
        store
            .append_unread_message(&unread)
            .expect("append duplicate");

        assert_eq!(store.load_unread_messages().expect("unread").len(), 1);
    }

    #[test]
    fn record_inbound_receipt_dedups_and_limits_history() {
        let tempdir = tempdir().expect("tempdir");
        let store = ClawbotStore::new(tempdir.path());

        store
            .record_inbound_receipt(InboundMessageReceipt {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                message_id: "msg_1".to_string(),
                received_at: 11,
            })
            .expect("record first");
        store
            .record_inbound_receipt(InboundMessageReceipt {
                provider: ProviderKind::Feishu,
                session_id: "chat_1".to_string(),
                message_id: "msg_1".to_string(),
                received_at: 12,
            })
            .expect("record duplicate");

        let receipts = store.load_inbound_receipts().expect("receipts");
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].received_at, 12);
        assert!(
            store
                .has_inbound_receipt(
                    &ProviderSessionRef::new(ProviderKind::Feishu, "chat_1"),
                    "msg_1"
                )
                .expect("has receipt")
        );
    }
}
