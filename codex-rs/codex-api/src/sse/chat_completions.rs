use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::rate_limits::parse_all_rate_limits;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

const OPENAI_MODEL_HEADER: &str = "openai-model";

pub fn spawn_chat_completions_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    _turn_state: Option<Arc<OnceLock<String>>>,
) -> ResponseStream {
    let rate_limit_snapshots = parse_all_rate_limits(&stream_response.headers);
    let server_model = stream_response
        .headers
        .get(OPENAI_MODEL_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        if let Some(server_model) = server_model {
            let _ = tx_event
                .send(Ok(ResponseEvent::ServerModel(server_model)))
                .await;
        }
        for snapshot in rate_limit_snapshots {
            let _ = tx_event.send(Ok(ResponseEvent::RateLimits(snapshot))).await;
        }
        process_chat_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });
    ResponseStream { rx_event }
}

pub async fn process_chat_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();

    #[derive(Default, Debug)]
    struct ToolCallState {
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    }

    let mut tool_calls: HashMap<usize, ToolCallState> = HashMap::new();
    let mut tool_call_order = Vec::new();
    let mut tool_call_order_seen = HashSet::new();
    let mut tool_call_index_by_id = HashMap::new();
    let mut next_tool_call_index = 0usize;
    let mut last_tool_call_index = None;
    let mut assistant_item = None;
    let mut reasoning_item = None;
    let mut completed_sent = false;

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(err))) => {
                let _ = tx_event.send(Err(ApiError::Stream(err.to_string()))).await;
                return;
            }
            Ok(None) => {
                if !completed_sent {
                    flush_and_complete(&tx_event, &mut reasoning_item, &mut assistant_item).await;
                }
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "idle timeout waiting for SSE".to_string(),
                    )))
                    .await;
                return;
            }
        };

        trace!("SSE event: {}", sse.data);

        let data = sse.data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" || data == "DONE" {
            if !completed_sent {
                flush_and_complete(&tx_event, &mut reasoning_item, &mut assistant_item).await;
            }
            return;
        }

        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(err) => {
                debug!("failed to parse chat completions event: {err}, data: {data}");
                continue;
            }
        };

        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            continue;
        };

        for choice in choices {
            if let Some(delta) = choice.get("delta") {
                if let Some(reasoning) = delta.get("reasoning")
                    && let Some(text) = extract_reasoning_text(reasoning)
                {
                    append_reasoning_text(&tx_event, &mut reasoning_item, text.to_string()).await;
                }

                if let Some(content) = delta.get("content") {
                    append_delta_content(&tx_event, &mut assistant_item, content).await;
                }

                if let Some(tool_call_values) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tool_call in tool_call_values {
                        let mut index = tool_call
                            .get("index")
                            .and_then(Value::as_u64)
                            .map(|index| index as usize);

                        let mut call_id_for_lookup = None;
                        if let Some(call_id) = tool_call.get("id").and_then(Value::as_str) {
                            call_id_for_lookup = Some(call_id.to_string());
                            if let Some(existing_index) = tool_call_index_by_id.get(call_id) {
                                index = Some(*existing_index);
                            }
                        }

                        if index.is_none() && call_id_for_lookup.is_none() {
                            index = last_tool_call_index;
                        }

                        let index = index.unwrap_or_else(|| {
                            while tool_calls.contains_key(&next_tool_call_index) {
                                next_tool_call_index += 1;
                            }
                            let index = next_tool_call_index;
                            next_tool_call_index += 1;
                            index
                        });

                        let call_state = tool_calls.entry(index).or_default();
                        if tool_call_order_seen.insert(index) {
                            tool_call_order.push(index);
                        }

                        if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                            call_state.id.get_or_insert_with(|| id.to_string());
                            tool_call_index_by_id.entry(id.to_string()).or_insert(index);
                        }

                        if let Some(function) = tool_call.get("function") {
                            if let Some(name) = function.get("name").and_then(Value::as_str)
                                && !name.is_empty()
                            {
                                call_state.name.get_or_insert_with(|| name.to_string());
                            }
                            if let Some(arguments) =
                                function.get("arguments").and_then(Value::as_str)
                            {
                                call_state.arguments.push_str(arguments);
                            }
                        }

                        last_tool_call_index = Some(index);
                    }
                }
            }

            if let Some(message) = choice.get("message")
                && let Some(reasoning) = message.get("reasoning")
                && let Some(text) = extract_reasoning_text(reasoning)
            {
                append_reasoning_text(&tx_event, &mut reasoning_item, text.to_string()).await;
            }

            let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
            if finish_reason == Some("stop") {
                if let Some(reasoning) = reasoning_item.take() {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(reasoning)))
                        .await;
                }
                if let Some(assistant) = assistant_item.take() {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(assistant)))
                        .await;
                }
                if !completed_sent {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: String::new(),
                            token_usage: None,
                        }))
                        .await;
                    completed_sent = true;
                }
                continue;
            }

            if finish_reason == Some("length") {
                let _ = tx_event.send(Err(ApiError::ContextWindowExceeded)).await;
                return;
            }

            if finish_reason == Some("tool_calls") {
                if let Some(reasoning) = reasoning_item.take() {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(reasoning)))
                        .await;
                }

                for index in tool_call_order.drain(..) {
                    let Some(tool_call) = tool_calls.remove(&index) else {
                        continue;
                    };
                    tool_call_order_seen.remove(&index);
                    let Some(name) = tool_call.name else {
                        debug!("skipping tool call {index} because name is missing");
                        continue;
                    };
                    let item = ResponseItem::FunctionCall {
                        id: None,
                        name,
                        namespace: None,
                        arguments: tool_call.arguments,
                        call_id: tool_call.id.unwrap_or_else(|| format!("tool-call-{index}")),
                    };
                    let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                }
            }
        }
    }
}

