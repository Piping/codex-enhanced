mod runtime_loop;
mod sync;

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use anyhow::anyhow;
use async_trait::async_trait;
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
use super::ProviderReactionReceipt;
use super::ProviderRuntime;
use crate::config::FeishuConfig;
use crate::events::ProviderInboundMessage;
use crate::model::ConnectionStatus;
use crate::model::ProviderKind;
use crate::model::ProviderRuntimeState;
use crate::model::ProviderSession;
use crate::model::ProviderSessionRef;
use crate::model::SessionStatus;

#[derive(Debug, Clone)]
pub struct FeishuInboundPrivateMessage {
    pub chat_id: String,
    pub chat_type: String,
    pub message_id: String,
    pub sender_open_id: Option<String>,
    pub sender_user_id: Option<String>,
    pub sender_union_id: Option<String>,
    pub sender_name: Option<String>,
    pub text: String,
    pub received_at: i64,
}

#[derive(Debug)]
pub struct FeishuProviderRuntime {
    config: FeishuConfig,
    runtime_state: ProviderRuntimeState,
}

impl FeishuProviderRuntime {
    pub fn new(config: FeishuConfig) -> Self {
        let runtime_state = if config.has_api_credentials() {
            ProviderRuntimeState {
                provider: ProviderKind::Feishu,
                connection: ConnectionStatus::Disconnected,
                last_error: None,
                updated_at: None,
            }
        } else {
            ProviderRuntimeState::unconfigured(ProviderKind::Feishu)
        };

        Self {
            config,
            runtime_state,
        }
    }

    pub async fn run(self, provider_event_tx: mpsc::UnboundedSender<ProviderEvent>) -> Result<()> {
        runtime_loop::run_with_reconnect(self.config, provider_event_tx).await
    }

    pub async fn scan_sessions(&mut self) -> Result<Vec<ProviderEvent>> {
        if !self.config.has_api_credentials() {
            let state = self.set_runtime_state(
                ConnectionStatus::Unconfigured,
                Some("missing app_id/app_secret".to_string()),
            )?;
            return Ok(vec![ProviderEvent::RuntimeStateUpdated(state)]);
        }

        let sync_result = sync::discover_private_sessions(&self.messaging_config()?).await?;
        let mut events = Vec::new();
        events.extend(
            sync_result
                .sessions
                .into_iter()
                .map(ProviderEvent::SessionUpserted),
        );
        Ok(events)
    }

    pub fn normalize_private_chat_message(
        message: FeishuInboundPrivateMessage,
    ) -> Option<Vec<ProviderEvent>> {
        if !is_private_chat_type(&message.chat_type) || message.text.trim().is_empty() {
            return None;
        }

        let session = ProviderSession {
            provider: ProviderKind::Feishu,
            session_id: message.chat_id.clone(),
            display_name: message
                .sender_name
                .or(message.sender_open_id.clone())
                .or(message.sender_user_id.clone())
                .or(message.sender_union_id.clone()),
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

    fn websocket_config(&self) -> Result<openlark_client::Config> {
        runtime_loop::build_websocket_config(&self.config)
    }

    fn messaging_config(&self) -> Result<open_lark::openlark_core::config::Config> {
        let config = self.websocket_config()?;
        Ok(config.build_core_config_with_token_provider())
    }

    fn set_runtime_state(
        &mut self,
        connection: ConnectionStatus,
        last_error: Option<String>,
    ) -> Result<ProviderRuntimeState> {
        self.runtime_state.connection = connection;
        self.runtime_state.last_error = last_error;
        self.runtime_state.updated_at = Some(unix_timestamp_now()?);
        Ok(self.runtime_state.clone())
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

#[async_trait]
impl ProviderRuntime for FeishuProviderRuntime {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Feishu
    }

    fn runtime_state(&self) -> &ProviderRuntimeState {
        &self.runtime_state
    }

    async fn connect(&mut self) -> Result<ProviderRuntimeState> {
        if !self.config.has_api_credentials() {
            return self.set_runtime_state(
                ConnectionStatus::Unconfigured,
                Some("missing app_id/app_secret".to_string()),
            );
        }

        self.websocket_config()?;
        self.set_runtime_state(ConnectionStatus::Disconnected, /*last_error*/ None)
    }

    async fn disconnect(&mut self) -> Result<ProviderRuntimeState> {
        self.set_runtime_state(ConnectionStatus::Disconnected, /*last_error*/ None)
    }

    async fn send_text(&mut self, message: ProviderOutboundTextMessage) -> Result<()> {
        if message.session.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot send {} message via Feishu runtime",
                message.session.provider.title()
            ));
        }

        let config = self.messaging_config()?;
        let body = CreateMessageBody {
            receive_id: message.session.session_id,
            msg_type: "text".to_string(),
            content: serde_json::to_string(&serde_json::json!({ "text": message.text }))?,
            uuid: None,
        };

        CreateMessageRequest::new(config)
            .receive_id_type(ReceiveIdType::ChatId)
            .execute(body)
            .await
            .map_err(|error| anyhow!("failed to send Feishu text message: {error}"))?;
        Ok(())
    }

