use super::CurrentClientSetup;
use super::ModelClientSession;
use super::RequestRouteTelemetry;
use codex_api::ChatCompletionsClient as ApiChatCompletionsClient;
use codex_api::ChatCompletionsRequestBuilder as ApiChatCompletionsRequestBuilder;
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

const CHAT_COMPLETIONS_ENDPOINT: &str = "/chat/completions";

pub(super) async fn stream_chat_completions(
    session: &mut ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    session_telemetry: &SessionTelemetry,
) -> Result<ResponseStream> {
    if prompt.output_schema.is_some() {
        return Err(CodexErr::UnsupportedOperation(
            "output_schema is not supported for wire_api = \"chat\"".to_string(),
        ));
    }

    let mut auth_recovery = session
        .client
        .state
        .auth_manager
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
            &api_auth,
            pending_retry,
        );
        let transport = ReqwestTransport::new(build_reqwest_client());
        let (request_telemetry, sse_telemetry) = ModelClientSession::build_streaming_telemetry(
            session_telemetry,
            auth_context,
            RequestRouteTelemetry::for_endpoint(CHAT_COMPLETIONS_ENDPOINT),
            session.client.state.auth_env_telemetry.clone(),
        );
        let client = ApiChatCompletionsClient::new(transport, api_provider, api_auth)
            .with_telemetry(Some(request_telemetry), Some(sse_telemetry));
        let tools = codex_tools::create_tools_json_for_chat_completions_api(&prompt.tools)?;
        let request = ApiChatCompletionsRequestBuilder::new(
            &model_info.slug,
            &prompt.base_instructions.text,
            &prompt.get_formatted_input(),
            &tools,
        )
        .conversation_id(Some(session.client.state.conversation_id.to_string()))
        .session_source(Some(session.client.state.session_source.clone()))
        .build();

        let stream_result = client.stream_request(request).await;
        match stream_result {
            Ok(stream) => {
                let (stream, _) = super::map_response_stream(stream, session_telemetry.clone());
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
