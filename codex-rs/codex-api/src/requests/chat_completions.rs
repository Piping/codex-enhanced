use crate::requests::headers::build_conversation_headers;
use crate::requests::headers::insert_header;
use crate::requests::headers::subagent_header;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use http::HeaderMap;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;

/// Assembled request body plus headers for Chat Completions streaming calls.
pub struct ChatCompletionsRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

pub struct ChatCompletionsRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
    conversation_id: Option<String>,
    session_source: Option<SessionSource>,
}

impl<'a> ChatCompletionsRequestBuilder<'a> {
    pub fn new(
        model: &'a str,
        instructions: &'a str,
        input: &'a [ResponseItem],
        tools: &'a [Value],
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            conversation_id: None,
            session_source: None,
        }
    }

    pub fn conversation_id(mut self, conversation_id: Option<String>) -> Self {
        self.conversation_id = conversation_id;
        self
    }

    pub fn session_source(mut self, session_source: Option<SessionSource>) -> Self {
        self.session_source = session_source;
        self
    }

    pub fn build(self) -> ChatCompletionsRequest {
        let mut messages = Vec::<Value>::new();
        messages.push(json!({
            "role": "system",
            "content": self.instructions,
        }));

        let mut reasoning_by_anchor_index = reasoning_by_anchor_index(self.input);
        let mut last_assistant_text: Option<String> = None;

        for (index, item) in self.input.iter().enumerate() {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    let (text, content_value) = content_to_chat_value(role, content);
                    if role == "assistant" && last_assistant_text.as_ref() == Some(&text) {
                        continue;
                    }
                    if role == "assistant" {
                        last_assistant_text = Some(text.clone());
                    }

                    let mut message = json!({
                        "role": role,
                        "content": content_value,
                    });
                    if role == "assistant"
                        && let Some(reasoning) = reasoning_by_anchor_index.remove(&index)
                        && let Some(map) = message.as_object_mut()
                    {
                        map.insert("reasoning".to_string(), reasoning.into());
                    }
                    messages.push(message);
                }
                ResponseItem::FunctionCall {
                    name,
                    namespace,
                    arguments,
                    call_id,
                    ..
                } => {
                    let tool_call = json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": tool_call_name(name, namespace.as_deref()),
                            "arguments": arguments,
                        }
                    });
                    push_tool_call_message(
                        &mut messages,
                        tool_call,
                        reasoning_by_anchor_index.remove(&index).as_deref(),
                    );
                }
                ResponseItem::FunctionCallOutput { call_id, output } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": function_output_to_chat_value(output),
                    }));
                }
                ResponseItem::CustomToolCall {
                    id, name, input, ..
                } => {
                    let tool_call = json!({
                        "id": id,
                        "type": "custom",
                        "custom": {
                            "name": name,
                            "input": input,
                        }
                    });
                    push_tool_call_message(
                        &mut messages,
                        tool_call,
                        reasoning_by_anchor_index.remove(&index).as_deref(),
                    );
                }
                ResponseItem::CustomToolCallOutput {
                    call_id, output, ..
                } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": function_output_to_chat_value(output),
                    }));
                }
                ResponseItem::Reasoning { .. }
                | ResponseItem::LocalShellCall { .. }
                | ResponseItem::ToolSearchCall { .. }
                | ResponseItem::ToolSearchOutput { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::ImageGenerationCall { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::Compaction { .. }
                | ResponseItem::Other => {}
            }
        }

        let body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "tools": self.tools,
        });

        let mut headers = build_conversation_headers(self.conversation_id);
        if let Some(subagent) = subagent_header(&self.session_source) {
            insert_header(&mut headers, "x-openai-subagent", &subagent);
        }

        ChatCompletionsRequest { body, headers }
    }
}

