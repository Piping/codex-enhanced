mod coordination;
mod runtime_loop;
mod sync;

use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use anyhow::anyhow;
use open_lark::openlark_client;
use open_lark::openlark_communication::common::api_utils::serialize_params;
use open_lark::openlark_communication::endpoints::IM_V1_MESSAGES;
use open_lark::openlark_communication::im::im::v1::message::create::CreateMessageBody;
use open_lark::openlark_communication::im::im::v1::message::create::CreateMessageRequest;
use open_lark::openlark_communication::im::im::v1::message::models::ReceiveIdType;
use open_lark::openlark_communication::im::im::v1::message::models::UserIdType;
use open_lark::openlark_communication::im::im::v1::message::reaction::list::ListMessageReactionsRequest;
use open_lark::openlark_communication::im::im::v1::message::reaction::models::CreateMessageReactionBody;
use open_lark::openlark_communication::im::im::v1::message::reaction::models::MessageReaction;
use open_lark::openlark_communication::im::im::v1::message::reaction::models::ReactionType;
use open_lark::openlark_core::api::ApiRequest;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;

use super::ProviderEvent;
use super::ProviderOutboundReaction;
use super::ProviderOutboundTextMessage;
use crate::append_diagnostic_event;
use crate::config::FeishuConfig;
use crate::events::ProviderInboundMessage;
use crate::model::ConnectionStatus;
use crate::model::ProviderKind;
use crate::model::ProviderRuntimeState;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;
use crate::model::SessionStatus;

#[derive(Debug, Clone)]
pub struct FeishuInboundMessage {
    pub chat_id: String,
    pub chat_type: String,
    pub chat_name: Option<String>,
    pub message_id: String,
    pub sender_open_id: Option<String>,
    pub sender_user_id: Option<String>,
    pub sender_union_id: Option<String>,
    pub text: String,
    pub received_at: i64,
}

#[derive(Debug, Clone)]
pub struct FeishuProviderRuntime {
    workspace_root: PathBuf,
    config: FeishuConfig,
}

