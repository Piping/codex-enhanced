use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use async_trait::async_trait;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::TUI_VISIBLE_COLLABORATION_MODES;
use codex_protocol::request_user_input::RequestUserInputArgs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuestionOptionsPolicy {
    RequireOptions,
    AllowFreeform,
}

fn question_is_available(mode: ModeKind) -> bool {
    mode.allows_request_user_input() || mode == ModeKind::Default
}

fn request_user_input_is_available(mode: ModeKind, default_mode_request_user_input: bool) -> bool {
    mode.allows_request_user_input()
        || (default_mode_request_user_input && mode == ModeKind::Default)
}

fn tool_is_available(
    tool_name: &str,
    mode: ModeKind,
    default_mode_request_user_input: bool,
) -> bool {
    match tool_name {
        "question" => question_is_available(mode),
        _ => request_user_input_is_available(mode, default_mode_request_user_input),
    }
}

fn format_allowed_modes(tool_name: &str, default_mode_request_user_input: bool) -> String {
    let mode_names: Vec<&str> = TUI_VISIBLE_COLLABORATION_MODES
        .into_iter()
        .filter(|mode| tool_is_available(tool_name, *mode, default_mode_request_user_input))
        .map(ModeKind::display_name)
        .collect();

    match mode_names.as_slice() {
        [] => "no modes".to_string(),
        [mode] => format!("{mode} mode"),
        [first, second] => format!("{first} or {second} mode"),
        [..] => format!("modes: {}", mode_names.join(",")),
    }
}

fn interactive_question_tool_description(
    tool_name: &str,
    tool_description: &str,
    default_mode_request_user_input: bool,
) -> String {
    let allowed_modes = format_allowed_modes(tool_name, default_mode_request_user_input);
    format!("{tool_description} This tool is only available in {allowed_modes}.")
}

pub(crate) fn request_user_input_unavailable_message(
    mode: ModeKind,
    default_mode_request_user_input: bool,
) -> Option<String> {
    if request_user_input_is_available(mode, default_mode_request_user_input) {
        None
    } else {
        let mode_name = mode.display_name();
        Some(format!(
            "request_user_input is unavailable in {mode_name} mode"
        ))
    }
}

pub(crate) fn question_unavailable_message(mode: ModeKind) -> Option<String> {
    if question_is_available(mode) {
        None
    } else {
        let mode_name = mode.display_name();
        Some(format!("question is unavailable in {mode_name} mode"))
    }
}

pub(crate) fn request_user_input_tool_description(default_mode_request_user_input: bool) -> String {
    interactive_question_tool_description(
        "request_user_input",
        "Request user input for one to three short questions and wait for the response.",
        default_mode_request_user_input,
    )
}

pub(crate) fn question_tool_description(default_mode_request_user_input: bool) -> String {
    interactive_question_tool_description(
        "question",
        "Ask the user a structured form with as many questions as needed and wait for the response. The client will render choices and/or text fields automatically.",
        default_mode_request_user_input,
    )
}

fn question_options_policy(tool_name: &str) -> QuestionOptionsPolicy {
    match tool_name {
        "question" => QuestionOptionsPolicy::AllowFreeform,
        _ => QuestionOptionsPolicy::RequireOptions,
    }
}

fn normalize_question_args(
    args: &mut RequestUserInputArgs,
    options_policy: QuestionOptionsPolicy,
) -> Result<(), FunctionCallError> {
    for question in &mut args.questions {
        if question.options.as_ref().is_some_and(Vec::is_empty) {
            question.options = None;
        }
        if question
            .options
            .as_ref()
            .is_some_and(|options| !options.is_empty())
        {
            question.is_other = true;
        }
    }

    if options_policy == QuestionOptionsPolicy::RequireOptions
        && args
            .questions
            .iter()
            .any(|question| question.options.as_ref().is_none_or(Vec::is_empty))
    {
        return Err(FunctionCallError::RespondToModel(
            "request_user_input requires non-empty options for every question".to_string(),
        ));
    }

    Ok(())
}

pub struct RequestUserInputHandler {
    pub default_mode_request_user_input: bool,
}

#[async_trait]
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

        let mode = session.collaboration_mode().await.mode;
        let unavailable_message = match tool_name.as_str() {
            "question" => question_unavailable_message(mode),
            _ => request_user_input_unavailable_message(mode, self.default_mode_request_user_input),
        };
        if let Some(message) = unavailable_message {
            return Err(FunctionCallError::RespondToModel(message));
        }

        let mut args: RequestUserInputArgs = parse_arguments(&arguments)?;
        normalize_question_args(&mut args, question_options_policy(&tool_name))?;
        let response = session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "request_user_input was cancelled before receiving a response".to_string(),
                )
            })?;

        let content = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize request_user_input response: {err}"
            ))
        })?;

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

#[cfg(test)]
#[path = "request_user_input_tests.rs"]
mod tests;
