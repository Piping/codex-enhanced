use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::rate_limits::parse_all_rate_limits;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

static NEXT_SYNTHETIC_ITEM_ID: AtomicU64 = AtomicU64::new(1);

pub fn spawn_messages_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let rate_limit_snapshots = parse_all_rate_limits(&stream_response.headers);
    let request_id = stream_response
        .headers
        .get("request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        for snapshot in rate_limit_snapshots {
            let _ = tx_event.send(Ok(ResponseEvent::RateLimits(snapshot))).await;
        }
        process_messages_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });
    ResponseStream {
        rx_event,
        upstream_request_id: request_id,
    }
}

pub async fn process_messages_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut content_blocks: HashMap<i64, ContentBlockState> = HashMap::new();
    let mut content_order = Vec::new();
    let mut assistant_item = None;

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
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "stream closed before message_stop".to_string(),
                    )))
                    .await;
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

        trace!("Anthropic Messages SSE event: {}", sse.data);

        let data = sse.data.trim();
        if data.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(err) => {
                debug!("failed to parse messages event: {err}, data: {data}");
                continue;
            }
        };

        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(sse.event.as_str());

        match kind {
            "message_start" => {
                if let Some(model) = value
                    .get("message")
                    .and_then(|message| message.get("model"))
                    .and_then(Value::as_str)
                    && tx_event
                        .send(Ok(ResponseEvent::ServerModel(model.to_string())))
                        .await
                        .is_err()
                {
                    return;
                }
            }
            "content_block_start" => {
                let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                let block = value.get("content_block").unwrap_or(&Value::Null);
                if content_blocks
                    .insert(index, ContentBlockState::new(block))
                    .is_none()
                {
                    content_order.push(index);
                }
            }
            "content_block_delta" => {
                let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                let delta = value.get("delta").unwrap_or(&Value::Null);
                if delta.get("type").and_then(Value::as_str) == Some("text_delta") {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        append_assistant_text(&tx_event, &mut assistant_item, text.to_string())
                            .await;
                    }
                    continue;
                }

                let state = content_blocks.entry(index).or_default();
                if !content_order.contains(&index) {
                    content_order.push(index);
                }
                if let Some(partial_json) = delta.get("partial_json").and_then(Value::as_str) {
                    state.input_json.push_str(partial_json);
                    let call_id = state.id.clone();
                    let _ = tx_event
                        .send(Ok(ResponseEvent::ToolCallInputDelta {
                            item_id: state.id.clone().unwrap_or_else(|| format!("toolu_{index}")),
                            call_id,
                            delta: partial_json.to_string(),
                        }))
                        .await;
                }
            }
            "content_block_stop" => {}
            "message_delta" => {
                if value
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                    == Some("max_tokens")
                {
                    let _ = tx_event.send(Err(ApiError::ContextWindowExceeded)).await;
                    return;
                }
            }
            "message_stop" => {
                if let Some(assistant) = assistant_item.take()
                    && tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(assistant)))
                        .await
                        .is_err()
                {
                    return;
                }
                for index in content_order.drain(..) {
                    let Some(state) = content_blocks.remove(&index) else {
                        continue;
                    };
                    if state.kind != Some("tool_use".to_string()) {
                        continue;
                    }
                    let Some(name) = state.name else {
                        continue;
                    };
                    let item = ResponseItem::FunctionCall {
                        id: None,
                        name,
                        namespace: None,
                        arguments: state.input_json,
                        call_id: state.id.unwrap_or_else(|| format!("toolu_{index}")),
                    };
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(item)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let token_usage = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                    .and_then(token_usage_from_value);
                let _ = tx_event
                    .send(Ok(ResponseEvent::Completed {
                        response_id: String::new(),
                        token_usage,
                        end_turn: None,
                    }))
                    .await;
                return;
            }
            "error" => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Anthropic Messages stream returned an error")
                    .to_string();
                let _ = tx_event.send(Err(ApiError::Stream(message))).await;
                return;
            }
            "ping" => {}
            _ => trace!("unhandled messages event: {kind}"),
        }
    }
}

#[derive(Default)]
struct ContentBlockState {
    kind: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input_json: String,
}

impl ContentBlockState {
    fn new(block: &Value) -> Self {
        let input_json = block
            .get("input")
            .filter(|input| input.as_object().is_none_or(|object| !object.is_empty()))
            .filter(|input| !input.is_null())
            .map(ToString::to_string)
            .unwrap_or_default();
        Self {
            kind: block
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string),
            id: block.get("id").and_then(Value::as_str).map(str::to_string),
            name: block
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            input_json,
        }
    }
}

fn token_usage_from_value(value: &Value) -> Option<TokenUsage> {
    let input_tokens = value.get("input_tokens")?.as_i64()?;
    let output_tokens = value.get("output_tokens")?.as_i64()?;
    let cached_input_tokens = value
        .get("cache_read_input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        + value
            .get("cache_creation_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
    Some(TokenUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens: 0,
        total_tokens: input_tokens + output_tokens,
    })
}

fn synthetic_item_id(kind: &str) -> String {
    let id = NEXT_SYNTHETIC_ITEM_ID.fetch_add(1, Ordering::Relaxed);
    format!("messages-{kind}-{id}")
}

async fn append_assistant_text(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    assistant_item: &mut Option<ResponseItem>,
    text: String,
) {
    if assistant_item.is_none() {
        let item = ResponseItem::Message {
            id: Some(synthetic_item_id("assistant")),
            role: "assistant".to_string(),
            content: Vec::new(),
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
        tokio::spawn(process_messages_sse(
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
    async fn streams_text_and_tool_calls() {
        let body = build_body(&[
            json!({"type":"message_start","message":{"model":"claude-test"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"shell","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":\"pwd\"}"}}),
            json!({"type":"message_stop"}),
        ]);

        let events = collect_events(&body).await;

        assert_matches!(&events[0], ResponseEvent::ServerModel(model) if model == "claude-test");
        assert_matches!(
            &events[1],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { id, .. }) if id.is_some()
        );
        assert_matches!(&events[2], ResponseEvent::OutputTextDelta(delta) if delta == "hi");
        assert_matches!(&events[3], ResponseEvent::ToolCallInputDelta { delta, .. } if delta == "{\"cmd\":\"pwd\"}");
        assert_matches!(
            &events[4],
            ResponseEvent::OutputItemDone(ResponseItem::Message { id, .. }) if id.is_some()
        );
        let ResponseEvent::OutputItemAdded(ResponseItem::Message { id: added_id, .. }) = &events[1]
        else {
            panic!("expected assistant item added");
        };
        let ResponseEvent::OutputItemDone(ResponseItem::Message { id: done_id, .. }) = &events[4]
        else {
            panic!("expected assistant item done");
        };
        assert_eq!(added_id, done_id);
        assert_matches!(
            &events[5],
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { call_id, name, arguments, .. })
                if call_id == "toolu_1" && name == "shell" && arguments == "{\"cmd\":\"pwd\"}"
        );
        assert_matches!(&events[6], ResponseEvent::Completed { .. });
    }

    #[test]
    fn parses_anthropic_usage() {
        let usage = token_usage_from_value(&json!({
            "input_tokens": 10,
            "cache_creation_input_tokens": 2,
            "cache_read_input_tokens": 3,
            "output_tokens": 4
        }))
        .expect("usage should parse");

        assert_eq!(
            usage,
            TokenUsage {
                input_tokens: 10,
                cached_input_tokens: 5,
                output_tokens: 4,
                reasoning_output_tokens: 0,
                total_tokens: 14,
            }
        );
    }
}