impl FeishuProviderRuntime {
    pub fn new(workspace_root: impl Into<PathBuf>, config: FeishuConfig) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            config,
        }
    }

    pub async fn run(self, provider_event_tx: mpsc::UnboundedSender<ProviderEvent>) -> Result<()> {
        runtime_loop::run_with_reconnect(self.workspace_root, self.config, provider_event_tx).await
    }

    pub async fn send_text(&self, message: ProviderOutboundTextMessage) -> Result<()> {
        if message.session.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot send {} message via Feishu runtime",
                message.session.provider.title()
            ));
        }

        let text = message.text;
        let session_id = message.session.session_id;
        let response = CreateMessageRequest::new(self.messaging_config()?)
            .receive_id_type(ReceiveIdType::ChatId)
            .execute(CreateMessageBody {
                receive_id: session_id.clone(),
                msg_type: "text".to_string(),
                content: serde_json::to_string(&serde_json::json!({ "text": text.clone() }))?,
                uuid: None,
            })
            .await
            .map_err(|error| anyhow!("failed to send Feishu text message: {error}"));
        match response {
            Ok(_) => {
                let _ = append_diagnostic_event(
                    self.workspace_root.as_path(),
                    "feishu.send_text_succeeded",
                    serde_json::json!({
                        "session_id": session_id,
                        "text": text,
                    }),
                );
                Ok(())
            }
            Err(error) => {
                let _ = append_diagnostic_event(
                    self.workspace_root.as_path(),
                    "feishu.send_text_failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "text": text,
                        "error": error.to_string(),
                    }),
                );
                Err(error)
            }
        }
    }

    pub async fn add_reaction(&self, reaction: ProviderOutboundReaction) -> Result<Option<String>> {
        if reaction.target.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot send {} reaction via Feishu runtime",
                reaction.target.provider.title()
            ));
        }

        let session_id = reaction.target.session_id.clone();
        let message_id = reaction.target.message_id.clone();
        let emoji_type = reaction.emoji_type.clone();
        let request: ApiRequest<Value> = ApiRequest::post(format!(
            "{IM_V1_MESSAGES}/{}/reactions",
            reaction.target.message_id
        ))
        .body(serialize_params(
            &CreateMessageReactionBody {
                reaction_type: ReactionType {
                    emoji_type: reaction.emoji_type,
                },
            },
            "添加消息表情回复",
        )?);
        let response = open_lark::openlark_core::http::Transport::<Value>::request(
            request,
            &self.messaging_config()?,
            Some(Default::default()),
        )
        .await
        .map_err(|error| anyhow!("failed to add Feishu message reaction: {error}"))?;
        if response.is_success() {
            let reaction_id = response
                .data
                .as_ref()
                .and_then(|data| data.get("reaction_id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let _ = append_diagnostic_event(
                self.workspace_root.as_path(),
                "feishu.add_reaction_succeeded",
                serde_json::json!({
                    "session_id": session_id,
                    "message_id": message_id,
                    "emoji_type": emoji_type,
                    "reaction_id": reaction_id,
                }),
            );
            Ok(reaction_id)
        } else {
            let error = anyhow!("failed to add Feishu message reaction: {}", response.msg());
            let _ = append_diagnostic_event(
                self.workspace_root.as_path(),
                "feishu.add_reaction_failed",
                serde_json::json!({
                    "session_id": session_id,
                    "message_id": message_id,
                    "emoji_type": emoji_type,
                    "error": error.to_string(),
                }),
            );
            Err(error)
        }
    }

    pub async fn remove_reaction(&self, reaction: ProviderOutboundReaction) -> Result<()> {
        if reaction.target.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot send {} reaction via Feishu runtime",
                reaction.target.provider.title()
            ));
        }

        let config = self.messaging_config()?;
        let matching_reactions = self
            .list_message_reactions(
                &config,
                reaction.target.message_id.as_str(),
                reaction.emoji_type.as_str(),
            )
            .await?;
        let reaction_ids = self.select_reaction_ids_to_remove(&reaction, &matching_reactions);

        if reaction_ids.is_empty() {
            let _ = append_diagnostic_event(
                self.workspace_root.as_path(),
                "feishu.remove_reaction_skipped",
                serde_json::json!({
                    "session_id": reaction.target.session_id,
                    "message_id": reaction.target.message_id,
                    "emoji_type": reaction.emoji_type,
                    "bot_open_id": self.config.bot_open_id,
                    "bot_user_id": self.config.bot_user_id,
                    "matching_reaction_count": matching_reactions.len(),
                    "matching_reactions": matching_reactions
                        .iter()
                        .map(|item| serde_json::json!({
                            "reaction_id": item.reaction_id,
                            "operator_id": item.operator.operator_id,
                            "operator_type": item.operator.operator_type,
                            "emoji_type": item.reaction_type.emoji_type,
                        }))
                        .collect::<Vec<_>>(),
                }),
            );
            return Ok(());
        }

        for reaction_id in &reaction_ids {
            self.remove_reaction_by_id(reaction.clone(), reaction_id)
                .await?;
        }

        let _ = append_diagnostic_event(
            self.workspace_root.as_path(),
            "feishu.remove_reaction_succeeded",
            serde_json::json!({
                "session_id": reaction.target.session_id,
                "message_id": reaction.target.message_id,
                "emoji_type": reaction.emoji_type,
                "removed_count": reaction_ids.len(),
                "remove_mode": "listed",
            }),
        );
        Ok(())
    }

    pub async fn remove_reaction_by_id(
        &self,
        reaction: ProviderOutboundReaction,
        reaction_id: &str,
    ) -> Result<()> {
        if reaction.target.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot send {} reaction via Feishu runtime",
                reaction.target.provider.title()
            ));
        }

        let request: ApiRequest<Value> = ApiRequest::delete(format!(
            "{}/{}/reactions/{}",
            IM_V1_MESSAGES, reaction.target.message_id, reaction_id
        ));
        let response = open_lark::openlark_core::http::Transport::<Value>::request(
            request,
            &self.messaging_config()?,
            Some(Default::default()),
        )
        .await
        .map_err(|error| anyhow!("failed to remove Feishu message reaction: {error}"))?;
        if !response.is_success() {
            return Err(anyhow!(
                "failed to remove Feishu message reaction: {}",
                response.msg()
            ));
        }

        let _ = append_diagnostic_event(
            self.workspace_root.as_path(),
            "feishu.remove_reaction_succeeded",
            serde_json::json!({
                "session_id": reaction.target.session_id,
                "message_id": reaction.target.message_id,
                "emoji_type": reaction.emoji_type,
                "removed_count": 1,
                "reaction_id": reaction_id,
                "remove_mode": "direct",
            }),
        );
        Ok(())
    }

    pub async fn scan_sessions(&self) -> Result<Vec<ProviderEvent>> {
        let sessions = sync::discover_supported_sessions(&self.messaging_config()?).await?;
        Ok(sessions
            .into_iter()
            .map(ProviderEvent::SessionUpserted)
            .collect())
    }

    pub fn normalize_chat_message(message: FeishuInboundMessage) -> Option<Vec<ProviderEvent>> {
        if !is_supported_chat_type(&message.chat_type) || message.text.trim().is_empty() {
            return None;
        }

        let display_name = if is_group_chat_type(&message.chat_type) {
            message.chat_name
        } else {
            message
                .chat_name
                .or(message.sender_open_id.clone())
                .or(message.sender_user_id.clone())
                .or(message.sender_union_id.clone())
        };
        let session = ProviderSession {
            provider: ProviderKind::Feishu,
            session_id: message.chat_id.clone(),
            display_name,
            unread_count: 0,
            last_message_at: Some(message.received_at),
            status: SessionStatus::Discovered,
            bound_thread_id: None,
        };
        let inbound_message = ProviderInboundMessage {
            session: ProviderSessionRef::new(ProviderKind::Feishu, message.chat_id),
            message_id: message.message_id,
            text: message.text,
            received_at: message.received_at,
        };

        Some(vec![
            ProviderEvent::SessionUpserted(session),
            ProviderEvent::InboundMessage(inbound_message),
        ])
    }

    pub(super) fn websocket_config(&self) -> Result<openlark_client::Config> {
        runtime_loop::build_websocket_config(&self.config)
    }

    fn messaging_config(&self) -> Result<open_lark::openlark_core::config::Config> {
        Ok(self
            .websocket_config()?
            .build_core_config_with_token_provider())
    }

    async fn list_message_reactions(
        &self,
        config: &open_lark::openlark_core::config::Config,
        message_id: &str,
        emoji_type: &str,
    ) -> Result<Vec<MessageReaction>> {
        let mut page_token = None;
        let mut reactions = Vec::new();

        loop {
            let mut request = ListMessageReactionsRequest::new(config.clone())
                .message_id(message_id.to_string())
                .reaction_type(emoji_type.to_string())
                .page_size(50)
                .user_id_type(UserIdType::OpenId);
            if let Some(token) = page_token.clone() {
                request = request.page_token(token);
            }
            let response = request
                .execute()
                .await
                .map_err(|error| anyhow!("failed to list Feishu message reactions: {error}"))?;
            reactions.extend(response.items.unwrap_or_default());
            if !response.has_more {
                break;
            }
            let Some(token) = response.page_token.filter(|token| !token.is_empty()) else {
                break;
            };
            page_token = Some(token);
        }

        Ok(reactions)
    }

    fn select_reaction_ids_to_remove(
        &self,
        reaction: &ProviderOutboundReaction,
        matching_reactions: &[MessageReaction],
    ) -> Vec<String> {
        let bot_open_id = self.config.bot_open_id.as_deref();
        let bot_user_id = self.config.bot_user_id.as_deref();
        let exact_matches = matching_reactions
            .iter()
            .filter(|item| {
                item.reaction_type.emoji_type == reaction.emoji_type
                    && (bot_open_id == Some(item.operator.operator_id.as_str())
                        || bot_user_id == Some(item.operator.operator_id.as_str()))
            })
            .map(|item| item.reaction_id.clone())
            .collect::<Vec<_>>();
        if !exact_matches.is_empty() {
            return exact_matches;
        }

        let fallback_matches = matching_reactions
            .iter()
            .filter(|item| item.reaction_type.emoji_type == reaction.emoji_type)
            .map(|item| item.reaction_id.clone())
            .collect::<Vec<_>>();
        if fallback_matches.len() == 1 {
            return fallback_matches;
        }

        Vec::new()
    }
}

