//! Verifies that the agent retries when the SSE stream terminates before
//! delivering a `response.completed` event.

use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use codex_utils_cargo_bin::find_resource;
use core_test_support::load_sse_fixture;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;

fn sse_incomplete() -> String {
    let fixture = find_resource!("tests/fixtures/incomplete_sse.json")
        .unwrap_or_else(|err| panic!("failed to resolve incomplete_sse fixture: {err}"));
    load_sse_fixture(fixture)
}

fn sse_incomplete_with_reason(reason: &str) -> String {
    responses::sse(vec![
        responses::ev_response_created("resp_incomplete"),
        responses::ev_message_item_added("msg_incomplete", "partial content"),
        responses::ev_output_text_delta("continued chunk"),
        serde_json::json!({
            "type": "response.incomplete",
            "response": {
                "id": "resp_incomplete",
                "object": "response",
                "status": "incomplete",
                "error": null,
                "incomplete_details": {
                    "reason": reason,
                }
            }
        }),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_on_early_close() {
    skip_if_no_network!();

    let incomplete_sse = sse_incomplete();
    let completed_sse = responses::sse_completed("resp_ok");

    let (server, _) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: None,
            body: incomplete_sse,
        }],
        vec![StreamingSseChunk {
            gate: None,
            body: completed_sse,
        }],
    ])
    .await;

    // Configure retry behavior explicitly to avoid mutating process-wide
    // environment variables.

    let model_provider = ModelProviderInfo {
        name: "openai".into(),
        base_url: Some(format!("{}/v1", server.uri())),
        // Environment variable that should exist in the test environment.
        // ModelClient will return an error if the environment variable for the
        // provider is not set.
        env_key: Some("PATH".into()),
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        wire_api: WireApi::Responses,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        // exercise retry path: first attempt yields incomplete stream, so allow 1 retry
        request_max_retries: Some(0),
        stream_max_retries: Some(1),
        stream_idle_timeout_ms: Some(2000),
        websocket_connect_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    };

    let TestCodex { codex, .. } = test_codex()
        .with_config(move |config| {
            config.model_provider = model_provider;
        })
        .build_with_streaming_server(&server)
        .await
        .unwrap();

    codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await
        .unwrap();

    // Wait until TurnComplete (should succeed after retry).
    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = server.requests().await;
    assert_eq!(
        requests.len(),
        2,
        "expected retry after incomplete SSE stream"
    );
    let first_request: serde_json::Value =
        serde_json::from_slice(&requests[0]).expect("first request should be valid JSON");
    let second_request: serde_json::Value =
        serde_json::from_slice(&requests[1]).expect("second request should be valid JSON");
    assert_ne!(
        first_request["prompt_cache_key"], second_request["prompt_cache_key"],
        "expected retry after incomplete stream to rotate prompt_cache_key"
    );

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_on_stream_disconnected_before_completion() {
    skip_if_no_network!();

    let incomplete_sse = sse_incomplete_with_reason("content_filter");
    let completed_sse = responses::sse_completed("resp_ok");

    let (server, _) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: None,
            body: incomplete_sse,
        }],
        vec![StreamingSseChunk {
            gate: None,
            body: completed_sse,
        }],
    ])
    .await;

    let model_provider = ModelProviderInfo {
        name: "openai".into(),
        base_url: Some(format!("{}/v1", server.uri())),
        env_key: Some("PATH".into()),
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        wire_api: WireApi::Responses,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: Some(0),
        stream_max_retries: Some(1),
        stream_idle_timeout_ms: Some(2000),
        websocket_connect_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    };

    let TestCodex { codex, .. } = test_codex()
        .with_config(move |config| {
            config.model_provider = model_provider;
        })
        .build_with_streaming_server(&server)
        .await
        .unwrap();

    codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await
        .unwrap();

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = server.requests().await;
    assert_eq!(
        requests.len(),
        2,
        "expected retry after incomplete response stream"
    );
    let first_request: serde_json::Value =
        serde_json::from_slice(&requests[0]).expect("first request should be valid JSON");
    let second_request: serde_json::Value =
        serde_json::from_slice(&requests[1]).expect("second request should be valid JSON");
    assert_ne!(
        first_request["prompt_cache_key"], second_request["prompt_cache_key"],
        "expected stream disconnected before completion retry to rotate prompt_cache_key"
    );

    server.shutdown().await;
}