fn reasoning_by_anchor_index(input: &[ResponseItem]) -> HashMap<usize, String> {
    let mut last_role: Option<&str> = None;
    for item in input {
        match item {
            ResponseItem::Message { role, .. } => last_role = Some(role.as_str()),
            ResponseItem::FunctionCall { .. } | ResponseItem::CustomToolCall { .. } => {
                last_role = Some("assistant");
            }
            ResponseItem::FunctionCallOutput { .. } | ResponseItem::CustomToolCallOutput { .. } => {
                last_role = Some("tool");
            }
            ResponseItem::Reasoning { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
    }

    if last_role == Some("user") {
        return HashMap::new();
    }

    let mut last_user_index = None;
    for (index, item) in input.iter().enumerate() {
        if let ResponseItem::Message { role, .. } = item
            && role == "user"
        {
            last_user_index = Some(index);
        }
    }

    let mut reasoning = HashMap::new();
    for (index, item) in input.iter().enumerate() {
        let ResponseItem::Reasoning {
            content: Some(content),
            ..
        } = item
        else {
            continue;
        };

        if let Some(last_user_index) = last_user_index
            && index <= last_user_index
        {
            continue;
        }

        let mut text = String::new();
        for item in content {
            match item {
                ReasoningItemContent::ReasoningText { text: chunk }
                | ReasoningItemContent::Text { text: chunk } => text.push_str(chunk),
            }
        }
        if text.trim().is_empty() {
            continue;
        }

        if index > 0
            && let ResponseItem::Message { role, .. } = &input[index - 1]
            && role == "assistant"
        {
            reasoning
                .entry(index - 1)
                .and_modify(|existing: &mut String| existing.push_str(&text))
                .or_insert(text.clone());
            continue;
        }

        if index + 1 < input.len() {
            let next_item = &input[index + 1];
            let can_attach = match next_item {
                ResponseItem::FunctionCall { .. } | ResponseItem::CustomToolCall { .. } => true,
                ResponseItem::Message { role, .. } => role == "assistant",
                ResponseItem::Reasoning { .. }
                | ResponseItem::FunctionCallOutput { .. }
                | ResponseItem::CustomToolCallOutput { .. }
                | ResponseItem::LocalShellCall { .. }
                | ResponseItem::ToolSearchCall { .. }
                | ResponseItem::ToolSearchOutput { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::ImageGenerationCall { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::Compaction { .. }
                | ResponseItem::Other => false,
            };
            if can_attach {
                reasoning
                    .entry(index + 1)
                    .and_modify(|existing: &mut String| existing.push_str(&text))
                    .or_insert(text);
            }
        }
    }

    reasoning
}

fn tool_call_name(name: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(namespace) if name.starts_with(namespace) => name.to_string(),
        Some(namespace) => format!("{namespace}{name}"),
        None => name.to_string(),
    }
}

fn content_to_chat_value(role: &str, content: &[ContentItem]) -> (String, Value) {
    let mut text = String::new();
    let mut items = Vec::<Value>::new();
    let mut saw_image = false;

    for item in content {
        match item {
            ContentItem::InputText { text: chunk } | ContentItem::OutputText { text: chunk } => {
                text.push_str(chunk);
                items.push(json!({
                    "type": "text",
                    "text": chunk,
                }));
            }
            ContentItem::InputImage { image_url } => {
                saw_image = true;
                items.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": image_url,
                    }
                }));
            }
        }
    }

    let value = if role == "assistant" {
        text.clone().into()
    } else if saw_image {
        items.into()
    } else {
        text.clone().into()
    };

    (text, value)
}

fn function_output_to_chat_value(
    output: &codex_protocol::models::FunctionCallOutputPayload,
) -> Value {
    if let Some(items) = output.content_items() {
        let items = items
            .iter()
            .map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => json!({
                    "type": "text",
                    "text": text,
                }),
                FunctionCallOutputContentItem::InputImage { image_url, .. } => json!({
                    "type": "image_url",
                    "image_url": {
                        "url": image_url,
                    }
                }),
            })
            .collect::<Vec<_>>();
        items.into()
    } else {
        output.to_string().into()
    }
}