pub fn failure_reply_text(message: &str) -> String {
    let summary = message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error");
    let truncated = truncate_chars(summary, /*max_chars*/ 160);
    format!("Request failed: {truncated}")
}

pub(super) fn provider_events_from_payload(
    payload: &[u8],
    config: &FeishuConfig,
    workspace_root: &Path,
) -> Vec<ProviderEvent> {
    let Ok(envelope) = serde_json::from_slice::<FeishuEventEnvelope>(payload) else {
        let _ = append_diagnostic_event(
            workspace_root,
            "feishu.payload_parse_failed",
            serde_json::json!({
                "payload": String::from_utf8_lossy(payload),
            }),
        );
        return Vec::new();
    };

    match envelope.header.event_type.as_str() {
        "im.message.receive_v1" => {
            serde_json::from_value::<FeishuMessageReceiveEvent>(envelope.event)
                .ok()
                .and_then(|event| {
                    normalize_message_receive_event(
                        FeishuMessageReceiveEnvelope { event },
                        config,
                        workspace_root,
                    )
                })
                .unwrap_or_default()
        }
        "im.chat.access_event.bot_p2p_chat_entered_v1" => {
            serde_json::from_value::<FeishuChatEnteredEvent>(envelope.event)
                .ok()
                .map(|event| normalize_chat_entered_event(FeishuChatEnteredEnvelope { event }))
                .unwrap_or_default()
        }
        _ => {
            let _ = append_diagnostic_event(
                workspace_root,
                "feishu.unsupported_event",
                serde_json::json!({
                    "event_type": envelope.header.event_type,
                }),
            );
            Vec::new()
        }
    }
}

