mod feishu;

use crate::events::ProviderInboundMessage;
use crate::model::ProviderRuntimeState;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;

pub use feishu::FeishuProviderRuntime;
pub use feishu::failure_reply_text as feishu_failure_reply_text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOutboundTextMessage {
    pub session: ProviderSessionRef,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    RuntimeStateUpdated(ProviderRuntimeState),
    SessionUpserted(ProviderSession),
    SessionRemoved(ProviderSessionRef),
    InboundMessage(ProviderInboundMessage),
}
