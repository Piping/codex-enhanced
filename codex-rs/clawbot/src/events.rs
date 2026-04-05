use serde::Deserialize;
use serde::Serialize;

use crate::model::ProviderSessionRef;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderInboundMessage {
    pub session: ProviderSessionRef,
    pub message_id: String,
    pub text: String,
    pub received_at: i64,
}