fn normalize_message_receive_event(
    envelope: FeishuMessageReceiveEnvelope,
    config: &FeishuConfig,
    workspace_root: &Path,
) -> Option<Vec<ProviderEvent>> {
    let chat = envelope.event.chat;
    let message = envelope.event.message;
    let chat_type = message
        .chat_type
        .clone()
        .or(chat.as_ref().and_then(|chat| chat.chat_type.clone()));
    let message_type = message
        .message_type
        .as_deref()
        .or(message.msg_type.as_deref());
    let sender = envelope.event.sender.sender_id;
    let Some(chat_type) = chat_type else {
        let _ = append_diagnostic_event(
            workspace_root,
            "feishu.message_dropped",
            serde_json::json!({
                "reason": "missing_chat_type",
                "message_id": message.message_id,
            }),
        );
        return None;
    };
    let Some(message_type) = message_type else {
        let _ = append_diagnostic_event(
            workspace_root,
            "feishu.message_dropped",
            serde_json::json!({
                "reason": "missing_message_type",
                "chat_id": message.chat_id,
                "message_id": message.message_id,
                "chat_type": chat_type,
            }),
        );
        return None;
    };
    if !is_supported_chat_type(&chat_type) {
        let _ = append_diagnostic_event(
            workspace_root,
            "feishu.message_dropped",
            serde_json::json!({
                "reason": "unsupported_chat_type",
                "chat_id": message.chat_id,
                "message_id": message.message_id,
                "chat_type": chat_type,
            }),
        );
        return None;
    }
    if message_type != "text" {
        let _ = append_diagnostic_event(
            workspace_root,
            "feishu.message_dropped",
            serde_json::json!({
                "reason": "unsupported_message_type",
                "chat_id": message.chat_id,
                "message_id": message.message_id,
                "chat_type": chat_type,
                "message_type": message_type,
            }),
        );
        return None;
    }
    if config.is_bot_sender(
        sender.open_id.as_deref(),
        sender.user_id.as_deref(),
        sender.app_id.as_deref(),
    ) {
        let _ = append_diagnostic_event(
            workspace_root,
            "feishu.message_dropped",
            serde_json::json!({
                "reason": "bot_sender",
                "chat_id": message.chat_id,
                "message_id": message.message_id,
                "chat_type": chat_type,
                "sender_open_id": sender.open_id,
                "sender_user_id": sender.user_id,
                "sender_app_id": sender.app_id,
            }),
        );
        return None;
    }

    let chat_id = chat
        .as_ref()
        .map(|chat| chat.chat_id.clone())
        .or(message.chat_id.clone())?;
    let raw_content = message
        .content
        .or(message.body.and_then(|body| body.content))
        .unwrap_or_default();
    let text = serde_json::from_str::<FeishuTextContent>(&raw_content)
        .ok()
        .map(|content| content.text)
        .unwrap_or(raw_content);
    let normalized_text = if is_group_chat_type(&chat_type) {
        strip_group_mention_prefix(&text)
    } else {
        text
    };
    let received_at = parse_timestamp_value(message.create_time)?;
    let _ = append_diagnostic_event(
        workspace_root,
        "feishu.message_normalized",
        serde_json::json!({
            "chat_id": chat_id.clone(),
            "chat_type": chat_type.clone(),
            "message_id": message.message_id.clone(),
            "sender_open_id": sender.open_id.clone(),
            "sender_user_id": sender.user_id.clone(),
            "sender_app_id": sender.app_id.clone(),
            "text": normalized_text.clone(),
        }),
    );

    FeishuProviderRuntime::normalize_chat_message(FeishuInboundMessage {
        chat_id,
        chat_type,
        chat_name: chat.and_then(|chat| chat.name),
        message_id: message.message_id,
        sender_open_id: sender.open_id,
        sender_user_id: sender.user_id,
        sender_union_id: sender.union_id,
        text: normalized_text,
        received_at,
    })
}

