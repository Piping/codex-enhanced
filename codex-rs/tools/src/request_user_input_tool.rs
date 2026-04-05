use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::TUI_VISIBLE_COLLABORATION_MODES;
use codex_protocol::request_user_input::RequestUserInputArgs;
use std::collections::BTreeMap;

pub const QUESTION_TOOL_NAME: &str = "question";
pub const REQUEST_USER_INPUT_TOOL_NAME: &str = "request_user_input";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuestionOptionsPolicy {
    RequireOptions,
    AllowFreeform,
}

pub fn create_question_tool(description: String) -> ToolSpec {
    create_interactive_question_tool(
        QUESTION_TOOL_NAME,
        QuestionToolSchema {
            questions_description: "Questions to show the user. There is no fixed maximum; use as many as needed for the form.",
            prompt_description: "Prompt shown to the user for this field.",
            options_description: "Optional mutually exclusive choices for this question. Omit this field for a freeform text answer. When provided, put the recommended option first and do not include an \"Other\" option; the client can collect additional notes separately.",
            options_required: false,
        },
        description,
    )
}

pub fn create_request_user_input_tool(description: String) -> ToolSpec {
    create_interactive_question_tool(
        REQUEST_USER_INPUT_TOOL_NAME,
        QuestionToolSchema {
            questions_description: "Questions to show the user. Prefer 1 and do not exceed 3",
            prompt_description: "Single-sentence prompt shown to the user.",
            options_description: "Provide 2-3 mutually exclusive choices. Put the recommended option first and suffix its label with \"(Recommended)\". Do not include an \"Other\" option in this list; the client will add a free-form \"Other\" option automatically.",
            options_required: true,
        },
        description,
    )
}

struct QuestionToolSchema {
    questions_description: &'static str,
    prompt_description: &'static str,
    options_description: &'static str,
    options_required: bool,
}

fn create_interactive_question_tool(
    name: &str,
    schema: QuestionToolSchema,
    description: String,
) -> ToolSpec {
    let option_props = BTreeMap::from([
        (
            "label".to_string(),
            JsonSchema::string(Some("User-facing label (1-5 words).".to_string())),
        ),
        (
            "description".to_string(),
            JsonSchema::string(Some(
                "One short sentence explaining impact/tradeoff if selected.".to_string(),
            )),
        ),
    ]);

<<<<<<< HEAD
    let options_schema = JsonSchema::array(
        JsonSchema::object(
            option_props,
            Some(vec!["label".to_string(), "description".to_string()]),
            Some(false.into()),
        ),
        Some(schema.options_description.to_string()),
    );

    let question_props = BTreeMap::from([
        (
            "id".to_string(),
            JsonSchema::string(Some(
                "Stable identifier for mapping answers (snake_case).".to_string(),
            )),
        ),
        (
            "header".to_string(),
            JsonSchema::string(Some(
                "Short header label shown in the UI (12 or fewer chars).".to_string(),
            )),
        ),
        (
            "question".to_string(),
            JsonSchema::string(Some(schema.prompt_description.to_string())),
        ),
        ("options".to_string(), options_schema),
    ]);

    let questions_schema = JsonSchema::array(
        JsonSchema::object(
            question_props,
            Some({
                let mut required = vec![
                    "id".to_string(),
                    "header".to_string(),
                    "question".to_string(),
                ];
                if schema.options_required {
                    required.push("options".to_string());
                }
                required
            }),
            Some(false.into()),
        ),
        Some(schema.questions_description.to_string()),
    );

    let properties = BTreeMap::from([("questions".to_string(), questions_schema)]);

    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description,
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["questions".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

fn question_is_available(mode: ModeKind) -> bool {
    mode.allows_request_user_input() || mode == ModeKind::Default
}

pub fn request_user_input_unavailable_message(
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

pub fn question_unavailable_message(mode: ModeKind) -> Option<String> {
    if question_is_available(mode) {
        None
    } else {
        let mode_name = mode.display_name();
        Some(format!("question is unavailable in {mode_name} mode"))
    }
}

fn tool_is_available(
    tool_name: &str,
    mode: ModeKind,
    default_mode_request_user_input: bool,
) -> bool {
    match tool_name {
        QUESTION_TOOL_NAME => question_is_available(mode),
        _ => request_user_input_is_available(mode, default_mode_request_user_input),
    }
}

fn question_options_policy(tool_name: &str) -> QuestionOptionsPolicy {
    match tool_name {
        QUESTION_TOOL_NAME => QuestionOptionsPolicy::AllowFreeform,
        _ => QuestionOptionsPolicy::RequireOptions,
    }
}

pub fn normalize_request_user_input_args(
    args: RequestUserInputArgs,
) -> Result<RequestUserInputArgs, String> {
    normalize_request_user_input_args_for_tool(REQUEST_USER_INPUT_TOOL_NAME, args)
}

pub fn normalize_request_user_input_args_for_tool(
    tool_name: &str,
    mut args: RequestUserInputArgs,
) -> Result<RequestUserInputArgs, String> {
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

    if question_options_policy(tool_name) == QuestionOptionsPolicy::RequireOptions
        && args
            .questions
            .iter()
            .any(|question| question.options.as_ref().is_none_or(Vec::is_empty))
    {
        return Err("request_user_input requires non-empty options for every question".to_string());
    }

    Ok(args)
}

pub fn request_user_input_tool_description(default_mode_request_user_input: bool) -> String {
    interactive_question_tool_description(
        REQUEST_USER_INPUT_TOOL_NAME,
        "Request user input for one to three short questions and wait for the response.",
        default_mode_request_user_input,
    )
}

pub fn question_tool_description(default_mode_request_user_input: bool) -> String {
    interactive_question_tool_description(
        QUESTION_TOOL_NAME,
        "Ask the user a structured form with as many questions as needed and wait for the response. The client will render choices and/or text fields automatically.",
        default_mode_request_user_input,
    )
}

fn request_user_input_is_available(mode: ModeKind, default_mode_request_user_input: bool) -> bool {
    mode.allows_request_user_input()
        || (default_mode_request_user_input && mode == ModeKind::Default)
}

fn interactive_question_tool_description(
    tool_name: &str,
    tool_description: &str,
    default_mode_request_user_input: bool,
) -> String {
    let allowed_modes = format_allowed_modes(tool_name, default_mode_request_user_input);
    format!("{tool_description} This tool is only available in {allowed_modes}.")
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

#[cfg(test)]
#[path = "request_user_input_tool_tests.rs"]
mod tests;
