use anyhow::Context;
use anyhow::Result;
use open_lark::openlark_communication::im::im::v1::chat::get::GetChatRequest;
use open_lark::openlark_communication::im::im::v1::chat::list::ListChatsRequest;
use open_lark::openlark_communication::im::im::v1::chat::models::ChatSortType;
use open_lark::openlark_communication::im::im::v1::message::models::UserIdType;
use serde::Deserialize;

use crate::model::ProviderKind;
use crate::model::ProviderSession;
use crate::model::SessionStatus;

pub(super) async fn discover_private_sessions(
    config: &open_lark::openlark_core::config::Config,
) -> Result<Vec<ProviderSession>> {
    let mut sessions = Vec::new();
    let mut page_token = None;

    loop {
        let response = list_chat_page(config, page_token.clone()).await?;
        let next_page_token = response.page_token.clone();

        for chat in response.items {
            if let Some(session) = load_private_session(config, chat).await? {
                sessions.push(session);
            }
        }

        if !response.has_more {
            break;
        }

        let Some(token) = next_page_token.filter(|token| !token.is_empty()) else {
            break;
        };
        page_token = Some(token);
    }

    sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    sessions.dedup_by(|left, right| left.session_id == right.session_id);
    Ok(sessions)
}

async fn list_chat_page(
    config: &open_lark::openlark_core::config::Config,
    page_token: Option<String>,
) -> Result<FeishuChatListResponse> {
    let mut request = ListChatsRequest::new(config.clone())
        .user_id_type(UserIdType::OpenId)
        .sort_type(ChatSortType::ByActiveTimeDesc)
        .page_size(100);
    if let Some(token) = page_token {
        request = request.page_token(token);
    }

    let response = request
        .execute()
        .await
        .context("failed to list Feishu chats")?;
    serde_json::from_value(response).context("failed to parse Feishu chat list response")
}

async fn load_private_session(
    config: &open_lark::openlark_core::config::Config,
    chat: FeishuChatListItem,
) -> Result<Option<ProviderSession>> {
    let chat_id = chat.chat_id.clone();
    let response = GetChatRequest::new(config.clone())
        .chat_id(chat_id.clone())
        .user_id_type(UserIdType::OpenId)
        .execute()
        .await
        .with_context(|| format!("failed to load Feishu chat {chat_id}"))?;
    let details: FeishuChatDetails = serde_json::from_value(response)
        .with_context(|| format!("failed to parse Feishu chat details for {chat_id}"))?;

    if !is_private_chat(&details) {
        return Ok(None);
    }

    Ok(Some(ProviderSession {
        provider: ProviderKind::Feishu,
        session_id: chat.chat_id,
        display_name: first_non_empty([chat.name, details.name]),
        unread_count: 0,
        last_message_at: None,
        status: SessionStatus::Discovered,
        bound_thread_id: None,
    }))
}

fn first_non_empty(values: [Option<String>; 2]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn is_private_chat(details: &FeishuChatDetails) -> bool {
    details.chat_type.as_deref() == Some("private")
        || details.chat_mode.as_deref() == Some("p2p")
        || details.r#type.as_deref() == Some("p2p")
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct FeishuChatListResponse {
    #[serde(default)]
    items: Vec<FeishuChatListItem>,
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct FeishuChatListItem {
    chat_id: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct FeishuChatDetails {
    name: Option<String>,
    chat_mode: Option<String>,
    chat_type: Option<String>,
    #[serde(rename = "type")]
    r#type: Option<String>,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::FeishuChatDetails;
    use super::FeishuChatListItem;
    use super::FeishuChatListResponse;
    use super::first_non_empty;
    use super::is_private_chat;

    #[test]
    fn parse_list_chat_response_with_items() {
        let response: FeishuChatListResponse = serde_json::from_value(serde_json::json!({
            "items": [
                { "chat_id": "oc_1", "name": "Alice" },
                { "chat_id": "oc_2", "name": "Bob" }
            ],
            "page_token": "next_token",
            "has_more": true
        }))
        .expect("response");

        assert_eq!(
            response,
            FeishuChatListResponse {
                items: vec![
                    FeishuChatListItem {
                        chat_id: "oc_1".to_string(),
                        name: Some("Alice".to_string()),
                    },
                    FeishuChatListItem {
                        chat_id: "oc_2".to_string(),
                        name: Some("Bob".to_string()),
                    },
                ],
                page_token: Some("next_token".to_string()),
                has_more: true,
            }
        );
    }

    #[test]
    fn parse_chat_details_with_type_field() {
        let response: FeishuChatDetails = serde_json::from_value(serde_json::json!({
            "chat_id": "oc_1",
            "name": "Alice",
            "type": "p2p"
        }))
        .expect("response");

        assert_eq!(
            response,
            FeishuChatDetails {
                name: Some("Alice".to_string()),
                chat_mode: None,
                chat_type: None,
                r#type: Some("p2p".to_string()),
            }
        );
    }

    #[test]
    fn private_chat_detection_accepts_private_variants() {
        assert_eq!(
            is_private_chat(&FeishuChatDetails {
                name: Some("Alice".to_string()),
                chat_mode: Some("group".to_string()),
                chat_type: Some("private".to_string()),
                r#type: None,
            }),
            true
        );
        assert_eq!(
            is_private_chat(&FeishuChatDetails {
                name: Some("Alice".to_string()),
                chat_mode: Some("p2p".to_string()),
                chat_type: Some("group".to_string()),
                r#type: None,
            }),
            true
        );
    }

    #[test]
    fn first_non_empty_skips_blank_values() {
        assert_eq!(
            first_non_empty([Some("   ".to_string()), Some("Alice".to_string())]),
            Some("Alice".to_string())
        );
    }
}
