use codex_clawbot::ClawbotTurnMode;
use codex_protocol::ThreadId;

pub(super) const FEISHU_AUTO_ACK_EMOJI_TYPE: &str = "TONGUE";
pub(super) const FEISHU_AUTO_ACK_DISPLAY: &str = "😛";

#[derive(Debug, Clone)]
pub(crate) struct PendingClawbotTurn {
    pub(crate) turn_id: String,
    pub(crate) thread_id: ThreadId,
    pub(crate) turn_mode: ClawbotTurnMode,
}
