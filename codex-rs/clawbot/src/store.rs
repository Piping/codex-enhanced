use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::Value as TomlValue;

use crate::config::ClawbotConfig;
use crate::model::CLAWBOT_BINDINGS_RELATIVE_PATH;
use crate::model::CLAWBOT_CONFIG_RELATIVE_PATH;
use crate::model::CLAWBOT_INBOUND_RECEIPTS_RELATIVE_PATH;
use crate::model::CLAWBOT_PENDING_TURNS_RELATIVE_PATH;
use crate::model::CLAWBOT_RELATIVE_DIR;
use crate::model::CLAWBOT_RUNTIME_RELATIVE_PATH;
use crate::model::CLAWBOT_SESSIONS_RELATIVE_PATH;
use crate::model::CLAWBOT_UNREAD_MESSAGES_RELATIVE_PATH;
use crate::model::CachedUnreadMessage;
use crate::model::ClawbotSnapshot;
use crate::model::InboundMessageReceipt;
use crate::model::PendingClawbotTurn;
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

    pub fn pending_turns_path(&self) -> PathBuf {
        self.workspace_root
            .join(CLAWBOT_PENDING_TURNS_RELATIVE_PATH)
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
        let mut runtime = self.load_runtime_states()?;
        if config.feishu.is_none()
            && runtime
                .iter()
                .all(|state| state.provider != ProviderKind::Feishu)
        {
            runtime.push(ProviderRuntimeState::unconfigured(ProviderKind::Feishu));
        }
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
        let rendered = toml::to_string_pretty(config).context("failed to encode config")?;
        let contents = if rendered.trim().is_empty() {
            String::new()
        } else {
            let normalized = rendered
                .parse::<TomlValue>()
                .ok()
                .and_then(|value| toml::to_string_pretty(&value).ok())
                .unwrap_or(rendered);
            format!("{normalized}\n")
        };
        self.write_string_file(&self.config_path(), &contents)
    }

    pub fn load_runtime_states(&self) -> Result<Vec<ProviderRuntimeState>> {
        read_optional_json_file(&self.runtime_path())
            .with_context(|| format!("failed to load {}", self.runtime_path().display()))
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
                .map(|message| serde_json::to_string(message).context("failed to encode unread"))
                .collect::<Result<Vec<_>>>()?;
            format!("{}\n", lines.join("\n"))
        };
        self.write_string_file(&self.unread_messages_path(), &rendered)
    }

    pub fn append_unread_message(&self, message: &CachedUnreadMessage) -> Result<()> {
        let mut unread_messages = self.load_unread_messages()?;
        unread_messages.push(message.clone());
        self.save_unread_messages(&unread_messages)
    }

    pub fn take_next_unread_message(
        &self,
        session: &ProviderSessionRef,
    ) -> Result<Option<CachedUnreadMessage>> {
        let mut unread_messages = self.load_unread_messages()?;
        let Some(index) = unread_messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message.session_ref() == *session)
            .min_by(|(_, left), (_, right)| {
                left.received_at
                    .cmp(&right.received_at)
                    .then(left.message_id.cmp(&right.message_id))
            })
            .map(|(index, _)| index)
        else {
            return Ok(None);
        };
        let message = unread_messages.remove(index);
        self.save_unread_messages(&unread_messages)?;
        Ok(Some(message))
    }

    pub fn load_pending_turns(&self) -> Result<Vec<PendingClawbotTurn>> {
        read_optional_json_file(&self.pending_turns_path())
            .with_context(|| format!("failed to load {}", self.pending_turns_path().display()))
    }

    pub fn save_pending_turns(&self, pending_turns: &[PendingClawbotTurn]) -> Result<()> {
        let mut sorted = pending_turns.to_vec();
        sorted.sort_by(|left, right| {
            left.thread_id
                .cmp(&right.thread_id)
                .then(left.turn_id.cmp(&right.turn_id))
                .then(left.session.session_id.cmp(&right.session.session_id))
                .then(left.message_id.cmp(&right.message_id))
        });
        sorted.dedup_by(|left, right| {
            left.thread_id == right.thread_id && left.turn_id == right.turn_id
        });
        self.write_json_file(&self.pending_turns_path(), &sorted)
    }

    pub fn upsert_pending_turn(&self, pending_turn: PendingClawbotTurn) -> Result<()> {
        let mut pending_turns = self.load_pending_turns()?;
        pending_turns.retain(|existing| {
            existing.thread_id != pending_turn.thread_id || existing.turn_id != pending_turn.turn_id
        });
        pending_turns.push(pending_turn);
        self.save_pending_turns(&pending_turns)
    }

    pub fn remove_pending_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Option<PendingClawbotTurn>> {
        let mut pending_turns = self.load_pending_turns()?;
        let Some(index) = pending_turns
            .iter()
            .position(|pending| pending.thread_id == thread_id && pending.turn_id == turn_id)
        else {
            return Ok(None);
        };
        let pending_turn = pending_turns.remove(index);
        self.save_pending_turns(&pending_turns)?;
        Ok(Some(pending_turn))
    }

    pub fn load_inbound_receipts(&self) -> Result<Vec<InboundMessageReceipt>> {
        read_optional_json_file(&self.inbound_receipts_path())
            .with_context(|| format!("failed to load {}", self.inbound_receipts_path().display()))
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
                .then(left.session_id.cmp(&right.session_id))
                .then(left.message_id.cmp(&right.message_id))
        });
        if receipts.len() > MAX_INBOUND_RECEIPTS {
            receipts.drain(..receipts.len() - MAX_INBOUND_RECEIPTS);
        }
        self.write_json_file(&self.inbound_receipts_path(), &receipts)
    }

    fn write_json_file<T>(&self, path: &Path, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let rendered = serde_json::to_string_pretty(value).context("failed to encode json")?;
        self.write_string_file(path, &format!("{rendered}\n"))
    }

    fn write_string_file(&self, path: &Path, contents: &str) -> Result<()> {
        self.ensure_root_dir()?;
        fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
    }
}

fn read_optional_json_file<T>(path: &Path) -> Result<Vec<T>>
where
    T: DeserializeOwned,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}