fn extract_reasoning_text(reasoning: &Value) -> Option<&str> {
    reasoning
        .as_str()
        .or_else(|| reasoning.get("text").and_then(Value::as_str))
        .or_else(|| reasoning.get("content").and_then(Value::as_str))
}

async fn append_delta_content(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    assistant_item: &mut Option<ResponseItem>,
    content: &Value,
) {
    if let Some(text) = content.as_str() {
        append_assistant_text(tx_event, assistant_item, text.to_string()).await;
        return;
    }

    if let Some(items) = content.as_array() {
        for item in items {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                append_assistant_text(tx_event, assistant_item, text.to_string()).await;
            }
        }
    }
}

async fn append_assistant_text(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    assistant_item: &mut Option<ResponseItem>,
    text: String,
) {
    if assistant_item.is_none() {
        let item = ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: Vec::new(),
            end_turn: None,
            phase: None,
        };
        *assistant_item = Some(item.clone());
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputItemAdded(item)))
            .await;
    }

    if let Some(ResponseItem::Message { content, .. }) = assistant_item {
        content.push(ContentItem::OutputText { text: text.clone() });
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputTextDelta(text)))
            .await;
    }
}

async fn append_reasoning_text(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    reasoning_item: &mut Option<ResponseItem>,
    text: String,
) {
    if reasoning_item.is_none() {
        let item = ResponseItem::Reasoning {
            id: String::new(),
            summary: Vec::new(),
            content: Some(Vec::new()),
            encrypted_content: None,
        };
        *reasoning_item = Some(item.clone());
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputItemAdded(item)))
            .await;
    }

    if let Some(ResponseItem::Reasoning {
        content: Some(content),
        ..
    }) = reasoning_item
    {
        let content_index = i64::try_from(content.len()).unwrap_or(i64::MAX);
        content.push(ReasoningItemContent::ReasoningText { text: text.clone() });
        let _ = tx_event
            .send(Ok(ResponseEvent::ReasoningContentDelta {
                delta: text,
                content_index,
            }))
            .await;
    }
}

async fn flush_and_complete(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    reasoning_item: &mut Option<ResponseItem>,
    assistant_item: &mut Option<ResponseItem>,
) {
    if let Some(reasoning) = reasoning_item.take() {
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputItemDone(reasoning)))
            .await;
    }
    if let Some(assistant) = assistant_item.take() {
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputItemDone(assistant)))
            .await;
    }
    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id: String::new(),
            token_usage: None,
        }))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use codex_client::TransportError;
    use futures::TryStreamExt;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tokio_util::io::ReaderStream;

    async fn collect_events(body: &str) -> Vec<ResponseEvent> {
        let reader = ReaderStream::new(std::io::Cursor::new(body.to_string()))
            .map_err(|err| TransportError::Network(err.to_string()));
        let (tx, mut rx) = mpsc::channel::<Result<ResponseEvent, ApiError>>(32);
        tokio::spawn(process_chat_sse(
            Box::pin(reader),
            tx,
            Duration::from_millis(1000),
            None,
        ));

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event.expect("stream should not error"));
        }
        events
    }

    fn build_body(events: &[Value]) -> String {
        let mut body = String::new();
        for event in events {
            body.push_str(&format!("event: message\ndata: {event}\n\n"));
        }
        body
    }

    #[tokio::test]
    async fn streams_text_and_reasoning() {
        let body = build_body(&[
            json!({"choices":[{"delta":{"reasoning":"think"}}]}),
            json!({"choices":[{"delta":{"content":"hi"}}]}),
            json!({"choices":[{"finish_reason":"stop"}]}),
        ]);

        let events = collect_events(&body).await;

        assert_eq!(events.len(), 7);
        assert_matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        );
        assert_matches!(
            &events[1],
            ResponseEvent::ReasoningContentDelta { delta, content_index }
                if delta == "think" && *content_index == 0
        );
        assert_matches!(
            &events[2],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        );
        assert_matches!(&events[3], ResponseEvent::OutputTextDelta(delta) if delta == "hi");
        assert_matches!(
            &events[4],
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning { .. })
        );
        assert_matches!(
            &events[5],
            ResponseEvent::OutputItemDone(ResponseItem::Message { role, .. }) if role == "assistant"
        );
        assert_matches!(&events[6], ResponseEvent::Completed { .. });
    }

    #[tokio::test]
    async fn concatenates_tool_call_arguments_across_deltas() {
        let body = build_body(&[
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "id": "call_a",
                            "index": 0,
                            "function": { "name": "apply_patch" }
                        }]
                    }
                }]
            }),
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "arguments": "{ \"input\":" }
                        }]
                    }
                }]
            }),
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "arguments": "\"*** Begin Patch\"}" }
                        }]
                    }
                }]
            }),
            json!({"choices":[{"finish_reason":"tool_calls"}]}),
        ]);

        let events = collect_events(&body).await;

        assert_matches!(
            &events[..],
            [
                ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { call_id, name, arguments, .. }),
                ResponseEvent::Completed { .. }
            ] if call_id == "call_a"
                && name == "apply_patch"
                && arguments == "{ \"input\":\"*** Begin Patch\"}"
        );
    }

    #[tokio::test]
    async fn handles_done_sentinel() {
        let events = collect_events("event: message\ndata: [DONE]\n\n").await;
        assert_matches!(
            &events[..],
            [ResponseEvent::Completed {
                response_id,
                token_usage: None,
            }] if response_id.is_empty()
        );
    }
}
