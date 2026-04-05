use super::*;
use codex_features::Feature;
use codex_features::Features;
use codex_protocol::config_types::ModeKind;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_tools::JsonSchema;
use codex_tools::request_user_input_available_modes;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

fn default_mode_enabled_available_modes() -> Vec<ModeKind> {
    let mut features = Features::with_defaults();
    features.enable(Feature::DefaultModeRequestUserInput);
    request_user_input_available_modes(&features)
}

fn default_available_modes() -> Vec<ModeKind> {
    request_user_input_available_modes(&Features::with_defaults())
}

#[test]
fn question_tool_includes_questions_schema() {
    assert_eq!(
        create_question_tool("Ask the user for details.".to_string()),
        ToolSpec::Function(ResponsesApiTool {
            name: QUESTION_TOOL_NAME.to_string(),
            description: "Ask the user for details.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::Object {
                properties: BTreeMap::from([(
                    "questions".to_string(),
                    JsonSchema::Array {
                        description: Some(
                            "Questions to show the user. There is no fixed maximum; use as many as needed for the form."
                                .to_string(),
                        ),
                        items: Box::new(JsonSchema::Object {
                            properties: BTreeMap::from([
                                (
                                    "header".to_string(),
                                    JsonSchema::String {
                                        description: Some(
                                            "Short header label shown in the UI (12 or fewer chars)."
                                                .to_string(),
                                        ),
                                    },
                                ),
                                (
                                    "id".to_string(),
                                    JsonSchema::String {
                                        description: Some(
                                            "Stable identifier for mapping answers (snake_case)."
                                                .to_string(),
                                        ),
                                    },
                                ),
                                (
                                    "options".to_string(),
                                    JsonSchema::Array {
                                        description: Some(
                                            "Optional mutually exclusive choices for this question. Omit this field for a freeform text answer. When provided, put the recommended option first and do not include an \"Other\" option; the client can collect additional notes separately."
                                                .to_string(),
                                        ),
                                        items: Box::new(JsonSchema::Object {
                                            properties: BTreeMap::from([
                                                (
                                                    "description".to_string(),
                                                    JsonSchema::String {
                                                        description: Some(
                                                            "One short sentence explaining impact/tradeoff if selected."
                                                                .to_string(),
                                                        ),
                                                    },
                                                ),
                                                (
                                                    "label".to_string(),
                                                    JsonSchema::String {
                                                        description: Some(
                                                            "User-facing label (1-5 words)."
                                                                .to_string(),
                                                        ),
                                                    },
                                                ),
                                            ]),
                                            required: Some(vec![
                                                "label".to_string(),
                                                "description".to_string(),
                                            ]),
                                            additional_properties: Some(false.into()),
                                        }),
                                    },
                                ),
                                (
                                    "question".to_string(),
                                    JsonSchema::String {
                                        description: Some(
                                            "Prompt shown to the user for this field.".to_string(),
                                        ),
                                    },
                                ),
                            ]),
                            required: Some(vec![
                                "id".to_string(),
                                "header".to_string(),
                                "question".to_string(),
                            ]),
                            additional_properties: Some(false.into()),
                        }),
                    },
                )]),
                required: Some(vec!["questions".to_string()]),
                additional_properties: Some(false.into()),
            },
            output_schema: None,
        })
    );
}

#[test]
fn request_user_input_tool_includes_questions_schema() {
    assert_eq!(
        create_request_user_input_tool("Ask the user to choose.".to_string()),
        ToolSpec::Function(ResponsesApiTool {
            name: REQUEST_USER_INPUT_TOOL_NAME.to_string(),
            description: "Ask the user to choose.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(BTreeMap::from([(
                    "questions".to_string(),
                    JsonSchema::array(
                        JsonSchema::object(
                            BTreeMap::from([
                                (
                                    "header".to_string(),
                                    JsonSchema::string(Some(
                                        "Short header label shown in the UI (12 or fewer chars)."
                                            .to_string(),
                                    )),
                                ),
                                (
                                    "id".to_string(),
                                    JsonSchema::string(Some(
                                        "Stable identifier for mapping answers (snake_case)."
                                            .to_string(),
                                    )),
                                ),
                                (
                                    "options".to_string(),
                                    JsonSchema::array(
                                        JsonSchema::object(
                                            BTreeMap::from([
                                                (
                                                    "description".to_string(),
                                                    JsonSchema::string(Some(
                                                        "One short sentence explaining impact/tradeoff if selected."
                                                            .to_string(),
                                                    )),
                                                ),
                                                (
                                                    "label".to_string(),
                                                    JsonSchema::string(Some(
                                                        "User-facing label (1-5 words)."
                                                            .to_string(),
                                                    )),
                                                ),
                                            ]),
                                            Some(vec![
                                                "label".to_string(),
                                                "description".to_string(),
                                            ]),
                                            Some(false.into()),
                                        ),
                                        Some(
                                            "Provide 2-3 mutually exclusive choices. Put the recommended option first and suffix its label with \"(Recommended)\". Do not include an \"Other\" option in this list; the client will add a free-form \"Other\" option automatically."
                                                .to_string(),
                                        ),
                                    ),
                                ),
                                (
                                    "question".to_string(),
                                    JsonSchema::string(Some(
                                        "Single-sentence prompt shown to the user.".to_string(),
                                    )),
                                ),
                            ]),
                            Some(vec![
                                "id".to_string(),
                                "header".to_string(),
                                "question".to_string(),
                                "options".to_string(),
                            ]),
                            Some(false.into()),
                        ),
                        Some(
                            "Questions to show the user. Prefer 1 and do not exceed 3".to_string(),
                        ),
                    ),
                )]), Some(vec!["questions".to_string()]), Some(false.into())),
            output_schema: None,
        })
    );
}

