use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use serde_json::Value;
use serde_json::json;

/// Assembled request body plus headers for Anthropic Messages streaming calls.
pub struct MessagesRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

pub struct MessagesRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
}

impl<'a> MessagesRequestBuilder<'a> {
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
        }
    }

    pub fn build(self) -> MessagesRequest {
        let mut messages = Vec::<Value>::new();
        let mut pending_tool_results = Vec::<Value>::new();
        let mut last_assistant_text: Option<String> = None;

        for item in self.input {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    if !pending_tool_results.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": std::mem::take(&mut pending_tool_results),
                        }));
                    }

                    let (text, content_value) = content_to_messages_value(content);
                    if role == "assistant" && last_assistant_text.as_ref() == Some(&text) {
                        continue;
                    }
                    if role == "assistant" {
                        last_assistant_text = Some(text);
                    }

                    let role = if role == "assistant" {
                        "assistant"
                    } else {
                        "user"
                    };
                    messages.push(json!({
                        "role": role,
                        "content": content_value,
                    }));
                }
                ResponseItem::FunctionCall {
                    name,
                    namespace,
                    arguments,
                    call_id,
                    ..
                } => {
                    if !pending_tool_results.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": std::mem::take(&mut pending_tool_results),
                        }));
                    }
                    push_assistant_tool_use(
                        &mut messages,
                        call_id,
                        &tool_call_name(name, namespace.as_deref()),
                        arguments,
                    );
                }
                ResponseItem::FunctionCallOutput { call_id, output } => {
                    pending_tool_results.push(function_output_to_messages_value(call_id, output));
                }
                ResponseItem::CustomToolCall {
                    call_id,
                    name,
                    input,
                    ..
                } => {
                    if !pending_tool_results.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": std::mem::take(&mut pending_tool_results),
                        }));
                    }
                    push_assistant_tool_use(&mut messages, call_id, name, input);
                }
                ResponseItem::CustomToolCallOutput {
                    call_id, output, ..
                } => {
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": output,
                    }));
                }
                ResponseItem::Reasoning { .. }
                | ResponseItem::LocalShellCall { .. }
                | ResponseItem::ToolSearchCall { .. }
                | ResponseItem::ToolSearchOutput { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::ImageGenerationCall { .. }
                | ResponseItem::Compaction { .. }
                | ResponseItem::ContextCompaction { .. }
                | ResponseItem::Other => {}
            }
        }

        if !pending_tool_results.is_empty() {
            messages.push(json!({
                "role": "user",
                "content": pending_tool_results,
            }));
        }

        let body = json!({
            "model": self.model,
            "system": self.instructions,
            "messages": messages,
            "stream": true,
            "max_tokens": 4096,
            "tools": self.tools,
        });

        MessagesRequest {
            body,
            headers: HeaderMap::new(),
        }
    }
}

fn tool_call_name(name: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(namespace) if name.starts_with(namespace) => name.to_string(),
        Some(namespace) => format!("{namespace}{name}"),
        None => name.to_string(),
    }
}

fn content_to_messages_value(content: &[ContentItem]) -> (String, Value) {
    let mut text = String::new();
    let items = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text: chunk } | ContentItem::OutputText { text: chunk } => {
                text.push_str(chunk);
                Some(json!({
                    "type": "text",
                    "text": chunk,
                }))
            }
            ContentItem::InputImage { image_url, .. } => image_url_to_messages_image(image_url),
        })
        .collect::<Vec<_>>();

    (text, items.into())
}

fn image_url_to_messages_image(image_url: &str) -> Option<Value> {
    let data = image_url.strip_prefix("data:")?;
    let (media_type, data) = data.split_once(";base64,")?;
    Some(json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": media_type,
            "data": data,
        }
    }))
}

fn function_output_to_messages_value(call_id: &str, output: &FunctionCallOutputPayload) -> Value {
    let content: Value = if let Some(items) = output.content_items() {
        items
            .iter()
            .filter_map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => Some(json!({
                    "type": "text",
                    "text": text,
                })),
                FunctionCallOutputContentItem::InputImage { image_url, .. } => {
                    image_url_to_messages_image(image_url)
                }
            })
            .collect::<Vec<_>>()
            .into()
    } else {
        output.to_string().into()
    };

    json!({
        "type": "tool_result",
        "tool_use_id": call_id,
        "content": content,
    })
}

fn push_assistant_tool_use(messages: &mut Vec<Value>, call_id: &str, name: &str, arguments: &str) {
    let input = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| {
        json!({
            "input": arguments,
        })
    });
    let tool_use = json!({
        "type": "tool_use",
        "id": call_id,
        "name": name,
        "input": input,
    });

    if let Some(Value::Object(message)) = messages.last_mut()
        && message.get("role").and_then(Value::as_str) == Some("assistant")
        && let Some(content) = message.get_mut("content").and_then(Value::as_array_mut)
    {
        content.push(tool_use);
        return;
    }

    messages.push(json!({
        "role": "assistant",
        "content": [tool_use],
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::FunctionCallOutputPayload;
    use pretty_assertions::assert_eq;

    fn user_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    #[test]
    fn builds_anthropic_messages_with_tool_roundtrip() {
        let input = vec![
            user_message("u1"),
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                namespace: None,
                arguments: "{\"cmd\":\"pwd\"}".to_string(),
                call_id: "toolu_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "toolu_1".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
        ];

        let request = MessagesRequestBuilder::new(
            "claude-test",
            "system",
            &input,
            &[json!({"name":"shell","input_schema":{"type":"object"}})],
        )
        .build();

        assert_eq!(request.body["model"], "claude-test");
        assert_eq!(request.body["system"], "system");
        assert_eq!(
            request.body["messages"],
            json!([
                {"role":"user","content":[{"type":"text","text":"u1"}]},
                {"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"shell","input":{"cmd":"pwd"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}
            ])
        );
    }
}
