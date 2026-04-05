use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::request_user_input_spec::QUESTION_TOOL_NAME;
use crate::tools::handlers::request_user_input_spec::normalize_request_user_input_args_for_tool;
use crate::tools::handlers::request_user_input_spec::question_unavailable_message;
use crate::tools::handlers::request_user_input_spec::request_user_input_unavailable_message;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::config_types::ModeKind;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_tools::ToolName;

pub struct RequestUserInputHandler {
    pub tool_name: ToolName,
    pub available_modes: Vec<ModeKind>,
}

impl ToolHandler for RequestUserInputHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        self.tool_name.clone()
    }

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

        if turn.session_source.is_non_root_agent() {
            return Err(FunctionCallError::RespondToModel(
                "request_user_input can only be used by the root thread".to_string(),
            ));
        }

        let mode = session.collaboration_mode().await.mode;
        let unavailable_message = match tool_name.name.as_str() {
            QUESTION_TOOL_NAME => question_unavailable_message(mode),
            _ => request_user_input_unavailable_message(mode, &self.available_modes),
        };
        if let Some(message) = unavailable_message {
            return Err(FunctionCallError::RespondToModel(message));
        }

        let args: RequestUserInputArgs = parse_arguments(&arguments)?;
        let args = normalize_request_user_input_args_for_tool(&tool_name.name, args)
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