#[test]
fn question_unavailable_messages_respect_mode_rules() {
    assert_eq!(question_unavailable_message(ModeKind::Plan), None);
    assert_eq!(question_unavailable_message(ModeKind::Default), None);
    assert_eq!(
        question_unavailable_message(ModeKind::Execute),
        Some("question is unavailable in Execute mode".to_string())
    );
    assert_eq!(
        question_unavailable_message(ModeKind::PairProgramming),
        Some("question is unavailable in Pair Programming mode".to_string())
    );
}

#[test]
fn request_user_input_unavailable_messages_respect_default_mode_feature_flag() {
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Plan, &default_available_modes()),
        None
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Default, &default_available_modes()),
        Some("request_user_input is unavailable in Default mode".to_string())
    );
    assert_eq!(
        request_user_input_unavailable_message(
            ModeKind::Default,
            &default_mode_enabled_available_modes()
        ),
        None
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Execute, &default_available_modes()),
        Some("request_user_input is unavailable in Execute mode".to_string())
    );
    assert_eq!(
        request_user_input_unavailable_message(
            ModeKind::PairProgramming,
            &default_available_modes()
        ),
        Some("request_user_input is unavailable in Pair Programming mode".to_string())
    );
}

#[test]
fn question_tool_description_mentions_available_modes() {
    assert_eq!(
        question_tool_description(/*default_mode_request_user_input*/ false),
        "Ask the user a structured form with as many questions as needed and wait for the response. The client will render choices and/or text fields automatically. This tool is only available in Default or Plan mode.".to_string()
    );
    assert_eq!(
        question_tool_description(/*default_mode_request_user_input*/ true),
        "Ask the user a structured form with as many questions as needed and wait for the response. The client will render choices and/or text fields automatically. This tool is only available in Default or Plan mode.".to_string()
    );
}

#[test]
fn request_user_input_tool_description_mentions_available_modes() {
    assert_eq!(
        request_user_input_tool_description(&default_available_modes()),
        "Request user input for one to three short questions and wait for the response. This tool is only available in Plan mode.".to_string()
    );
    assert_eq!(
        request_user_input_tool_description(&default_mode_enabled_available_modes()),
        "Request user input for one to three short questions and wait for the response. This tool is only available in Default or Plan mode.".to_string()
    );
}

#[test]
fn normalize_question_args_allows_freeform_questions() {
    let args = RequestUserInputArgs {
        questions: vec![RequestUserInputQuestion {
            id: "details".to_string(),
            header: "Details".to_string(),
            question: "What changed?".to_string(),
            is_other: false,
            is_secret: false,
            options: None,
        }],
    };

    assert_eq!(
        normalize_request_user_input_args_for_tool(QUESTION_TOOL_NAME, args.clone()),
        Ok(args)
    );
}

#[test]
fn normalize_question_args_marks_multiple_choice_entries_as_other() {
    let args = RequestUserInputArgs {
        questions: vec![
            RequestUserInputQuestion {
                id: "details".to_string(),
                header: "Details".to_string(),
                question: "What changed?".to_string(),
                is_other: false,
                is_secret: false,
                options: Some(Vec::new()),
            },
            RequestUserInputQuestion {
                id: "confirm".to_string(),
                header: "Confirm".to_string(),
                question: "Continue?".to_string(),
                is_other: false,
                is_secret: false,
                options: Some(vec![RequestUserInputQuestionOption {
                    label: "Yes".to_string(),
                    description: "Keep going.".to_string(),
                }]),
            },
        ],
    };

    assert_eq!(
        normalize_request_user_input_args_for_tool(QUESTION_TOOL_NAME, args),
        Ok(RequestUserInputArgs {
            questions: vec![
                RequestUserInputQuestion {
                    id: "details".to_string(),
                    header: "Details".to_string(),
                    question: "What changed?".to_string(),
                    is_other: false,
                    is_secret: false,
                    options: None,
                },
                RequestUserInputQuestion {
                    id: "confirm".to_string(),
                    header: "Confirm".to_string(),
                    question: "Continue?".to_string(),
                    is_other: true,
                    is_secret: false,
                    options: Some(vec![RequestUserInputQuestionOption {
                        label: "Yes".to_string(),
                        description: "Keep going.".to_string(),
                    }]),
                },
            ],
        })
    );
}

#[test]
fn normalize_request_user_input_args_requires_options() {
    let args = RequestUserInputArgs {
        questions: vec![RequestUserInputQuestion {
            id: "confirm".to_string(),
            header: "Confirm".to_string(),
            question: "Continue?".to_string(),
            is_other: false,
            is_secret: false,
            options: None,
        }],
    };

    assert_eq!(
        normalize_request_user_input_args(args),
        Err("request_user_input requires non-empty options for every question".to_string())
    );
}
