use super::CurrentClientSetup;
use super::ModelClientSession;
use super::RequestRouteTelemetry;
use codex_api::MessagesClient as ApiMessagesClient;
use codex_api::MessagesRequestBuilder as ApiMessagesRequestBuilder;
use codex_api::ReqwestTransport;
use codex_api::TransportError;
use codex_login::CodexAuth;
use codex_login::default_client::build_reqwest_client;
use codex_otel::SessionTelemetry;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::openai_models::ModelInfo;
use reqwest::StatusCode;

use crate::client_common::Prompt;
use crate::client_common::ResponseStream;
use codex_api::map_api_error;

const MESSAGES_ENDPOINT: &str = "/messages";

pub(super) async fn stream_messages(
    session: &mut ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    session_telemetry: &SessionTelemetry,
) -> Result<ResponseStream> {
    if prompt.output_schema.is_some() {
        return Err(CodexErr::UnsupportedOperation(
            "output_schema is not supported for wire_api = \"message\"".to_string(),
        ));
    }

    let mut auth_recovery = session
        .client
        .auth_manager()
        .as_ref()
        .map(codex_login::AuthManager::unauthorized_recovery);
    let mut pending_retry = super::PendingUnauthorizedRetry::default();

    loop {
        let CurrentClientSetup {
            auth,
            api_provider,
            api_auth,
        } = session.client.current_client_setup().await?;
        let auth_context = super::AuthRequestTelemetryContext::new(
            auth.as_ref().map(CodexAuth::auth_mode),
            api_auth.as_ref(),
            pending_retry,
        );
        let transport = ReqwestTransport::new(build_reqwest_client());
        let (request_telemetry, sse_telemetry) = ModelClientSession::build_streaming_telemetry(
            session_telemetry,
            auth_context,
            RequestRouteTelemetry::for_endpoint(MESSAGES_ENDPOINT),
            session.client.state.auth_env_telemetry.clone(),
        );
        let client = ApiMessagesClient::new(transport, api_provider, api_auth)
            .with_telemetry(Some(request_telemetry), Some(sse_telemetry));
        let tools = codex_tools::create_tools_json_for_messages_api(&prompt.tools)?;
        let request = ApiMessagesRequestBuilder::new(
            &model_info.slug,
            &prompt.base_instructions.text,
            &prompt.get_formatted_input(),
            &tools,
        )
        .build();

        let stream_result = client.stream_request(request).await;
        match stream_result {
            Ok(stream) => {
                let (stream, _) = super::map_response_stream(
                    stream,
                    session_telemetry.clone(),
                    codex_rollout_trace::InferenceTraceAttempt::disabled(),
                );
                return Ok(stream);
            }
            Err(codex_api::ApiError::Transport(
                unauthorized_transport @ TransportError::Http { status, .. },
            )) if status == StatusCode::UNAUTHORIZED => {
                pending_retry = super::PendingUnauthorizedRetry::from_recovery(
                    super::handle_unauthorized(
                        unauthorized_transport,
                        &mut auth_recovery,
                        session_telemetry,
                    )
                    .await?,
                );
                continue;
            }
            Err(err) => return Err(map_api_error(err)),
        }
    }
}