    async fn add_reaction(
        &mut self,
        reaction: ProviderOutboundReaction,
    ) -> Result<ProviderReactionReceipt> {
        if reaction.target.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot send {} reaction via Feishu runtime",
                reaction.target.provider.title()
            ));
        }

        let config = self.messaging_config()?;
        let target = reaction.target;
        let emoji_type = reaction.emoji_type;
        let request: ApiRequest<Value> =
            ApiRequest::post(format!("{IM_V1_MESSAGES}/{}/reactions", target.message_id)).body(
                serialize_params(
                    &CreateMessageReactionBody {
                        reaction_type: ReactionType {
                            emoji_type: emoji_type.clone(),
                        },
                    },
                    "添加消息表情回复",
                )?,
            );
        let response = open_lark::openlark_core::http::Transport::<Value>::request(
            request,
            &config,
            Some(Default::default()),
        )
        .await
        .map_err(|error| anyhow!("failed to add Feishu message reaction: {error}"))?;
        if !response.is_success() {
            return Err(anyhow!(
                "failed to add Feishu message reaction: {}",
                response.msg()
            ));
        }

        if let Some(data) = response.data().cloned() {
            let message_reaction =
                serde_json::from_value::<MessageReaction>(data).map_err(|error| {
                    anyhow!("failed to parse Feishu message reaction response: {error}")
                })?;
            return Ok(ProviderReactionReceipt {
                target,
                reaction_id: message_reaction.reaction_id,
                emoji_type: message_reaction.reaction_type.emoji_type,
            });
        }

        find_feishu_reaction_receipt(&config, &self.config, &target, &emoji_type)
            .await
            .map_err(|error| anyhow!("failed to add Feishu message reaction: {error}"))
    }

    async fn remove_reaction(&mut self, reaction: ProviderReactionReceipt) -> Result<()> {
        if reaction.target.provider != ProviderKind::Feishu {
            return Err(anyhow!(
                "cannot remove {} reaction via Feishu runtime",
                reaction.target.provider.title()
            ));
        }

        let config = self.messaging_config()?;
        let request: ApiRequest<Value> = ApiRequest::delete(format!(
            "{IM_V1_MESSAGES}/{}/reactions/{}",
            reaction.target.message_id, reaction.reaction_id
        ));
        let response = open_lark::openlark_core::http::Transport::<Value>::request(
            request,
            &config,
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
        Ok(())
    }
}

