use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::protocol::SessionSource;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_tools::QUESTION_TOOL_NAME;
use codex_tools::normalize_request_user_input_args_for_tool;
use codex_tools::question_unavailable_message;
use codex_tools::request_user_input_unavailable_message;

pub struct RequestUserInputHandler {
    pub default_mode_request_user_input: bool,
}

impl ToolHandler for RequestUserInputHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "{tool_name} handler received unsupported payload"
                )));
            }
        };

        if matches!(turn.session_source, SessionSource::SubAgent(_)) {
            return Err(FunctionCallError::RespondToModel(
                "request_user_input can only be used by the root thread".to_string(),
            ));
        }

        let mode = session.collaboration_mode().await.mode;
        let unavailable_message = match tool_name.as_str() {
            QUESTION_TOOL_NAME => question_unavailable_message(mode),
            _ => request_user_input_unavailable_message(mode, self.default_mode_request_user_input),
        };
        if let Some(message) = unavailable_message {
            return Err(FunctionCallError::RespondToModel(message));
        }

        let args: RequestUserInputArgs = parse_arguments(&arguments)?;
        let args = normalize_request_user_input_args_for_tool(&tool_name, args)
            .map_err(FunctionCallError::RespondToModel)?;
        let response = session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(format!(
                    "{tool_name} was cancelled before receiving a response"
                ))
            })?;

        let content = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize {tool_name} response: {err}"))
        })?;

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

#[cfg(test)]
#[path = "request_user_input_tests.rs"]
mod tests;