fn normalize_chat_entered_event(envelope: FeishuChatEnteredEnvelope) -> Vec<ProviderEvent> {
    let operator = envelope.event.operator_id;
    vec![ProviderEvent::SessionUpserted(ProviderSession {
        provider: ProviderKind::Feishu,
        session_id: envelope.event.chat_id,
        display_name: operator
            .open_id
            .clone()
            .or(operator.user_id.clone())
            .or(operator.union_id),
        unread_count: 0,
        last_message_at: parse_optional_timestamp(envelope.event.last_message_create_time),
        status: SessionStatus::Discovered,
        bound_thread_id: None,
    })]
}

fn parse_optional_timestamp(timestamp: Option<String>) -> Option<i64> {
    timestamp.and_then(|value| value.parse::<i64>().ok())
}

fn parse_timestamp_value(timestamp: serde_json::Value) -> Option<i64> {
    match timestamp {
        serde_json::Value::String(value) => parse_optional_timestamp(Some(value)),
        serde_json::Value::Number(value) => value.as_i64(),
        _ => None,
    }
}

fn is_supported_chat_type(chat_type: &str) -> bool {
    is_private_chat_type(chat_type) || is_group_chat_type(chat_type)
}

fn is_private_chat_type(chat_type: &str) -> bool {
    matches!(chat_type, "p2p" | "private")
}

fn is_group_chat_type(chat_type: &str) -> bool {
    matches!(chat_type, "group")
}

fn strip_group_mention_prefix(text: &str) -> String {
    let mut remaining = text.trim_start();
    let mut stripped = false;

    loop {
        let Some(after_at) = remaining.strip_prefix('@') else {
            break;
        };
        let mention_len = after_at
            .char_indices()
            .find_map(|(idx, ch)| {
                (ch.is_whitespace() || matches!(ch, ':' | '：' | ',' | '，')).then_some(idx)
            })
            .unwrap_or(after_at.len());
        if mention_len == 0 {
            break;
        }
        remaining = &after_at[mention_len..];
        remaining = remaining.trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, ':' | '：' | ',' | '，')
        });
        stripped = true;
    }

    if stripped {
        remaining.to_string()
    } else {
        text.to_string()
    }
}