pub(super) fn provider_events_from_payload(payload: &[u8]) -> Vec<ProviderEvent> {
    let Ok(envelope) = serde_json::from_slice::<FeishuEventEnvelope>(payload) else {
        return Vec::new();
    };

    match envelope.header.event_type.as_str() {
        "im.message.receive_v1" => {
            serde_json::from_value::<FeishuMessageReceiveEvent>(envelope.event)
                .ok()
                .and_then(|event| {
                    normalize_message_receive_event(FeishuMessageReceiveEnvelope { event })
                })
                .unwrap_or_default()
        }
        "im.chat.access_event.bot_p2p_chat_entered_v1" => {
            serde_json::from_value::<FeishuChatEnteredEvent>(envelope.event)
                .ok()
                .map(|event| normalize_chat_entered_event(FeishuChatEnteredEnvelope { event }))
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn normalize_message_receive_event(
    envelope: FeishuMessageReceiveEnvelope,
) -> Option<Vec<ProviderEvent>> {
    let chat = envelope.event.chat;
    let message = envelope.event.message;
    if !is_private_chat_type(&message.chat_type) || message.message_type != "text" {
        return None;
    }

    let chat_id = chat
        .as_ref()
        .map(|chat| chat.chat_id.clone())
        .or(message.chat_id.clone())?;
    let text = serde_json::from_str::<FeishuTextContent>(&message.content)
        .ok()
        .map(|content| content.text)
        .unwrap_or_default();
    let received_at = parse_optional_timestamp(Some(message.create_time))?;

    FeishuProviderRuntime::normalize_private_chat_message(FeishuInboundPrivateMessage {
        chat_id,
        chat_type: message.chat_type,
        message_id: message.message_id,
        sender_open_id: envelope.event.sender.sender_id.open_id,
        sender_user_id: envelope.event.sender.sender_id.user_id,
        sender_union_id: envelope.event.sender.sender_id.union_id,
        sender_name: chat.and_then(|chat| chat.name),
        text,
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

fn is_private_chat_type(chat_type: &str) -> bool {
    matches!(chat_type, "p2p" | "private")
}

async fn find_feishu_reaction_receipt(
    config: &open_lark::openlark_core::config::Config,
    feishu_config: &FeishuConfig,
    target: &crate::model::ProviderMessageRef,
    emoji_type: &str,
) -> Result<ProviderReactionReceipt> {
    let mut request = ListMessageReactionsRequest::new(config.clone())
        .message_id(target.message_id.clone())
        .reaction_type(emoji_type)
        .page_size(50);
    if let Some(user_id_type) = reaction_list_user_id_type(feishu_config) {
        request = request.user_id_type(user_id_type);
    }

    let response = request
        .execute()
        .await
        .map_err(|error| anyhow!("failed to list Feishu message reactions: {error}"))?;
    let expected_operator_id = expected_bot_operator_id(feishu_config);
    let reaction = response
        .items
        .unwrap_or_default()
        .into_iter()
        .filter(|candidate| candidate.reaction_type.emoji_type == emoji_type)
        .find(|candidate| {
            expected_operator_id
                .is_none_or(|operator_id| candidate.operator.operator_id == operator_id)
        })
        .ok_or_else(|| anyhow!("reaction was created but could not be resolved from Feishu"))?;

    Ok(ProviderReactionReceipt {
        target: target.clone(),
        reaction_id: reaction.reaction_id,
        emoji_type: reaction.reaction_type.emoji_type,
    })
}

fn reaction_list_user_id_type(config: &FeishuConfig) -> Option<UserIdType> {
    if config
        .bot_open_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        Some(UserIdType::OpenId)
    } else if config
        .bot_user_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        Some(UserIdType::UserId)
    } else {
        None
    }
}

fn expected_bot_operator_id(config: &FeishuConfig) -> Option<&str> {
    config
        .bot_open_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            config
                .bot_user_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
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
}

#[derive(Debug, Deserialize)]
struct FeishuEventMessage {
    message_id: String,
    create_time: String,
    #[serde(default)]
    chat_id: Option<String>,
    chat_type: String,
    message_type: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct FeishuEventChat {
    chat_id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuTextContent {
    text: String,
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
    use pretty_assertions::assert_eq;

    use super::FeishuInboundPrivateMessage;
    use super::failure_reply_text;
    use super::is_private_chat_type;
    use super::normalize_chat_entered_event;
    use super::normalize_message_receive_event;
    use super::parse_optional_timestamp;
    use crate::model::ProviderKind;
    use crate::model::ProviderSession;
    use crate::model::ProviderSessionRef;
    use crate::model::SessionStatus;
    use crate::provider::ProviderEvent;

    #[test]
    fn normalize_private_chat_message_creates_session_and_inbound_events() {
        let events = super::FeishuProviderRuntime::normalize_private_chat_message(
            FeishuInboundPrivateMessage {
                chat_id: "chat_123".to_string(),
                chat_type: "p2p".to_string(),
                message_id: "msg_123".to_string(),
                sender_open_id: Some("ou_123".to_string()),
                sender_user_id: None,
                sender_union_id: None,
                sender_name: Some("Alice".to_string()),
                text: "hello".to_string(),
                received_at: 123,
            },
        )
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
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_123".to_string(),
                    create_time: "456".to_string(),
                    chat_id: Some("chat_123".to_string()),
                    chat_type: "p2p".to_string(),
                    message_type: "image".to_string(),
                    content: "{}".to_string(),
                },
                chat: None,
            },
        };

        assert_eq!(normalize_message_receive_event(envelope), None);
    }

    #[test]
    fn message_receive_event_reads_chat_id_from_chat_object() {
        let events = normalize_message_receive_event(super::FeishuMessageReceiveEnvelope {
            event: super::FeishuMessageReceiveEvent {
                sender: super::FeishuEventSender {
                    sender_id: super::FeishuUserId {
                        open_id: Some("ou_123".to_string()),
                        user_id: None,
                        union_id: None,
                    },
                },
                message: super::FeishuEventMessage {
                    message_id: "msg_123".to_string(),
                    create_time: "456".to_string(),
                    chat_id: None,
                    chat_type: "p2p".to_string(),
                    message_type: "text".to_string(),
                    content: "{\"text\":\"hello\"}".to_string(),
                },
                chat: Some(super::FeishuEventChat {
                    chat_id: "chat_123".to_string(),
                    name: Some("机器人".to_string()),
                }),
            },
        })
        .expect("events");

        assert_eq!(
            events,
            vec![
                ProviderEvent::SessionUpserted(ProviderSession {
                    provider: ProviderKind::Feishu,
                    session_id: "chat_123".to_string(),
                    display_name: Some("机器人".to_string()),
                    unread_count: 0,
                    last_message_at: Some(456),
                    status: SessionStatus::Discovered,
                    bound_thread_id: None,
                }),
                ProviderEvent::InboundMessage(crate::events::ProviderInboundMessage {
                    session: ProviderSessionRef::new(ProviderKind::Feishu, "chat_123"),
                    message_id: "msg_123".to_string(),
                    text: "hello".to_string(),
                    received_at: 456,
                }),
            ]
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
                },
                last_message_create_time: Some("789".to_string()),
            },
        });

        assert_eq!(
            events,
            vec![ProviderEvent::SessionUpserted(ProviderSession {
                provider: ProviderKind::Feishu,
                session_id: "chat_123".to_string(),
                display_name: Some("ou_123".to_string()),
                unread_count: 0,
                last_message_at: Some(789),
                status: SessionStatus::Discovered,
                bound_thread_id: None,
            })]
        );
    }

    #[test]
    fn parse_optional_timestamp_returns_none_for_invalid_input() {
        assert_eq!(
            parse_optional_timestamp(Some("not-a-number".to_string())),
            None
        );
    }

    #[test]
    fn failure_reply_text_uses_first_non_empty_line_and_truncates() {
        let message = format!("\n\n{}\nsecond line", "x".repeat(170));

        assert_eq!(
            failure_reply_text(&message),
            format!("Request failed: {}…", "x".repeat(160))
        );
    }

    #[test]
    fn private_chat_type_accepts_p2p_and_private() {
        assert_eq!(is_private_chat_type("p2p"), true);
        assert_eq!(is_private_chat_type("private"), true);
        assert_eq!(is_private_chat_type("group"), false);
    }
}
