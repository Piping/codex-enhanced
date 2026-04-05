use super::*;
use crate::JsonSchema;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

#[test]
fn list_dir_tool_matches_expected_spec() {
    assert_eq!(
        create_list_dir_tool(),
        ToolSpec::Function(ResponsesApiTool {
            name: "list_dir".to_string(),
            description:
                "Lists entries in a local directory with 1-indexed entry numbers and simple type labels."
                    .to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(BTreeMap::from([
                    (
                        "depth".to_string(),
                        JsonSchema::number(Some(
                            "The maximum directory depth to traverse. Must be 1 or greater."
                                .to_string(),
                        )),
                    ),
                    (
                        "dir_path".to_string(),
                        JsonSchema::string(Some(
                            "Absolute path to the directory to list.".to_string(),
                        )),
                    ),
                    (
                        "limit".to_string(),
                        JsonSchema::number(Some(
                            "The maximum number of entries to return.".to_string(),
                        )),
                    ),
                    (
                        "offset".to_string(),
                        JsonSchema::number(Some(
                            "The entry number to start listing from. Must be 1 or greater."
                                .to_string(),
                        )),
                    ),
                ]), Some(vec!["dir_path".to_string()]), Some(false.into())),
            output_schema: None,
        })
    );
}

#[test]
fn grep_files_tool_matches_expected_spec() {
    assert_eq!(
        create_grep_files_tool(),
        ToolSpec::Function(ResponsesApiTool {
            name: "grep_files".to_string(),
            description: "Finds files whose contents match the pattern and lists them by modification \
                          time."
                .to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::Object {
                properties: BTreeMap::from([
                    (
                        "include".to_string(),
                        JsonSchema::String {
                            description: Some(
                                "Optional glob that limits which files are searched (e.g. \"*.rs\" or \
                                 \"*.{ts,tsx}\")."
                                    .to_string(),
                            ),
                        },
                    ),
                    (
                        "limit".to_string(),
                        JsonSchema::Number {
                            description: Some(
                                "Maximum number of file paths to return (defaults to 100)."
                                    .to_string(),
                            ),
                        },
                    ),
                    (
                        "path".to_string(),
                        JsonSchema::String {
                            description: Some(
                                "Directory or file path to search. Defaults to the session's working directory."
                                    .to_string(),
                            ),
                        },
                    ),
                    (
                        "pattern".to_string(),
                        JsonSchema::String {
                            description: Some(
                                "Regular expression pattern to search for.".to_string(),
                            ),
                        },
                    ),
                ]),
                required: Some(vec!["pattern".to_string()]),
                additional_properties: Some(false.into()),
            },
            output_schema: None,
        })
    );
}

#[test]
fn read_file_tool_matches_expected_spec() {
    assert_eq!(
        create_read_file_tool(),
        ToolSpec::Function(ResponsesApiTool {
            name: "read_file".to_string(),
            description:
                "Reads a local file with 1-indexed line numbers, supporting slice and indentation-aware block modes."
                    .to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::Object {
                properties: BTreeMap::from([
                    (
                        "file_path".to_string(),
                        JsonSchema::String {
                            description: Some("Absolute path to the file".to_string()),
                        },
                    ),
                    (
                        "indentation".to_string(),
                        JsonSchema::Object {
                            properties: BTreeMap::from([
                                (
                                    "anchor_line".to_string(),
                                    JsonSchema::Number {
                                        description: Some(
                                            "Anchor line to center the indentation lookup on (defaults to offset)."
                                                .to_string(),
                                        ),
                                    },
                                ),
                                (
                                    "include_header".to_string(),
                                    JsonSchema::Boolean {
                                        description: Some(
                                            "Include doc comments or attributes directly above the selected block."
                                                .to_string(),
                                        ),
                                    },
                                ),
                                (
                                    "include_siblings".to_string(),
                                    JsonSchema::Boolean {
                                        description: Some(
                                            "When true, include additional blocks that share the anchor indentation."
                                                .to_string(),
                                        ),
                                    },
                                ),
                                (
                                    "max_levels".to_string(),
                                    JsonSchema::Number {
                                        description: Some(
                                            "How many parent indentation levels (smaller indents) to include."
                                                .to_string(),
                                        ),
                                    },
                                ),
                                (
                                    "max_lines".to_string(),
                                    JsonSchema::Number {
                                        description: Some(
                                            "Hard cap on the number of lines returned when using indentation mode."
                                                .to_string(),
                                        ),
                                    },
                                ),
                            ]),
                            required: None,
                            additional_properties: Some(false.into()),
                        },
                    ),
                    (
                        "limit".to_string(),
                        JsonSchema::Number {
                            description: Some(
                                "The maximum number of lines to return.".to_string(),
                            ),
                        },
                    ),
                    (
                        "mode".to_string(),
                        JsonSchema::String {
                            description: Some(
                                "Optional mode selector: \"slice\" for simple ranges (default) or \"indentation\" \
                                 to expand around an anchor line."
                                    .to_string(),
                            ),
                        },
                    ),
                    (
                        "offset".to_string(),
                        JsonSchema::Number {
                            description: Some(
                                "The line number to start reading from. Must be 1 or greater."
                                    .to_string(),
                            ),
                        },
                    ),
                ]),
                required: Some(vec!["file_path".to_string()]),
                additional_properties: Some(false.into()),
            },
            output_schema: None,
        })
    );
}

#[test]
fn test_sync_tool_matches_expected_spec() {
    assert_eq!(
        create_test_sync_tool(),
        ToolSpec::Function(ResponsesApiTool {
            name: "test_sync_tool".to_string(),
            description: "Internal synchronization helper used by Codex integration tests."
                .to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(BTreeMap::from([
                    (
                        "barrier".to_string(),
                        JsonSchema::object(
                            BTreeMap::from([
                                (
                                    "id".to_string(),
                                    JsonSchema::string(Some(
                                        "Identifier shared by concurrent calls that should rendezvous"
                                            .to_string(),
                                    )),
                                ),
                                (
                                    "participants".to_string(),
                                    JsonSchema::number(Some(
                                        "Number of tool calls that must arrive before the barrier opens"
                                            .to_string(),
                                    )),
                                ),
                                (
                                    "timeout_ms".to_string(),
                                    JsonSchema::number(Some(
                                        "Maximum time in milliseconds to wait at the barrier"
                                            .to_string(),
                                    )),
                                ),
                            ]),
                            Some(vec!["id".to_string(), "participants".to_string()]),
                            Some(false.into()),
                        ),
                    ),
                    (
                        "sleep_after_ms".to_string(),
                        JsonSchema::number(Some(
                            "Optional delay in milliseconds after completing the barrier"
                                .to_string(),
                        )),
                    ),
                    (
                        "sleep_before_ms".to_string(),
                        JsonSchema::number(Some(
                            "Optional delay in milliseconds before any other action".to_string(),
                        )),
                    ),
                ]), /*required*/ None, Some(false.into())),
            output_schema: None,
        })
    );
}