pub(super) fn runtime_state(
    connection: ConnectionStatus,
    last_error: Option<String>,
) -> Result<ProviderRuntimeState> {
    Ok(ProviderRuntimeState {
        provider: ProviderKind::Feishu,
        connection,
        last_error,
        updated_at: Some(unix_timestamp_now()?),
    })
}

fn unix_timestamp_now() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

#[derive(Debug, Deserialize)]
struct FeishuEventEnvelope {
    header: FeishuEventHeader,
    event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct FeishuEventHeader {
    event_type: String,
}

#[derive(Debug, Deserialize)]
struct FeishuMessageReceiveEnvelope {
    event: FeishuMessageReceiveEvent,
}

#[derive(Debug, Deserialize)]
struct FeishuMessageReceiveEvent {
    sender: FeishuEventSender,
    message: FeishuEventMessage,
    #[serde(default)]
    chat: Option<FeishuEventChat>,
}

#[derive(Debug, Deserialize)]
struct FeishuEventSender {
    sender_id: FeishuUserId,
}

#[derive(Debug, Deserialize)]
struct FeishuUserId {
    open_id: Option<String>,
    user_id: Option<String>,
    union_id: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuEventMessage {
    message_id: String,
    create_time: serde_json::Value,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    chat_type: Option<String>,
    #[serde(default)]
    message_type: Option<String>,
    #[serde(default)]
    msg_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    body: Option<FeishuEventMessageBody>,
}

#[derive(Debug, Deserialize)]
struct FeishuEventChat {
    chat_id: String,
    #[serde(default)]
    chat_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuTextContent {
    text: String,
}

#[derive(Debug, Deserialize)]
struct FeishuEventMessageBody {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuChatEnteredEnvelope {
    event: FeishuChatEnteredEvent,
}

#[derive(Debug, Deserialize)]
struct FeishuChatEnteredEvent {
    chat_id: String,
    operator_id: FeishuUserId,
    last_message_create_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::FeishuInboundMessage;
    use super::failure_reply_text;
    use super::normalize_chat_entered_event;
    use super::normalize_message_receive_event;
    use super::strip_group_mention_prefix;
    use crate::config::FeishuConfig;
    use crate::model::ProviderKind;
    use crate::model::ProviderSession;
    use crate::model::ProviderSessionRef;
    use crate::model::SessionStatus;
    use crate::provider::ProviderEvent;

    #[test]
    fn normalize_chat_message_creates_session_and_inbound_events() {
        let events = super::FeishuProviderRuntime::normalize_chat_message(FeishuInboundMessage {
            chat_id: "chat_123".to_string(),
            chat_type: "p2p".to_string(),
            chat_name: Some("Alice".to_string()),
            message_id: "msg_123".to_string(),
            sender_open_id: Some("ou_123".to_string()),
            sender_user_id: None,
            sender_union_id: None,
            text: "hello".to_string(),
            received_at: 123,
        })
        .expect("events");

        assert_eq!(
            events,
            vec![
                ProviderEvent::SessionUpserted(ProviderSession {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_123".to_string(),
                    display_name: Some("Alice".to_string()),
                    unread_count: 0,
                    last_message_at: Some(123),
                    status: SessionStatus::Discovered,
                    bound_thread_id: None,
                }),
                ProviderEvent::InboundMessage(crate::events::ProviderInboundMessage {
                    session: ProviderSessionRef::new(ProviderKind::Feishu, "chat_123"),
                    message_id: "msg_123".to_string(),
                    text: "hello".to_string(),
                    received_at: 123,
                }),
            ]
        );
    }

