mod feishu;

use anyhow::Result;
use async_trait::async_trait;

use crate::events::ProviderInboundMessage;
use crate::model::ProviderKind;
use crate::model::ProviderMessageRef;
use crate::model::ProviderRuntimeState;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;

pub use feishu::FeishuInboundPrivateMessage;
pub use feishu::FeishuProviderRuntime;
pub use feishu::failure_reply_text as feishu_failure_reply_text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOutboundTextMessage {
    pub session: ProviderSessionRef,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOutboundReaction {
    pub target: ProviderMessageRef,
    pub emoji_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReactionReceipt {
    pub target: ProviderMessageRef,
    pub reaction_id: String,
    pub emoji_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOutboundAction {
    Text(ProviderOutboundTextMessage),
    AddReaction(ProviderOutboundReaction),
    RemoveReaction(ProviderReactionReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    RuntimeStateUpdated(ProviderRuntimeState),
    SessionUpserted(ProviderSession),
    SessionRemoved(ProviderSessionRef),
    InboundMessage(ProviderInboundMessage),
}

#[async_trait]
pub trait ProviderRuntime: Send {
    fn provider(&self) -> ProviderKind;
    fn runtime_state(&self) -> &ProviderRuntimeState;

    async fn connect(&mut self) -> Result<ProviderRuntimeState>;
    async fn disconnect(&mut self) -> Result<ProviderRuntimeState>;
    async fn send_text(&mut self, message: ProviderOutboundTextMessage) -> Result<()>;
    async fn add_reaction(
        &mut self,
        reaction: ProviderOutboundReaction,
    ) -> Result<ProviderReactionReceipt>;
    async fn remove_reaction(&mut self, reaction: ProviderReactionReceipt) -> Result<()>;
}