fn push_tool_call_message(messages: &mut Vec<Value>, tool_call: Value, reasoning: Option<&str>) {
    if let Some(Value::Object(message)) = messages.last_mut()
        && message.get("role").and_then(Value::as_str) == Some("assistant")
        && message.get("content").is_some_and(Value::is_null)
        && let Some(tool_calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut)
    {
        tool_calls.push(tool_call);
        if let Some(reasoning) = reasoning {
            if let Some(Value::String(existing)) = message.get_mut("reasoning") {
                if !existing.is_empty() {
                    existing.push('\n');
                }
                existing.push_str(reasoning);
            } else {
                message.insert("reasoning".to_string(), reasoning.to_string().into());
            }
        }
        return;
    }

    let mut message = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [tool_call],
    });
    if let Some(reasoning) = reasoning
        && let Some(map) = message.as_object_mut()
    {
        map.insert("reasoning".to_string(), reasoning.to_string().into());
    }
    messages.push(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::protocol::SubAgentSource;
    use pretty_assertions::assert_eq;

    fn user_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            end_turn: None,
            phase: None,
        }
    }

    fn assistant_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: text.to_string(),
            }],
            end_turn: None,
            phase: None,
        }
    }

    fn reasoning_item(text: &str) -> ResponseItem {
        ResponseItem::Reasoning {
            id: String::new(),
            summary: Vec::new(),
            content: Some(vec![ReasoningItemContent::ReasoningText {
                text: text.to_string(),
            }]),
            encrypted_content: None,
        }
    }

    #[test]
    fn groups_tool_calls_and_preserves_headers() {
        let input = vec![
            user_message("u1"),
            reasoning_item("why"),
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                namespace: None,
                arguments: "{\"command\":[\"pwd\"]}".to_string(),
                call_id: "call-a".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-a".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "apply_patch".to_string(),
                namespace: None,
                arguments: "{\"input\":\"*** Begin Patch\\n*** End Patch\\n\"}".to_string(),
                call_id: "call-b".to_string(),
            },
        ];

        let request = ChatCompletionsRequestBuilder::new(
            "gpt-test",
            "system",
            &input,
            &[json!({"type":"function","function":{"name":"shell"}})],
        )
        .conversation_id(Some("conv-1".to_string()))
        .session_source(Some(SessionSource::SubAgent(SubAgentSource::Other(
            "memory_consolidation".to_string(),
        ))))
        .build();

        let messages = request.body["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], Value::Null);
        assert_eq!(messages[2]["reasoning"], "why");
        assert_eq!(
            messages[2]["tool_calls"],
            json!([
                {
                    "id": "call-a",
                    "type": "function",
                    "function": {
                        "name": "shell",
                        "arguments": "{\"command\":[\"pwd\"]}",
                    }
                }
            ])
        );
        assert_eq!(
            messages[3],
            json!({
                "role": "tool",
                "tool_call_id": "call-a",
                "content": "ok",
            })
        );
        assert_eq!(
            messages[4],
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-b",
                    "type": "function",
                    "function": {
                        "name": "apply_patch",
                        "arguments": "{\"input\":\"*** Begin Patch\\n*** End Patch\\n\"}",
                    }
                }],
            })
        );
        assert_eq!(
            request
                .headers
                .get("x-openai-subagent")
                .and_then(|value| value.to_str().ok()),
            Some("memory_consolidation")
        );
    }

    #[test]
    fn normalizes_namespace_names_and_skips_duplicate_assistant_messages() {
        let input = vec![
            assistant_message("dup"),
            assistant_message("dup"),
            ResponseItem::FunctionCall {
                id: None,
                name: "lookup".to_string(),
                namespace: Some("mcp__server__".to_string()),
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
        ];

        let request = ChatCompletionsRequestBuilder::new("gpt-test", "system", &input, &[]).build();
        let messages = request.body["messages"]
            .as_array()
            .expect("messages should be an array");

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["content"], "dup");
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["name"],
            "mcp__server__lookup"
        );
    }

    #[test]
    fn preserves_prefixed_function_call_names() {
        let input = vec![ResponseItem::FunctionCall {
            id: None,
            name: "mcp__server__lookup".to_string(),
            namespace: Some("mcp__server__".to_string()),
            arguments: "{}".to_string(),
            call_id: "call-1".to_string(),
        }];

        let request = ChatCompletionsRequestBuilder::new("gpt-test", "system", &input, &[]).build();
        let messages = request.body["messages"]
            .as_array()
            .expect("messages should be an array");

        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["name"],
            "mcp__server__lookup"
        );
    }
}