    #[test]
    fn message_receive_event_skips_non_text_messages() {
        let envelope = super::FeishuMessageReceiveEnvelope {
            event: super::FeishuMessageReceiveEvent {
                sender: super::FeishuEventSender {
                    sender_id: super::FeishuUserId {
                        open_id: Some("ou_123".to_string()),
                        user_id: None,
                        union_id: None,
                        app_id: None,
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_123".to_string(),
                    create_time: serde_json::json!("456"),
                    chat_id: Some("chat_123".to_string()),
                    chat_type: Some("p2p".to_string()),
                    message_type: Some("image".to_string()),
                    msg_type: None,
                    content: Some("{}".to_string()),
                    body: None,
                },
                chat: None,
            },
        };

        assert_eq!(
            normalize_message_receive_event(envelope, &FeishuConfig::default(), Path::new("/tmp")),
            None
        );
    }

    #[test]
    fn message_receive_event_accepts_group_text_messages() {
        let envelope = super::FeishuMessageReceiveEnvelope {
            event: super::FeishuMessageReceiveEvent {
                sender: super::FeishuEventSender {
                    sender_id: super::FeishuUserId {
                        open_id: Some("ou_member".to_string()),
                        user_id: None,
                        union_id: None,
                        app_id: None,
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_group_1".to_string(),
                    create_time: serde_json::json!("456"),
                    chat_id: Some("chat_group_123".to_string()),
                    chat_type: Some("group".to_string()),
                    message_type: Some("text".to_string()),
                    msg_type: None,
                    content: Some("{\"text\":\"hello group\"}".to_string()),
                    body: None,
                },
                chat: Some(super::FeishuEventChat {
                    chat_id: "chat_group_123".to_string(),
                    chat_type: Some("group".to_string()),
                    name: Some("tracker".to_string()),
                }),
            },
        };

        assert_eq!(
            normalize_message_receive_event(envelope, &FeishuConfig::default(), Path::new("/tmp")),
            Some(vec![
                ProviderEvent::SessionUpserted(ProviderSession {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_group_123".to_string(),
                    display_name: Some("tracker".to_string()),
                    unread_count: 0,
                    last_message_at: Some(456),
                    status: SessionStatus::Discovered,
                    bound_thread_id: None,
                }),
                ProviderEvent::InboundMessage(crate::events::ProviderInboundMessage {
                    session: ProviderSessionRef::new(ProviderKind::Feishu, "chat_group_123"),
                    message_id: "msg_group_1".to_string(),
                    text: "hello group".to_string(),
                    received_at: 456,
                }),
            ])
        );
    }

    #[test]
    fn message_receive_event_skips_bot_authored_messages() {
        let envelope = super::FeishuMessageReceiveEnvelope {
            event: super::FeishuMessageReceiveEvent {
                sender: super::FeishuEventSender {
                    sender_id: super::FeishuUserId {
                        open_id: Some("ou_bot".to_string()),
                        user_id: None,
                        union_id: None,
                        app_id: None,
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_bot_1".to_string(),
                    create_time: serde_json::json!("456"),
                    chat_id: Some("chat_group_123".to_string()),
                    chat_type: Some("group".to_string()),
                    message_type: Some("text".to_string()),
                    msg_type: None,
                    content: Some("{\"text\":\"hello group\"}".to_string()),
                    body: None,
                },
                chat: Some(super::FeishuEventChat {
                    chat_id: "chat_group_123".to_string(),
                    chat_type: Some("group".to_string()),
                    name: Some("tracker".to_string()),
                }),
            },
        };

        assert_eq!(
            normalize_message_receive_event(
                envelope,
                &FeishuConfig {
                    bot_open_id: Some("ou_bot".to_string()),
                    ..FeishuConfig::default()
                },
                Path::new("/tmp"),
            ),
            None
        );
    }

    #[test]
    fn message_receive_event_skips_app_authored_messages() {
        let envelope = super::FeishuMessageReceiveEnvelope {
            event: super::FeishuMessageReceiveEvent {
                sender: super::FeishuEventSender {
                    sender_id: super::FeishuUserId {
                        open_id: None,
                        user_id: None,
                        union_id: None,
                        app_id: Some("cli_app_123".to_string()),
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_bot_app_1".to_string(),
                    create_time: serde_json::json!("456"),
                    chat_id: Some("chat_group_123".to_string()),
                    chat_type: Some("group".to_string()),
                    message_type: Some("text".to_string()),
                    msg_type: None,
                    content: Some("{\"text\":\"hello group\"}".to_string()),
                    body: None,
                },
                chat: Some(super::FeishuEventChat {
                    chat_id: "chat_group_123".to_string(),
                    chat_type: Some("group".to_string()),
                    name: Some("tracker".to_string()),
                }),
            },
        };

        assert_eq!(
            normalize_message_receive_event(
                envelope,
                &FeishuConfig {
                    app_id: "cli_app_123".to_string(),
                    ..FeishuConfig::default()
                },
                Path::new("/tmp"),
            ),
            None
        );
    }

    #[test]
    fn message_receive_event_uses_chat_fallbacks_for_group_payloads() {
        let envelope = super::FeishuMessageReceiveEnvelope {
            event: super::FeishuMessageReceiveEvent {
                sender: super::FeishuEventSender {
                    sender_id: super::FeishuUserId {
                        open_id: Some("ou_member".to_string()),
                        user_id: None,
                        union_id: None,
                        app_id: None,
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_group_fallback_1".to_string(),
                    create_time: serde_json::json!(456),
                    chat_id: None,
                    chat_type: None,
                    message_type: None,
                    msg_type: Some("text".to_string()),
                    content: None,
                    body: Some(super::FeishuEventMessageBody {
                        content: Some("{\"text\":\"hello fallback\"}".to_string()),
                    }),
                },
                chat: Some(super::FeishuEventChat {
                    chat_id: "chat_group_fallback_123".to_string(),
                    chat_type: Some("group".to_string()),
                    name: Some("tracker".to_string()),
                }),
            },
        };

        assert_eq!(
            normalize_message_receive_event(envelope, &FeishuConfig::default(), Path::new("/tmp")),
            Some(vec![
                ProviderEvent::SessionUpserted(ProviderSession {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_group_fallback_123".to_string(),
                    display_name: Some("tracker".to_string()),
                    unread_count: 0,
                    last_message_at: Some(456),
                    status: SessionStatus::Discovered,
                    bound_thread_id: None,
                }),
                ProviderEvent::InboundMessage(crate::events::ProviderInboundMessage {
                    session: ProviderSessionRef::new(
                        ProviderKind::Feishu,
                        "chat_group_fallback_123",
                    ),
                    message_id: "msg_group_fallback_1".to_string(),
                    text: "hello fallback".to_string(),
                    received_at: 456,
                }),
            ])
        );
    }

    #[test]
    fn chat_entered_event_creates_discovered_session() {
        let events = normalize_chat_entered_event(super::FeishuChatEnteredEnvelope {
            event: super::FeishuChatEnteredEvent {
                chat_id: "chat_123".to_string(),
                operator_id: super::FeishuUserId {
                    open_id: Some("ou_123".to_string()),
                    user_id: None,
                    union_id: None,
                    app_id: None,
                },
                last_message_create_time: Some("456".to_string()),
            },
        });

        assert_eq!(
            events,
            vec![ProviderEvent::SessionUpserted(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_123".to_string(),
                display_name: Some("ou_123".to_string()),
                unread_count: 0,
                last_message_at: Some(456),
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })]
        );
    }

    #[test]
    fn failure_reply_text_uses_first_nonempty_line() {
        assert_eq!(
            failure_reply_text("\nboom\nsecond"),
            "Request failed: boom".to_string()
        );
    }

    #[test]
    fn strip_group_mention_prefix_removes_leading_mentions() {
        assert_eq!(
            strip_group_mention_prefix("@_user_1 TRACKER TEST 2"),
            "TRACKER TEST 2".to_string()
        );
        assert_eq!(
            strip_group_mention_prefix("@bot： hello"),
            "hello".to_string()
        );
        assert_eq!(
            strip_group_mention_prefix("@bot @helper ping"),
            "ping".to_string()
        );
    }
}
